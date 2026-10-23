#!/usr/bin/env python3
"""Assemble an offline, relocatable macOS app from explicitly supplied local runtimes.

No binary is downloaded. Default builds --release --locked --offline. See
macos-runtime.example.json for runtime provenance and license inputs. Signing is
ad-hoc unless --sign-identity is supplied; notarization is explicitly opt-in.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SYSTEM = ("/usr/lib/", "/System/Library/")
MAGICS = {b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"}
NOTICE = re.compile(r"^(licen[cs]e|copying|copyright|notice|authors|[al]?gpl|mit|bsd|isc|apache|unlicense)([._-].*)?$", re.I)


def run(args, **kwargs):
    result = subprocess.run([str(x) for x in args], check=True, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
    return result.stdout


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def path(value):
    return Path(os.path.expandvars(value)).expanduser().resolve(strict=True)


def macho(file):
    with file.open("rb") as stream:
        return stream.read(4) in MAGICS


def loads(file):
    text = run(["/usr/bin/otool", "-l", file])
    dependencies, rpaths, identity = [], [], None
    command = None
    minimum = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("cmd "):
            command = line[4:]
        elif command in ("LC_LOAD_DYLIB", "LC_LOAD_WEAK_DYLIB", "LC_REEXPORT_DYLIB", "LC_LOAD_UPWARD_DYLIB") and line.startswith("name "):
            dependencies.append(line[5:].rsplit(" (offset", 1)[0])
        elif command == "LC_ID_DYLIB" and line.startswith("name "):
            identity = line[5:].rsplit(" (offset", 1)[0]
        elif command == "LC_RPATH" and line.startswith("path "):
            rpaths.append(line[5:].rsplit(" (offset", 1)[0])
        elif command == "LC_BUILD_VERSION" and line.startswith("minos "):
            minimum.append(line.split()[1])
        elif command == "LC_VERSION_MIN_MACOSX" and line.startswith("version "):
            minimum.append(line.split()[1])
    return dependencies, rpaths, identity, minimum


class Assembler:
    def __init__(self, args, stage, config):
        self.args, self.app, self.config = args, stage, config
        self.macos = stage / "Contents/MacOS"
        self.resources = stage / "Contents/Resources"
        self.frameworks = stage / "Contents/Frameworks"
        self.licenses = self.resources / "licenses"
        for directory in (self.macos / "tools", self.frameworks, self.licenses):
            directory.mkdir(parents=True, exist_ok=True)
        self.records, self.component_records, self.destinations, self.processing = {}, {}, {}, set()
        self.minimum = ["12.0"]
        self.search = [path(x) for x in config.get("library_search_paths", [])]
        self.arch = platform.machine()

    def provenance(self, source):
        for provider in self.config.get("providers", []):
            if any(source == path(p) or source.is_relative_to(path(p)) for p in provider["roots"]):
                return provider
        parts = source.parts
        if "Cellar" in parts:
            index = parts.index("Cellar")
            cellar = Path(*parts[:index + 3])
            name, version = parts[index + 1:index + 3]
            licenses = [str(p) for p in cellar.iterdir() if p.is_file() and NOTICE.match(p.name)]
            if not licenses:
                licenses = [str(p) for p in cellar.glob("share/**/LICENSE*")]
            # SQLite explicitly disclaims copyright at the beginning of its public
            # API header; its Homebrew bottle ships no separate LICENSE file.
            if not licenses and name == "sqlite" and (cellar / "include/sqlite3.h").is_file():
                licenses = [str(cellar / "include/sqlite3.h")]
            return {"name": name, "version": version, "source": f"https://formulae.brew.sh/formula/{name}",
                    "roots": [str(cellar)], "licenses": licenses,
                    "metadata": [str(p) for p in [cellar / "INSTALL_RECEIPT.json", cellar / "sbom.spdx.json", cellar / ".brew" / f"{name}.rb"] if p.is_file()]}
        raise RuntimeError(f"No provenance provider for runtime {source}; add a provider with roots/source/version/licenses")

    def register(self, source):
        provider = self.provenance(source)
        key = provider["name"] + "-" + provider["version"]
        if key not in self.component_records:
            if not provider.get("source") or not provider.get("licenses"):
                raise RuntimeError(f"Missing source/license evidence: {key}")
            dest = self.licenses / re.sub(r"[^\w.+-]", "_", key)
            dest.mkdir(parents=True)
            copied = []
            for number, input_file in enumerate(provider["licenses"]):
                file = path(input_file)
                if file.is_dir():
                    for item in sorted(file.rglob("*")):
                        if item.is_file():
                            target = dest / str(number) / item.relative_to(file)
                            target.parent.mkdir(parents=True, exist_ok=True)
                            shutil.copy2(item, target)
                            copied.append(str(target.relative_to(self.app)))
                else:
                    target = dest / f"{number}-{file.name}"
                    shutil.copy2(file, target)
                    copied.append(str(target.relative_to(self.app)))
            if not copied:
                raise RuntimeError(f"Empty license input for {key}")
            for number, metadata in enumerate(provider.get("metadata", [])):
                file = path(metadata)
                shutil.copy2(file, dest / f"metadata-{number}-{file.name}")
            self.component_records[key] = {"name": provider["name"], "version": provider["version"],
                "source": provider["source"], "license_files": copied, "source_files": []}
        self.component_records[key]["source_files"].append({"path": str(source), "sha256": sha(source)})
        return key

    def copy(self, source, target, *, register=True):
        source = source.resolve(strict=True)
        digest = sha(source)
        if target in self.destinations:
            if self.destinations[target] != digest:
                raise RuntimeError(f"Conflicting runtime basename: {target}: {source}")
            return target
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        target.chmod(target.stat().st_mode | 0o200)
        self.destinations[target] = digest
        if register:
            self.register(source)
        if macho(source):
            self.relocate(source, target)
        return target

    def resolve(self, name, source, rpaths):
        if name.startswith(SYSTEM):
            return None
        candidates = []
        def expanded(value):
            return value.replace("@loader_path", str(source.parent)).replace("@executable_path", str(source.parent))
        if name.startswith("@rpath/"):
            suffix = name[len("@rpath/"):]
            candidates.extend(Path(expanded(r)) / suffix for r in rpaths)
            candidates.extend(directory / suffix for directory in self.search)
        elif name.startswith(("@loader_path/", "@executable_path/")):
            candidates.append(Path(expanded(name)))
        elif Path(name).is_absolute():
            candidates.append(Path(name))
        else:
            candidates.append(source.parent / name)
        candidates.extend(directory / Path(name).name for directory in self.search)
        for candidate in candidates:
            if candidate.is_file():
                return candidate.resolve()
        raise RuntimeError(f"Unresolved runtime {name} from {source}; searched {candidates}")

    def relocate(self, source, target):
        if target in self.processing:
            return
        self.processing.add(target)
        architectures = run(["/usr/bin/lipo", "-archs", source]).split()
        if self.arch not in architectures:
            raise RuntimeError(f"Architecture mismatch: need {self.arch}, {source} contains {architectures}")
        deps, rpaths, identity, minimum = loads(source)
        self.minimum.extend(minimum)
        # Keep the old signature until replacement: removing it first can leave modern
        # Homebrew __LINKEDIT layouts unprocessable by install_name_tool. Every image
        # is force-signed after its load commands have been rewritten.
        for name in deps:
            dependency = self.resolve(name, source, rpaths)
            if dependency is None:
                continue
            destination = self.frameworks / dependency.name
            if destination == target:
                continue
            self.copy(dependency, destination)
            replacement = "@loader_path/" + os.path.relpath(destination, target.parent)
            run(["/usr/bin/install_name_tool", "-change", name, replacement, target])
        for rpath in dict.fromkeys(rpaths):
            run(["/usr/bin/install_name_tool", "-delete_rpath", rpath, target])
        if identity:
            run(["/usr/bin/install_name_tool", "-id", "@loader_path/" + target.name, target])
        self.records[str(target.relative_to(self.app))] = {"source": str(source), "source_sha256": sha(source), "architecture": architectures}

    def python_downloader(self, source):
        settings = self.config.get("python")
        if not settings:
            raise RuntimeError("yt-dlp is a script: configure python.home and python.site_packages, or supply standalone Mach-O yt-dlp")
        home = path(settings["home"])
        version = settings["version"]
        dest = self.resources / "python"
        executable = home / "bin" / f"python{version}"
        # Homebrew's bin/python is a framework launcher which posix_spawns
        # Resources/Python.app relative to libPython. Use the real interpreter
        # entry point and explicit PYTHONHOME to avoid that external bundle path.
        framework_interpreter = home / "Resources/Python.app/Contents/MacOS/Python"
        if framework_interpreter.is_file():
            executable = framework_interpreter
        self.copy(executable, dest / "bin" / f"python{version}")
        # Copy only the stdlib and explicitly named distribution directories. Never copy .pth,
        # bytecode, sitecustomize, venv metadata or arbitrary user site packages.
        stdlib = home / "lib" / f"python{version}"
        self.register(home / "Python")
        for item in sorted(stdlib.rglob("*")):
            relative = item.relative_to(stdlib)
            if not item.is_file() or any(part in {"site-packages", "__pycache__", "test", "tests", "idle_test"} for part in relative.parts) or item.suffix == ".pyc":
                continue
            self.copy(item, dest / "lib" / f"python{version}" / relative, register=False)
        packages = dest / "site-packages"
        for site in settings["site_packages"]:
            site = path(site)
            for item in sorted(site.rglob("*")):
                relative = item.relative_to(site)
                if not item.is_file() or "__pycache__" in relative.parts or item.suffix in {".pyc", ".pth"} or item.name in {"sitecustomize.py", "usercustomize.py"}:
                    continue
                self.copy(item, packages / relative, register=False)
            for distribution in site.glob("*.dist-info"):
                metadata = distribution / "METADATA"
                lines = metadata.read_text(errors="replace").splitlines()
                name = next((x[6:] for x in lines if x.startswith("Name: ")), distribution.name)
                version_number = next((x[9:] for x in lines if x.startswith("Version: ")), "unknown")
                notices = [p for p in distribution.rglob("*") if p.is_file() and (NOTICE.match(p.name) or "licenses" in p.relative_to(distribution).parts)]
                if not notices:
                    raise RuntimeError(f"Python distribution has no license notice: {distribution}")
                key = f"python-{name}-{version_number}"
                out = self.licenses / key
                out.mkdir()
                for notice in notices:
                    target = out / notice.relative_to(distribution)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(notice, target)
                self.component_records[key] = {"name": name, "version": version_number, "source": f"https://pypi.org/project/{name}/{version_number}/", "license_files": [str(p.relative_to(self.app)) for p in out.rglob("*") if p.is_file()]}
        if not (packages / "certifi/cacert.pem").is_file():
            raise RuntimeError("Python yt-dlp requires an explicit certifi distribution and CA bundle; system/Homebrew certificate fallback is not permitted")
        # -S bypasses all site/.pth loading; bootstrap explicitly adds only packaged distributions.
        (dest / "yt-dlp-bootstrap.py").write_text("import runpy, sys\nfrom pathlib import Path\nsys.path.insert(0, str(Path(__file__).parent / 'site-packages'))\nrunpy.run_module('yt_dlp', run_name='__main__')\n")
        if not re.fullmatch(r"[0-9]+\.[0-9]+", version):
            raise RuntimeError("python.version must be major.minor")
        # MacOS/ is a code directory. Keep the stdlib in Resources and use a
        # signed native launcher so plain Python files are not misclassified as
        # unsigned nested executables by the macOS app resource seal.
        with tempfile.TemporaryDirectory(prefix="clarora-python-launcher-") as temp:
            launcher = Path(temp) / "yt-dlp"
            run(["/usr/bin/clang", "-O2", "-std=c11", "-Wall", "-Wextra", "-Werror",
                 "-arch", self.arch, "-mmacosx-version-min=14.0",
                 '-DCLARORA_PYTHON_VERSION="' + version + '"',
                 ROOT / "scripts/python-tool-launcher.c", "-o", launcher])
            self.copy(launcher, self.macos / "tools/yt-dlp", register=False)
        self.register(source)

    def rust_licenses(self):
        from packaging_licenses import collect_rust_licenses, load_sources
        defaults = ROOT / "scripts/third-party-licenses"
        override_manifest = self.config.get("crate_license_overrides_manifest", defaults / "overrides.json")
        spdx_manifest = self.config.get("spdx_license_manifest", defaults / "spdx/manifest.json")
        overrides = self.config.get("crate_license_overrides", {})
        if Path(override_manifest).is_file():
            overrides = {**load_sources(Path(override_manifest), "crate_license_overrides"), **overrides}
        spdx = self.config.get("spdx_license_texts", {})
        if Path(spdx_manifest).is_file():
            spdx = {**load_sources(Path(spdx_manifest), "license_texts"), **spdx}
        report = collect_rust_licenses(ROOT / "Cargo.toml",
            f"{self.arch.replace('arm64', 'aarch64')}-apple-darwin", self.licenses / "rust", overrides, spdx)
        self.rust_audit_required = report["upstream_license_file_absent"]
        return report["crate_count"]

    def assemble(self, binary):
        self.copy(binary, self.macos / "clarora-gpui", register=False)
        required = {"libmpv": "libmpv.2.dylib", "pdfium": "libpdfium.dylib", "ffmpeg": "tools/ffmpeg", "ffprobe": "tools/ffprobe", "asr": "tools/clarora-asr", "tts": "tools/clarora-tts", "ggml_cpu": "tools/libggml-cpu.so"}
        for component, relative in required.items():
            self.copy(path(self.config["components"][component]), self.macos / relative)
        downloader = path(self.config["components"]["yt_dlp"])
        if macho(downloader):
            self.copy(downloader, self.macos / "tools/yt-dlp")
        else:
            self.python_downloader(downloader)
        shutil.copy2(ROOT.parent / "LICENSE", self.resources / "LICENSE")
        shutil.copy2(ROOT / "README.md", self.resources / "README.md")
        (self.resources / "scripts").mkdir(exist_ok=True)
        shutil.copy2(ROOT / "scripts/PACKAGING.md", self.resources / "scripts/PACKAGING.md")
        crates = self.rust_licenses()
        self.minimum.sort(key=lambda x: tuple(map(int, x.split("."))))
        version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)
        with (self.app / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump({"CFBundleIdentifier": "com.clarora.gpui", "CFBundleName": "Clarora", "CFBundleDisplayName": "Clarora", "CFBundleExecutable": "clarora-gpui", "CFBundleIconFile": "Clarora.icns", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version, "CFBundleVersion": str(self.args.build_number), "NSHighResolutionCapable": True, "LSMinimumSystemVersion": self.minimum[-1], "NSMicrophoneUsageDescription": "录制朗读音频，用于本机保存、转写和跟读文本对齐。"}, stream)
        with tempfile.TemporaryDirectory(prefix="clarora-icon-") as temp:
            iconset = Path(temp) / "Clarora.iconset"
            iconset.mkdir()
            artwork = ROOT.parent / "clarora-app/macos/Clarora-macOS/Assets.xcassets/AppIcon.appiconset"
            for image in artwork.glob("icon_*.png"):
                shutil.copy2(image, iconset)
            run(["/usr/bin/iconutil", "--convert", "icns", iconset, "--output", self.resources / "Clarora.icns"])
        (self.resources / "runtime-manifest.json").write_text(json.dumps({"schema_version": 1, "platform": "macOS", "architecture": self.arch, "strict_runtime": True, "inventory": "package-manifest.json"}, indent=2) + "\n")
        return {"schema_version": 1, "platform": "macOS", "architecture": self.arch, "version": version,
            "minimum_macos": self.minimum[-1], "build_profile": "release" if not self.args.binary else "explicit-binary",
            "signing": "ad-hoc" if self.args.sign_identity == "-" else "Developer ID", "notarized": False,
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "source_revision": run(["git", "-C", ROOT.parent, "rev-parse", "HEAD"]).strip(),
            "source_worktree_dirty": bool(run(["git", "-C", ROOT.parent, "status", "--porcelain"]).strip()),
            "cargo_lock_sha256": sha(ROOT / "Cargo.lock"),
            "models_included": False, "rust_crate_count": crates,
            "license_legal_audit_passed": False, "rust_license_audit_required": self.rust_audit_required, "components": list(self.component_records.values()), "macho": self.records}


def write_manifest(app, manifest):
    manifest["files"] = []
    for file in sorted(app.rglob("*")):
        if not file.is_file() or "_CodeSignature" in file.parts or file.name == "package-manifest.json":
            continue
        relative = str(file.relative_to(app))
        if relative == "Contents/MacOS/clarora-gpui":
            # The outer app signature writes a resource-envelope hash into the main
            # executable, which itself covers this manifest. Avoid a circular hash:
            # OS codesign verifies the entry point; release-files.json outside the
            # app records its complete final SHA-256 after signing/stapling.
            manifest["files"].append({"path": relative, "verification": "codesign", "size": None, "sha256": None})
        else:
            manifest["files"].append({"path": relative, "verification": "sha256", "size": file.stat().st_size, "sha256": sha(file)})
    (app / "Contents/Resources/package-manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-config", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="Destination directory under workspace artifacts/clarora")
    parser.add_argument("--binary", type=Path, help="Explicit previously built binary (recorded in manifest); otherwise build release")
    parser.add_argument("--build-number", type=int, default=1)
    parser.add_argument("--sign-identity", default="-", help="Developer ID identity, or '-' for local ad-hoc signing")
    parser.add_argument("--notarize-profile", help="Existing notarytool Keychain profile; requires Developer ID signing")
    parser.add_argument("--dmg", action="store_true", help="Also create an Applications-link drag-to-install DMG")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("macOS tools are required")
    output = args.output.expanduser().resolve()
    artifacts = ROOT.parent.parent / "artifacts/clarora"
    if not output.is_relative_to(artifacts):
        parser.error(f"Output must be below {artifacts}")
    if args.notarize_profile and args.sign_identity == "-":
        parser.error("Notarization requires --sign-identity Developer ID")
    output.mkdir(parents=True, exist_ok=True)
    final = output / "Clarora.app"
    if final.exists():
        parser.error(f"Refusing to overwrite existing app: {final}; choose a fresh output directory")
    config = json.loads(args.runtime_config.read_text())
    if config.get("schema_version") != 1:
        parser.error("runtime-config schema_version must be 1")
    if not args.binary:
        subprocess.run(["cargo", "build", "--manifest-path", str(ROOT / "Cargo.toml"), "--locked", "--offline", "--release", "--bin", "clarora-gpui"], check=True)
    binary = args.binary.resolve(strict=True) if args.binary else ROOT / "target/release/clarora-gpui"
    with tempfile.TemporaryDirectory(prefix=".clarora-package-", dir=output) as temporary:
        app = Path(temporary) / final.name
        assembler = Assembler(args, app, config)
        manifest = assembler.assemble(binary)
        # Sign each nested image before the outer app. Hardened runtime only for Developer ID.
        options = [] if args.sign_identity == "-" else ["--options", "runtime", "--timestamp"]
        for relative in sorted(assembler.records, key=lambda p: p.count("/"), reverse=True):
            run(["/usr/bin/codesign", "--force", "--sign", args.sign_identity, *options, app / relative])
        write_manifest(app, manifest)
        run(["/usr/bin/codesign", "--force", "--sign", args.sign_identity, *options, app])
        run(["/usr/bin/codesign", "--verify", "--deep", "--strict", app])
        run([sys.executable, ROOT / "scripts/verify-macos-package.py", app, "--evidence", output / "verification"])
        if args.notarize_profile:
            upload = Path(temporary) / "notarization.zip"
            run(["/usr/bin/ditto", "-c", "-k", "--keepParent", app, upload])
            response = run(["/usr/bin/xcrun", "notarytool", "submit", upload, "--keychain-profile", args.notarize_profile, "--wait", "--output-format", "json"])
            (output / "notarization.json").write_text(response)
            if json.loads(response).get("status") != "Accepted":
                raise RuntimeError("Notarization was not accepted")
            run(["/usr/bin/xcrun", "stapler", "staple", app])
            run(["/usr/bin/xcrun", "stapler", "validate", app])
        shutil.move(app, final)
    (output / "release-files.json").write_text(json.dumps({"schema_version": 1,
        "files": [{"path": str(file.relative_to(final)), "size": file.stat().st_size, "sha256": sha(file)}
                  for file in sorted(final.rglob("*")) if file.is_file()]}, indent=2) + "\n")
    zip_file = output / f"Clarora-{manifest['version']}-macOS-{manifest['architecture']}.zip"
    run(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", final, zip_file])
    archives = [zip_file]
    if args.dmg:
        with tempfile.TemporaryDirectory(prefix=".clarora-dmg-", dir=output) as temporary:
            payload = Path(temporary) / "payload"
            payload.mkdir()
            run(["/usr/bin/ditto", final, payload / final.name])
            (payload / "Applications").symlink_to("/Applications")
            dmg = output / f"Clarora-{manifest['version']}-macOS-{manifest['architecture']}.dmg"
            run(["/usr/bin/hdiutil", "create", "-volname", "Clarora", "-srcfolder", payload, "-format", "UDZO", dmg])
            run(["/usr/bin/hdiutil", "verify", dmg])
            archives.append(dmg)
    report = {"schema_version": 1, "app": str(final), "architecture": manifest["architecture"], "minimum_macos": manifest["minimum_macos"], "signing": manifest["signing"], "notarized": bool(args.notarize_profile), "archives": [{"path": p.name, "size": p.stat().st_size, "sha256": sha(p)} for p in archives]}
    (output / "release.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError, ValueError, KeyError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr or "", file=sys.stderr)
        raise SystemExit(1)
