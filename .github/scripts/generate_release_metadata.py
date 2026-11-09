#!/usr/bin/env python3
"""Generate desktop release checksums and metadata from uploaded macOS assets.

Run after the Windows ZIP/installer and macos-assets.json are present in the
working directory. Intended for the one-off GitHub Actions upload workflow.
"""
import hashlib
import json
import os
import pathlib


def digest(path):
    value = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def main():
    version = os.environ.get("VERSION", "0.3.0")
    tag = os.environ.get("TAG", f"v{version}")
    source_sha = os.environ["SOURCE_SHA"]
    run_id = os.environ["WINDOWS_RUN_ID"]
    repo = "https://github.com/a49a/clarora"
    macos = json.load(open("macos-assets.json"))["assets"]

    windows = []
    for name in ["Clarora-windows-x64.zip", "Clarora-windows-x64-setup-unsigned.exe"]:
        path = pathlib.Path(name)
        windows.append({"path": name, "size": path.stat().st_size, "sha256": digest(path)})

    macos_zip = next(item for item in macos if item["path"].endswith(".zip"))
    macos_dmg = next(item for item in macos if item["path"].endswith(".dmg"))
    windows_zip = next(item for item in windows if item["path"].endswith(".zip"))
    windows_setup = next(item for item in windows if item["path"].endswith(".exe"))
    sums = [macos_dmg, macos_zip, windows_setup, windows_zip]
    pathlib.Path("SHA256SUMS.txt").write_text(
        "".join(f"{item['sha256']}  {item['path']}\n" for item in sums)
    )

    assets_base = f"{repo}/releases/download/{tag}"
    provenance = {
        "source_repository": repo,
        "source_commit": source_sha,
        "tag": tag,
        "version": version,
        "application_name": "Clarora",
        "build_profile": "release",
        "build_number": 20261109,
        "source_inputs_match_commit": True,
        "macos_worktree_dirty": False,
        "excluded_uncommitted_paths": [],
        "platforms": {
            "macos": {
                "architecture": "arm64",
                "minimum_os": "14.0",
                "signing": "ad-hoc",
                "notarized": False,
            },
            "windows": {
                "architecture": "x86_64",
                "minimum_os": "10.0.17763",
                "signing": "unsigned",
                "ci_run": f"https://github.com/a49a/clarora/actions/runs/{run_id}",
                "install_runtime_overwrite_uninstall_data_preservation": "passed",
            },
        },
        "legal_audit_passed": False,
        "archives": [
            {**macos_zip, "canonical_path": f"Clarora-{version}-macOS-arm64.zip", "platform": "macos"},
            {**macos_dmg, "canonical_path": f"Clarora-{version}-macOS-arm64.dmg", "platform": "macos"},
            {**windows_zip, "canonical_path": f"Clarora-{version}-windows-x64-unsigned.zip", "platform": "windows"},
            {**windows_setup, "canonical_path": f"Clarora-{version}-windows-x64-setup-unsigned.exe", "platform": "windows"},
        ],
    }
    pathlib.Path("source-provenance.json").write_text(
        json.dumps(provenance, indent=2, ensure_ascii=False) + "\n"
    )

    metadata = {
        "version": version,
        "tag": tag,
        "source_sha": source_sha,
        "repository": repo,
        "assets_base": assets_base,
        "latest_url": f"{repo}/releases/tag/{tag}",
        "platforms": {
            "macos": {
                "arch": "arm64",
                "asset_name": "Clarora-macos-arm64.dmg",
                "status": "released",
                "requirements": "macOS 14+,Apple Silicon",
                "url": f"{assets_base}/Clarora-macos-arm64.dmg",
            },
            "windows": {
                "arch": "x64",
                "asset_name": "Clarora-windows-x64.zip",
                "status": "released",
                "requirements": "Windows 10 1809+,安装器或便携 ZIP",
                "url": f"{assets_base}/Clarora-windows-x64.zip",
            },
            "android": {
                "asset_name": "Clarora-android.apk",
                "arch": "universal",
                "status": "planned",
                "requirements": "Android 8+",
            },
            "ios": {
                "status": "planned",
                "requirements": "iOS 15.1+,需设备验证",
            },
        },
        "mobile_release": {
            "version": "0.2.1",
            "tag": "v0.2.1",
            "source_sha": "206b2d4e1ec9a734910a774d25147ac39065dc2c",
            "platforms": {
                "android": {
                    "status": "released",
                    "asset_name": "Clarora-android.apk",
                    "url": "https://github.com/a49a/clarora/releases/download/v0.2.1/Clarora-android.apk",
                },
                "ios": {"status": "planned"},
            },
        },
    }
    pathlib.Path("release-metadata.json").write_text(
        json.dumps(metadata, indent=2, ensure_ascii=False) + "\n"
    )


if __name__ == "__main__":
    main()
