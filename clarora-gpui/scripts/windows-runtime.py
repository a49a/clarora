#!/usr/bin/env python3
"""Pinned Windows runtime fetch/stage and deterministic package inventory helpers."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("windows_verify", SCRIPTS / "verify-windows-package.py")
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def copy(source, destination):
    verify.require(source.is_file() and source.stat().st_size, f"Missing runtime input: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_file():
        verify.require(verify.sha256(source) == verify.sha256(destination), f"Conflicting runtime DLL/file: {destination}")
    else:
        shutil.copy2(source, destination)


def one(root, name):
    matches = list(root.rglob(name))
    verify.require(len(matches) == 1, f"Expected one {name} under {root}, found {len(matches)}")
    return matches[0]


def extract(archive, root, kind):
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    if kind == "zip":
        with zipfile.ZipFile(archive) as handle:
            for member in handle.infolist():
                verify.relative_path(member.filename.rstrip("/"))
                verify.require((member.external_attr >> 16) & 0o170000 != 0o120000, "Dependency ZIP symlink is not allowed")
            handle.extractall(root)
    elif kind == "tar":
        with tarfile.open(archive) as handle:
            for member in handle.getmembers():
                target = root / verify.relative_path(member.name.rstrip("/"))
                verify.require(member.isdir() or member.isfile(), f"Dependency tar link/device is not allowed: {member.name}")
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with handle.extractfile(member) as source, target.open("wb") as output:
                        shutil.copyfileobj(source, output)
    elif kind == "7z":
        # Windows runners include 7-Zip. Fixed archive hash is verified first.
        command = shutil.which("7z") or shutil.which("7z.exe")
        if not command:
            default = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "7-Zip/7z.exe"
            command = str(default) if default.is_file() else None
        verify.require(command, "7-Zip is required for the checksum-pinned mpv archive")
        subprocess.run([command, "x", "-y", str(archive), "-o" + str(root)], check=True, stdout=subprocess.DEVNULL)
    else:
        raise ValueError(f"Unknown archive type: {kind}")
    for path in root.rglob("*"):
        verify.require(not path.is_symlink(), f"Dependency symlink is not allowed: {path}")


def fetch(work):
    lock = json.loads((SCRIPTS / "windows-runtime-lock.json").read_text())
    verify.require(lock["schema_version"] == 1, "Unsupported dependency lock")
    sources = {}
    for item in lock["artifacts"]:
        digest = item["sha256"]
        verify.require(len(digest) == 64 and all(c in "0123456789abcdef" for c in digest), "Invalid pinned SHA-256")
        archive = work / "downloads" / (item["name"] + ".download")
        archive.parent.mkdir(parents=True, exist_ok=True)
        if not archive.is_file() or verify.sha256(archive) != digest:
            partial = archive.with_suffix(".partial")
            print(f"Downloading pinned {item['name']} {item['version']}", flush=True)
            request = urllib.request.Request(item["url"], headers={"User-Agent": "Clarora-GPUI-runtime-packager"})
            with urllib.request.urlopen(request, timeout=90) as response, partial.open("wb") as output:
                shutil.copyfileobj(response, output)
            verify.require(verify.sha256(partial) == digest, f"SHA-256 mismatch: {item['name']}")
            partial.replace(archive)
        root = work / "sources" / item["name"]
        if item["kind"] == "file":
            root.mkdir(parents=True, exist_ok=True)
            shutil.copy2(archive, root / item["filename"])
        else:
            extract(archive, root, item["kind"])
        sources[item["name"]] = str(root.resolve())
    sherpa_lib = one(Path(sources["sherpa-onnx"]), "sherpa-onnx-c-api.lib")
    sherpa_include = work / "sherpa-include"
    copy(Path(sources["sherpa-header"]) / "c-api.h", sherpa_include / "sherpa-onnx/c-api/c-api.h")
    whisper_roots = list(Path(sources["whisper.cpp"]).iterdir())
    verify.require(len(whisper_roots) == 1, "Unexpected Whisper source archive root")
    inputs = {"sources": sources, "lock": lock,
              "whisper_source": str(whisper_roots[0]),
              "sherpa_include": str(sherpa_include.resolve()), "sherpa_library": str(sherpa_lib.resolve())}
    verify.require((Path(inputs["whisper_source"]) / "CMakeLists.txt").is_file(), "Whisper source layout changed")
    write_json(work / "build-inputs.json", inputs)


def license_files(source, destination):
    collected = []
    for path in sorted(source.rglob("*")):
        name = path.name.lower()
        if path.is_file() and (name.startswith(("license", "copying", "copyright", "thirdpartynotices", "third_party_license")) or name == "redist.txt"):
            target = destination / path.relative_to(source)
            copy(path, target)
            collected.append(target)
    return collected


def stage(work, output, msvc_redist, msvc_licenses):
    inputs = json.loads((work / "build-inputs.json").read_text())
    sources = {name: Path(path) for name, path in inputs["sources"].items()}
    lock = {item["name"]: item for item in inputs["lock"]["artifacts"]}
    verify.require(msvc_redist.is_dir() and "debug" not in str(msvc_redist).lower(), "MSVC release redistributable directory is required")
    verify.require(msvc_licenses.is_dir(), "Visual Studio license directory is required")
    verify.require(output != work and not work.is_relative_to(output) and not output.is_relative_to(work / "sources"), "Runtime output must be separate from dependency sources")
    if output.exists():
        verify.require(not any(output.iterdir()) or (output / "runtime-manifest.json").is_file()
                       or (output / ".clarora-runtime-stage").is_file(),
                       "Refusing to replace a nonempty directory without a Clarora runtime manifest")
        shutil.rmtree(output)
    output.mkdir(parents=True)
    (output / ".clarora-runtime-stage").write_text("Clarora Windows runtime staging directory\n")
    components = []

    def component(name, paths, licenses, source_name=None, version=None, source=None):
        for path, destination in paths:
            copy(path, output / destination)
        verify.require(licenses, f"No third-party license inventory: {name}")
        origin = lock.get(source_name or name, {})
        components.append({"name": name, "version": version or origin.get("version"),
                           "source": source or origin.get("url"), "source_sha256": origin.get("sha256"),
                           "binary_files": [destination for _, destination in paths],
                           "license_files": sorted(path.relative_to(output).as_posix() for path in licenses)})

    mpv = list(sources["mpv"].rglob("*mpv-2.dll"))
    verify.require(len(mpv) == 1, "Pinned mpv dev archive does not contain one libmpv DLL")
    mpv_dlls = sorted(sources["mpv"].rglob("*.dll"))
    component("mpv", [(p, "mpv-2.dll" if p == mpv[0] else p.name) for p in mpv_dlls],
              license_files(sources["mpv-source"], output / "licenses/mpv") + license_files(sources["mpv"], output / "licenses/mpv-runtime"))
    vulkan = sources["vulkan-loader"] / f"VulkanRT-X64-{lock['vulkan-loader']['version']}-Components"
    vulkan_notice = output / "licenses/vulkan-loader/VulkanRT-License.txt"
    vulkan_license = output / "licenses/vulkan-loader/LICENSE.txt"
    copy(vulkan / "VulkanRT-License.txt", vulkan_notice)
    copy(sources["vulkan-loader-license"] / "LICENSE.txt", vulkan_license)
    component("vulkan-loader", [(vulkan / "x64/vulkan-1.dll", "vulkan-1.dll")],
              [vulkan_notice, vulkan_license])
    component("ffmpeg", [(one(sources["ffmpeg"], name), "tools/" + name) for name in ("ffmpeg.exe", "ffprobe.exe")],
              license_files(sources["ffmpeg"], output / "licenses/ffmpeg"))
    yt_licenses = license_files(sources["yt-dlp-license"], output / "licenses/yt-dlp") + license_files(sources["yt-dlp-notices"], output / "licenses/yt-dlp")
    component("yt-dlp", [(one(sources["yt-dlp"], "yt-dlp.exe"), "tools/yt-dlp.exe")], yt_licenses)
    component("pdfium", [(one(sources["pdfium"], "pdfium.dll"), "pdfium.dll")],
              license_files(sources["pdfium"], output / "licenses/pdfium"))
    whisper_dlls = sorted((work / "whisper-install").rglob("*.dll"))
    verify.require(any(p.name == "ggml-cpu.dll" for p in whisper_dlls), "Whisper installation lacks the dynamic GGML CPU backend")
    component("whisper.cpp", [(p, "tools/" + p.name) for p in whisper_dlls],
              license_files(sources["whisper.cpp"], output / "licenses/whisper.cpp"))
    sherpa_dlls = sorted(sources["sherpa-onnx"].rglob("*.dll"))
    component("sherpa-onnx", [(p, "tools/" + p.name) for p in sherpa_dlls],
              license_files(sources["sherpa-license"], output / "licenses/sherpa-onnx")
              + license_files(sources["onnxruntime-license"], output / "licenses/onnxruntime")
              + license_files(sources["onnxruntime-notices"], output / "licenses/onnxruntime"))
    crt = sorted(msvc_redist.glob("*.dll"))
    for directory in msvc_redist.parent.glob("Microsoft.VC*.OpenMP"):
        crt.extend(sorted(directory.glob("*.dll")))
    verify.require(any(p.name.lower() == "vcruntime140.dll" for p in crt), "MSVC release CRT DLLs missing")
    crt_license = license_files(msvc_licenses, output / "licenses/msvc-runtime")
    component("msvc-runtime", [(p, prefix + p.name) for p in crt for prefix in ("", "tools/")], crt_license,
              version=msvc_redist.parent.parent.name,
              source="Microsoft Visual Studio 2022 VC/Redist/MSVC/" + msvc_redist.parent.parent.name + "/x64/Microsoft.VC143.CRT")
    native = [(one(work / "native-build", name), "tools/" + name) for name in ("clarora-asr.exe", "clarora-tts.exe")]
    native_license = output / "licenses/clarora-native/LICENSE"
    copy(SCRIPTS.parent.parent / "LICENSE", native_license)
    miniaudio = output / "licenses/clarora-native/miniaudio.h"
    copy(SCRIPTS.parent.parent / "clarora-app/windows/clarora-asr/miniaudio.h", miniaudio)
    revision = subprocess.check_output(["git", "-C", str(SCRIPTS.parent.parent), "rev-parse", "HEAD"], text=True).strip()
    component("clarora-native", native, [native_license, miniaudio], version=revision,
              source="https://github.com/a49a/clarora/tree/" + revision)
    rust_licenses = output / "licenses/rust"
    subprocess.run([sys.executable, str(SCRIPTS / "packaging_licenses.py"),
                    "--manifest-path", str(SCRIPTS.parent / "Cargo.toml"),
                    "--target", "x86_64-pc-windows-msvc", "--output", str(rust_licenses)], check=True)
    rust_inventory = json.loads((rust_licenses / "inventory.json").read_text(encoding="utf-8"))
    component("rust-crates", [], [rust_licenses / "inventory.json", rust_licenses / "README.txt"],
              version=verify.sha256(rust_licenses / "Cargo.lock"),
              source="https://github.com/a49a/clarora/blob/" + revision + "/clarora-gpui/Cargo.lock")
    write_json(output / "runtime-manifest.json", {"schema_version": 1, "platform": "windows", "architecture": "x86_64",
               "components": components, "license_audit_required": rust_inventory["audit_required"], "legal_audit_passed": False,
               "files": verify.inventory(output, {"runtime-manifest.json"})})
    write_json(work / "runtime-build-provenance.json", {"source_revision": revision, "lock": inputs["lock"],
               "msvc_redist": str(msvc_redist), "msvc_licenses": str(msvc_licenses), "runtime": str(output)})


def manifest(root, version, signing):
    path = root / "runtime-manifest.json"
    runtime = json.loads(path.read_text(encoding="utf-8-sig"))
    for item in runtime["files"]:
        binary = root / verify.relative_path(item["path"])
        item.update(size=binary.stat().st_size, sha256=verify.sha256(binary))
    write_json(path, runtime)
    write_json(root / "package-manifest.json", {"schema_version": 1, "platform": "windows", "architecture": "x86_64",
               "version": version, "signing": signing, "components": runtime["components"],
               "license_audit_required": runtime.get("license_audit_required", True), "legal_audit_passed": False,
               "files": verify.inventory(root, {"package-manifest.json"})})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    cmd = commands.add_parser("fetch")
    cmd.add_argument("work", type=Path)
    cmd = commands.add_parser("stage")
    cmd.add_argument("work", type=Path)
    cmd.add_argument("output", type=Path)
    cmd.add_argument("--msvc-redist", required=True, type=Path)
    cmd.add_argument("--msvc-licenses", required=True, type=Path)
    cmd = commands.add_parser("manifest")
    cmd.add_argument("root", type=Path)
    cmd.add_argument("--version", required=True)
    cmd.add_argument("--signing", choices=("unsigned", "authenticode-verified"), required=True)
    cmd = commands.add_parser("zip")
    cmd.add_argument("root", type=Path)
    cmd.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "fetch":
            fetch(args.work.resolve())
        elif args.command == "stage":
            stage(args.work.resolve(), args.output.resolve(), args.msvc_redist.resolve(), args.msvc_licenses.resolve())
        elif args.command == "manifest":
            manifest(args.root.resolve(), args.version, args.signing)
        elif args.command == "zip":
            with zipfile.ZipFile(args.output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as archive:
                for path in sorted(args.root.rglob("*")):
                    if path.is_file():
                        archive.write(path, path.relative_to(args.root).as_posix())
    except Exception as error:
        print(f"Windows runtime packaging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
