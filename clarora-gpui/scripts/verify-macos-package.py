#!/usr/bin/env python3
"""Verify a packaged app without Homebrew PATH, DYLD, Python, or user data.

Static closure/hash/signature checks run before real FFmpeg encoding, ffprobe,
yt-dlp, helper-loader checks and the application's --runtime-check entry point.
Evidence is written outside the signed app. --launch-seconds additionally starts
an isolated GUI process; it never attaches to or terminates an existing app.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import tempfile
import time

SYSTEM = ("/usr/lib/", "/System/Library/")
MAGICS = {b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"}


def sha(file):
    digest = hashlib.sha256()
    with file.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def macho(file):
    with file.open("rb") as stream:
        return stream.read(4) in MAGICS


def execute(command, env=None, timeout=30, expected=0):
    result = subprocess.run([str(x) for x in command], env=env, stdin=subprocess.DEVNULL,
                            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    if result.returncode != expected:
        raise RuntimeError(f"{command[0]} returned {result.returncode}, expected {expected}: {result.stderr[-4000:]}")
    return {"command": [str(x) for x in command], "returncode": result.returncode,
            "stdout": result.stdout[-4000:], "stderr": result.stderr[-4000:]}


def verify(args, report):
    app = args.app.resolve(strict=True)
    macos = app / "Contents/MacOS"
    evidence = args.evidence.resolve()
    if evidence == app or evidence.is_relative_to(app):
        raise RuntimeError("Evidence must be outside signed app")
    evidence.mkdir(parents=True, exist_ok=True)
    manifest_file = app / "Contents/Resources/package-manifest.json"
    manifest = json.loads(manifest_file.read_text())
    if manifest.get("schema_version") != 1 or manifest.get("platform") != "macOS":
        raise RuntimeError("Unsupported package manifest")
    with (app / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    if info["CFBundleExecutable"] != "clarora-gpui":
        raise RuntimeError("Unexpected app entry point")
    files = {}
    for entry in manifest["files"]:
        relative = Path(entry["path"])
        if relative.is_absolute() or ".." in relative.parts or entry["path"] in files:
            raise RuntimeError(f"Invalid manifest path: {relative}")
        file = app / relative
        if not file.is_file() or file.is_symlink():
            raise RuntimeError(f"Manifest payload missing or linked: {relative}")
        if entry.get("verification") == "codesign":
            if entry["path"] != "Contents/MacOS/clarora-gpui" or entry.get("sha256") is not None:
                raise RuntimeError(f"Unexpected codesign-only manifest entry: {relative}")
        elif file.stat().st_size != entry["size"] or sha(file) != entry["sha256"]:
            raise RuntimeError(f"Manifest payload mismatch: {relative}")
        files[entry["path"]] = entry
    images = []
    forbidden = {".sqlite", ".sqlite3", ".db", ".onnx", ".gguf", ".safetensors", ".pkl"}
    for file in sorted(app.rglob("*")):
        if file.is_symlink():
            raise RuntimeError(f"Unexpected payload symlink: {file}")
        if not file.is_file():
            continue
        relative = str(file.relative_to(app))
        if "_CodeSignature" in file.parts or file == manifest_file:
            continue
        if relative not in files:
            raise RuntimeError(f"Unlisted payload file: {relative}")
        if file.suffix.lower() in forbidden or re.match(r"ggml-.*\.bin$", file.name) or file.name in {".env", "credentials.json", "pyvenv.cfg"}:
            raise RuntimeError(f"User data/model/config forbidden in app: {relative}")
        if macho(file):
            command = None
            dependencies = []
            complete = subprocess.check_output(["/usr/bin/otool", "-l", str(file)], text=True)
            for line in complete.splitlines():
                line = line.strip()
                if line.startswith("cmd "):
                    command = line[4:]
                elif command == "LC_RPATH" and line.startswith("path "):
                    raise RuntimeError(f"Unexpected LC_RPATH in {relative}: {line}")
                elif command in {"LC_LOAD_DYLIB", "LC_LOAD_WEAK_DYLIB", "LC_REEXPORT_DYLIB", "LC_LOAD_UPWARD_DYLIB"} and line.startswith("name "):
                    name = line[5:].rsplit(" (offset", 1)[0]
                    if name.startswith(SYSTEM):
                        dependencies.append(name)
                    elif name.startswith("@loader_path/"):
                        target = (file.parent / name[len("@loader_path/"):]).resolve(strict=True)
                        if not target.is_relative_to(app) or not target.is_file():
                            raise RuntimeError(f"External loader dependency {name} in {relative}")
                        dependencies.append(name)
                    else:
                        raise RuntimeError(f"Unrelocated dependency {name} in {relative}")
            archs = subprocess.check_output(["/usr/bin/lipo", "-archs", str(file)], text=True).split()
            if manifest["architecture"] not in archs:
                raise RuntimeError(f"Wrong architecture in {relative}: {archs}")
            execute(["/usr/bin/codesign", "--verify", "--strict", file])
            images.append({"path": relative, "dependencies": dependencies, "architectures": archs})
    execute(["/usr/bin/codesign", "--verify", "--deep", "--strict", app])
    for component in manifest["components"]:
        if not component.get("source") or not component.get("license_files"):
            raise RuntimeError(f"Missing provenance/notice for {component.get('name')}")
        for notice in component["license_files"]:
            if notice not in files:
                raise RuntimeError(f"Missing component notice {notice}")
    rust_inventory = json.loads((app / "Contents/Resources/licenses/rust/inventory.json").read_text())
    rust = rust_inventory["crates"]
    if len(rust) != manifest["rust_crate_count"]:
        raise RuntimeError("Rust license inventory count mismatch")
    report["static"] = {"passed": True, "files_verified": len(files), "native_images": images,
                        "native_components": len(manifest["components"]), "rust_crates": len(rust),
                        "crates_without_upstream_license_text": [f"{x['name']} {x['version']}" for x in rust if not x["license_files"]]}
    # A tiny environment prevents accidental reliance on shell dotfiles, Homebrew PATH,
    # DYLD search paths, Python site packages, user configuration or existing app data.
    home = evidence / "isolated-home"
    home.mkdir(exist_ok=True)
    env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home), "TMPDIR": str(evidence),
           "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
    tools = macos / "tools"
    commands = []
    commands.append(execute([tools / "ffmpeg", "-version"], env))
    fixture = evidence / "relocated-media.mp4"
    commands.append(execute([tools / "ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "color=c=teal:s=160x90:d=0.25", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.25", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", fixture], env))
    probe = execute([tools / "ffprobe", "-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json", fixture], env)
    parsed = json.loads(probe["stdout"])
    if {x["codec_type"] for x in parsed["streams"]} != {"audio", "video"}:
        raise RuntimeError("FFmpeg fixture missing audio/video")
    commands.append(probe)
    commands.append(execute([tools / "yt-dlp", "--ignore-config", "--version"], env))
    python_home = app / "Contents/Resources/python"
    if python_home.is_dir():
        interpreters = sorted((python_home / "bin").glob("python*"))
        if len(interpreters) != 1:
            raise RuntimeError("Packaged downloader interpreter is ambiguous")
        # HTTP extraction alone does not prove TLS trust works on a clean host.
        # Build the downloader's real SSL context using its bundled certifi roots.
        code = ("import sys,json;from pathlib import Path;"
                "sys.path.insert(0,str(Path(sys.argv[1])/'site-packages'));"
                "import certifi;from yt_dlp.networking._helper import make_ssl_context;"
                "p=Path(certifi.where()).resolve();assert p.is_relative_to(Path(sys.argv[1]));"
                "s=make_ssl_context().cert_store_stats();assert s['x509_ca']>0;"
                "print(json.dumps({'ca_bundle':str(p),'certificate_store':s}))")
        commands.append(execute([interpreters[0], "-S", "-P", "-c", code, python_home],
                                {**env, "PYTHONHOME": str(python_home)}))
    # The downloader exercises a local HTTP extraction path, including its real TLS/network
    # imports and URL extractor modules, without depending on an external video account.
    import http.server
    import threading
    handler = lambda *a, **k: http.server.SimpleHTTPRequestHandler(*a, directory=str(evidence), **k)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        commands.append(execute([tools / "yt-dlp", "--ignore-config", "--no-playlist", "--skip-download", "--dump-single-json", f"http://127.0.0.1:{server.server_port}/{fixture.name}"], env))
    finally:
        server.shutdown()
        server.server_close()
    commands.append(execute([tools / "clarora-asr"], env, expected=2))
    commands.append(execute([tools / "clarora-tts"], env, expected=2))
    runtime_report = evidence / "runtime-check.json"
    commands.append(execute([macos / "clarora-gpui", "--runtime-check", runtime_report], env, timeout=60))
    native = json.loads(runtime_report.read_text())
    if not native.get("passed") or not native.get("packaged"):
        raise RuntimeError("Application runtime check did not pass in strict packaged mode")
    for component in native.get("components", []):
        if component.get("path") and not Path(component["path"]).resolve(strict=True).is_relative_to(app):
            raise RuntimeError(f"Application borrowed an external runtime: {component['path']}")
    report["runtime"] = {"passed": True, "environment": env, "commands": commands,
                         "runtime_check": native, "fixture_sha256": sha(fixture)}
    if args.launch_seconds:
        with (evidence / "gui-startup.log").open("w") as log:
            child = subprocess.Popen([str(macos / "clarora-gpui"), "--data-dir", str(evidence / "gui-data")], env=env,
                                     stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            try:
                try:
                    code = child.wait(timeout=args.launch_seconds)
                    raise RuntimeError(f"GUI exited during startup: {code}; see gui-startup.log")
                except subprocess.TimeoutExpired:
                    report["gui_startup"] = {"passed": True, "pid": child.pid, "seconds_alive": args.launch_seconds, "scope": "startup only; no GUI interaction asserted"}
            finally:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
    report["passed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--launch-seconds", type=float, default=0)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    report = {"schema_version": 1, "passed": False, "app": str(args.app.resolve()), "checked_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    try:
        verify(args, report)
    except Exception as error:
        report["error"] = str(error)
    (args.evidence / "package-verification.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k not in {"static", "runtime"}}, indent=2))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
