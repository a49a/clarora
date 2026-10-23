#!/usr/bin/env python3
"""Verify a complete Windows x64 runtime tree or final portable ZIP, without PATH.

Uses only Python's standard library, so PE/import/inventory checks also run on
macOS. --native additionally loads libraries and executes tools on Windows.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import struct
import subprocess
import sys
import tempfile
import zipfile


# OS-owned libraries only. VC++/OpenMP/codec runtimes deliberately are not here.
SYSTEM_DLLS = set("""
advapi32 avicap32 avrt bcrypt bcryptprimitives cabinet cfgmgr32 combase comctl32 comdlg32
crypt32 cryptbase cryptnet cryptsp d2d1 d3d9 d3d11 d3d12 dcomp dbghelp dhcpcsvc
dnsapi dsound dwmapi dwrite dxgi dxva2 gdi32 gdi32full hid imagehlp imm32
iphlpapi kernel32 kernelbase ksuser mf mfcore mfplat mfreadwrite mfuuid mmdevapi
mpr msacm32 msctf msimg32 msvcrt mswsock ncrypt netapi32 normaliz ntdll ole32
oleacc oleaut32 opengl32 powrprof profapi propsys psapi rasapi32 rpcrt4 secur32
setupapi shcore shell32 shlwapi user32 userenv usp10 uxtheme version win32u
winhttp wininet winmm winscard winspool wintrust wlanapi wldap32 ws2_32
wtsapi32 ucrtbase urlmon winusb authz clbcatq fwpuclnt iertutil msasn1 ntasn1
ntmarta nsi samcli srvcli sspicli windowscodecs winmmbase xinput1_4 d3dcompiler_47
coremessaging coreuicomponents dinput8 glu32 msvcp_win textinputframework
uiautomationcore windows.storage windows.ui
""".split())
REQUIRED_COMPONENTS = {
    "mpv", "vulkan-loader", "ffmpeg", "yt-dlp", "pdfium", "whisper.cpp", "sherpa-onnx", "msvc-runtime", "rust-crates",
}
REQUIRED_FILES = (
    "pdfium.dll", "tools/ffmpeg.exe", "tools/ffprobe.exe", "tools/yt-dlp.exe",
    "tools/clarora-asr.exe", "tools/clarora-tts.exe", "tools/ggml-cpu.dll",
    "licenses/rust/inventory.json", "licenses/rust/Cargo.lock",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def relative_path(text):
    value = PurePosixPath(text)
    require(text and not value.is_absolute() and ".." not in value.parts
            and "\\" not in text and ":" not in text, f"Unsafe package path: {text}")
    return value


def pe_info(path):
    """Parse PE32+ normal and delay-load imports; reject other architectures."""
    data = path.read_bytes()
    require(len(data) > 64 and data[:2] == b"MZ", f"Not a PE executable: {path}")
    pe = struct.unpack_from("<I", data, 60)[0]
    require(pe + 24 <= len(data) and data[pe:pe + 4] == b"PE\0\0", f"Invalid PE header: {path}")
    machine, sections, _, _, _, optional_size, _ = struct.unpack_from("<HHIIIHH", data, pe + 4)
    require(machine == 0x8664, f"Expected x64 PE (0x8664), got {machine:#x}: {path}")
    opt = pe + 24
    require(opt + optional_size <= len(data) and struct.unpack_from("<H", data, opt)[0] == 0x20b,
            f"Expected PE32+ optional header: {path}")
    require(optional_size >= 112 + 14 * 8, f"Truncated PE data directories: {path}")
    image_base = struct.unpack_from("<Q", data, opt + 24)[0]
    size_headers = struct.unpack_from("<I", data, opt + 60)[0]
    table = opt + optional_size
    mappings = []
    for index in range(sections):
        offset = table + index * 40
        require(offset + 40 <= len(data), f"Truncated PE section table: {path}")
        virtual_size, virtual_address, raw_size, raw_start = struct.unpack_from("<IIII", data, offset + 8)
        mappings.append((virtual_address, max(virtual_size, raw_size), raw_start, raw_size))

    def rva(value, length=1):
        if value < size_headers and value + length <= len(data):
            return value
        for start, size, raw, raw_size in mappings:
            if start <= value < start + size:
                offset = raw + value - start
                require(value - start + length <= raw_size and offset + length <= len(data),
                        f"Unmapped PE RVA {value:#x}: {path}")
                return offset
        raise ValueError(f"Unknown PE RVA {value:#x}: {path}")

    def name(value):
        offset = rva(value)
        end = data.find(b"\0", offset, min(len(data), offset + 512))
        require(end >= 0, f"Unterminated import name: {path}")
        result = data[offset:end].decode("ascii").lower()
        require(result.endswith((".dll", ".drv")) and "/" not in result and "\\" not in result,
                f"Invalid DLL import {result}: {path}")
        return result

    imports = set()
    for directory, stride, name_index in ((1, 20, 3), (13, 32, 1)):
        address, size = struct.unpack_from("<II", data, opt + 112 + directory * 8)
        if not address:
            continue
        require(size >= stride, f"Invalid import directory size: {path}")
        for index in range(size // stride + 1):
            row = struct.unpack_from("<" + "I" * (stride // 4), data, rva(address + index * stride, stride))
            if not any(row):
                break
            value = row[name_index]
            if directory == 13 and not row[0] & 1:
                value -= image_base
            imports.add(name(value))
        else:
            raise ValueError(f"Unterminated import directory: {path}")
    return {"machine": "x86_64", "imports": sorted(imports)}


def system_dll(name):
    return name == "winspool.drv" or name.startswith(("api-ms-win-", "ext-ms-win-")) or name.removesuffix(".dll") in SYSTEM_DLLS


def inventory(root, excluded):
    result = []
    for path in sorted(root.rglob("*"), key=lambda path: path.relative_to(root).as_posix()):
        require(not path.is_symlink(), f"Package symlink is not allowed: {path}")
        if path.is_file() and path.relative_to(root).as_posix() not in excluded:
            result.append({"path": path.relative_to(root).as_posix(), "size": path.stat().st_size,
                           "sha256": sha256(path)})
    return result


def native_checks(root):
    require(sys.platform == "win32", "--native requires a Windows host")
    env = os.environ.copy()
    # Never satisfy media tools from a developer's PATH. OS executables stay available.
    env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")
    commands = [
        ("tools/ffmpeg.exe", ["-version"], 0),
        ("tools/ffprobe.exe", ["-version"], 0),
        ("tools/yt-dlp.exe", ["--ignore-config", "--version"], 0),
        ("tools/clarora-asr.exe", [], 2),
        ("tools/clarora-tts.exe", [], 2),
    ]
    evidence = []
    for executable, args, expected in commands:
        result = subprocess.run([str(root / executable), *args], env=env, cwd=root,
                                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=45)
        require(result.returncode == expected,
                f"Native runtime check failed ({result.returncode}): {executable}: {result.stderr[-2000:]}")
        evidence.append({"path": executable, "exit_code": result.returncode,
                         "output": (result.stdout + result.stderr)[:2000]})
    # These are dlopen dependencies, absent from the Rust executable's PE imports.
    handles = []
    try:
        for name in ("mpv-2.dll" if (root / "mpv-2.dll").is_file() else "libmpv-2.dll", "pdfium.dll", "tools/ggml-cpu.dll"):
            library = ctypes.WinDLL(str(root / name), winmode=0x00000100 | 0x00001000)
            handles.append(library)
            evidence.append({"path": name, "loaded": True})
    finally:
        # ctypes does not unload acquired DLL handles for us. Release them before
        # TemporaryDirectory removes an extracted portable ZIP on Windows.
        from _ctypes import FreeLibrary
        for library in reversed(handles):
            FreeLibrary(library._handle)
    return evidence


def verify(root, runtime=False, native=False, installed=False):
    manifest_name = "runtime-manifest.json" if runtime else "package-manifest.json"
    manifest_path = root / manifest_name
    require(manifest_path.is_file(), f"Missing {manifest_name}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    require(manifest.get("schema_version") == 1 and manifest.get("platform") == "windows"
            and manifest.get("architecture") == "x86_64", "Unsupported Windows runtime manifest")
    components = manifest.get("components", [])
    require(REQUIRED_COMPONENTS <= {item.get("name") for item in components}, "Missing runtime component provenance")
    for item in components:
        require(item.get("version") and item.get("source") and item.get("license_files"),
                f"Incomplete component provenance: {item.get('name')}")
        for text in item["license_files"]:
            path = root / relative_path(text)
            require(path.is_file() and path.stat().st_size > 0, f"Missing/empty license: {text}")
    for text in REQUIRED_FILES + (() if runtime else ("clarora-gpui.exe", "LICENSE", "README.md")):
        require((root / text).is_file() and (root / text).stat().st_size > 0, f"Missing runtime file: {text}")
    require((root / "mpv-2.dll").is_file() or (root / "libmpv-2.dll").is_file(), "Missing libmpv runtime")
    rust = json.loads((root / "licenses/rust/inventory.json").read_text(encoding="utf-8"))
    require(rust.get("target") == "x86_64-pc-windows-msvc" and rust.get("crate_count", 0) > 0
            and len(rust.get("crates", [])) == rust["crate_count"], "Invalid Rust license inventory")
    for crate in rust["crates"]:
        evidence = crate.get("license_files", []) + [item["path"] for item in crate.get("standard_license_texts", [])]
        require(evidence, f"No Rust license evidence: {crate.get('name')}")
        for text in evidence:
            path = root / "licenses/rust" / relative_path(text)
            require(path.is_file() and path.stat().st_size > 0, f"Missing Rust license evidence: {text}")
    declared = manifest.get("files", [])
    require(declared and len({x.get("path") for x in declared}) == len(declared), "Missing/duplicate inventory entries")
    installer_files = {"unins000.exe", "unins000.dat", "unins000.msg"} if installed else set()
    actual = inventory(root, {manifest_name, *installer_files})
    expected = sorted(declared, key=lambda item: item["path"])
    require(actual == expected, "Package inventory differs: missing, additional or changed files")
    graph = []
    lower_files = {path.relative_to(root).as_posix().lower(): path for path in root.rglob("*")
                   if path.is_file() and path.relative_to(root).as_posix() not in installer_files}
    require(len(lower_files) == len(actual) + 1, "Case-colliding Windows package paths")
    for path in sorted(root.rglob("*")):
        if path.suffix.lower() not in (".dll", ".exe"):
            continue
        if path.relative_to(root).as_posix() in installer_files:
            continue
        info = pe_info(path)
        relative = path.relative_to(root)
        resolved = {}
        for dependency in info["imports"]:
            if system_dll(dependency):
                resolved[dependency] = "Windows system"
                continue
            # Both app-dir and helper-dir binaries resolve beside themselves. A
            # root DLL cannot use a DLL that exists only in tools/.
            key = (relative.parent / dependency).as_posix().lower()
            require(key in lower_files, f"Unbundled DLL: {relative} -> {dependency}")
            resolved[dependency] = lower_files[key].relative_to(root).as_posix()
        graph.append({"path": relative.as_posix(), **info, "resolved": resolved})
    require(graph, "No Windows PE files found")
    return {"schema_version": 1, "status": "passed", "platform": "windows", "architecture": "x86_64",
            "runtime_only": runtime, "files_verified": len(actual), "components": components,
            "pe": graph, "native": native_checks(root) if native else "not executed"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--runtime", action="store_true")
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--installed", action="store_true", help="Allow Inno's three unins000 files in an installed tree")
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    report = {"schema_version": 1, "status": "failed", "package": str(args.package.resolve())}
    try:
        if args.package.is_file():
            with tempfile.TemporaryDirectory(prefix="clarora-windows-package-") as temporary:
                root = Path(temporary)
                with zipfile.ZipFile(args.package) as archive:
                    for item in archive.infolist():
                        relative_path(item.filename.rstrip("/"))
                        require((item.external_attr >> 16) & 0o170000 != 0o120000, "ZIP symlink is not allowed")
                    archive.extractall(root)
                report.update(verify(root, args.runtime, args.native, args.installed))
        else:
            report.update(verify(args.package.resolve(), args.runtime, args.native, args.installed))
    except Exception as error:
        report["error"] = str(error)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key not in ("pe", "components")}, ensure_ascii=False))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
