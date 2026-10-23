#!/usr/bin/env python3
"""Collect target-specific Cargo license evidence for macOS or Windows packages.

Upstream LICENSE/NOTICE files and fixed-source, SHA-256 checked overrides are
preserved verbatim. When a crate declares SPDX but supplies no license file,
the original package metadata, authors, checksum and applicable SPDX standard
texts are included and the entry is explicitly flagged for legal audit. SPDX
standard texts are never presented as an invented upstream copyright notice.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

NOTICE = re.compile(r"^(licen[cs]e|copying|copyright|notice|authors|[al]?gpl|mit|bsd|isc|apache|unlicense)([._-].*)?$", re.I)


def sha256(file):
    digest = hashlib.sha256()
    with file.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_sources(file, key):
    """Resolve local evidence paths relative to their checked-in manifest."""
    if file is None:
        return {}
    file = Path(file).resolve(strict=True)
    mapping = json.loads(file.read_text(encoding="utf-8"))[key]
    for value in mapping.values():
        entries = value if isinstance(value, list) else [value]
        for entry in entries:
            local = Path(entry["path"])
            entry["path"] = str((file.parent / local).resolve(strict=True) if not local.is_absolute() else local.resolve(strict=True))
    return mapping


def verified_source(entry):
    file = Path(entry["path"]).resolve(strict=True)
    if not entry.get("source") or sha256(file) != entry["sha256"]:
        raise RuntimeError(f"License provenance/hash mismatch: {file}")
    return file


def collect_rust_licenses(manifest_path, target, destination, overrides=None, spdx_sources=None):
    manifest_path = Path(manifest_path).resolve(strict=True)
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    overrides, spdx_sources = overrides or {}, spdx_sources or {}
    result = subprocess.run(["cargo", "metadata", "--manifest-path", str(manifest_path), "--locked", "--offline",
                             "--format-version", "1", "--filter-platform", target], check=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
    metadata = json.loads(result.stdout)
    ids = {node["id"] for node in metadata["resolve"]["nodes"]}
    inventory, standards, missing = [], set(), []
    for package in sorted(metadata["packages"], key=lambda x: (x["name"], x["version"])):
        if package["id"] not in ids:
            continue
        directory = Path(package["manifest_path"]).parent
        notices = [p for p in directory.rglob("*") if p.is_file() and len(p.relative_to(directory).parts) <= 3 and NOTICE.match(p.name)]
        if package.get("license_file"):
            notices.append((directory / package["license_file"]).resolve(strict=True))
        if package["name"] == "clarora-gpui":
            notices.append(manifest_path.parent.parent / "LICENSE")
        provenance = []
        for supplied in overrides.get(package["name"] + "@" + package["version"], []):
            notices.append(verified_source(supplied))
            provenance.append({"source": supplied["source"], "sha256": supplied["sha256"]})
        expression = package.get("license")
        full_license = re.compile(r"^(licen[cs]e|copying|[al]?gpl|mit|bsd|isc|apache|unlicense)([._-].*)?$", re.I)
        if not expression and not package.get("license_file") and not any(full_license.match(p.name) for p in notices):
            raise RuntimeError(f"Rust crate has neither SPDX declaration nor license file: {package['name']} {package['version']}")
        crate = destination / (package["name"] + "-" + package["version"])
        crate.mkdir(exist_ok=True)
        files = []
        for index, notice in enumerate(dict.fromkeys(notices)):
            if not notice.is_file():
                raise RuntimeError(f"Missing upstream notice: {notice}")
            copied = crate / f"{index}-{notice.name}"
            shutil.copy2(notice, copied)
            files.append(str(copied.relative_to(destination)).replace("\\", "/"))
        checksum_file = directory / ".cargo-checksum.json"
        package_checksum = json.loads(checksum_file.read_text(encoding="utf-8")).get("package") if checksum_file.is_file() else None
        entry = {"name": package["name"], "version": package["version"], "license_expression": expression,
                 "authors": package.get("authors", []), "repository": package.get("repository"),
                 "registry_source": package.get("source"), "package_checksum_sha256": package_checksum,
                 "license_files": files, "license_sources": provenance,
                 "notice_status": "upstream-license-files-included" if files else "upstream-license-file-absent",
                 "audit_required": not bool(files)}
        if not files:
            # Preserve upstream evidence rather than fabricating a copyright holder
            # or treating a generic standard license as the upstream notice.
            for name in ["Cargo.toml", "Cargo.toml.orig", ".cargo_vcs_info.json", ".cargo-checksum.json"]:
                if (directory / name).is_file():
                    shutil.copy2(directory / name, crate / name)
            identifiers = set(re.findall(r"[A-Za-z0-9.+-]+", expression)) - {"OR", "AND", "WITH"}
            entry["standard_license_texts"] = []
            for identifier in sorted(identifiers):
                if identifier not in spdx_sources:
                    raise RuntimeError(f"SPDX standard license text missing for {identifier}, required by {package['name']}")
                source = spdx_sources[identifier]
                original = verified_source(source)
                standards.add(identifier)
                standard_directory = destination / "spdx-standard-texts"
                standard_directory.mkdir(exist_ok=True)
                copied = standard_directory / f"{identifier}.txt"
                shutil.copy2(original, copied)
                entry["standard_license_texts"].append({"identifier": identifier, "path": str(copied.relative_to(destination)).replace("\\", "/"),
                    "source": source["source"], "sha256": source["sha256"]})
            (crate / "upstream-package-metadata.json").write_text(json.dumps(entry, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            missing.append(package["name"] + "@" + package["version"])
        inventory.append(entry)
    report = {"schema_version": 1, "target": target, "crate_count": len(inventory),
              "legal_audit_passed": False, "audit_required": bool(missing), "upstream_license_file_absent": missing,
              "standard_license_texts": sorted(standards), "crates": inventory}
    (destination / "inventory.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    shutil.copy2(manifest_path.parent / "Cargo.lock", destination / "Cargo.lock")
    (destination / "README.txt").write_text(
        "This inventory preserves upstream license files and exact-source notices where supplied.\n"
        "Entries marked upstream-license-file-absent retain the original Cargo package metadata,\n"
        "authors, version, checksum and declared SPDX expression, plus the referenced standard terms.\n"
        "Standard terms do not replace an upstream copyright notice. These entries require legal audit.\n"
        "This is a reproducible component inventory, not a legal clearance or a redistribution approval.\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-path", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--output", type=Path, required=True)
    default_root = Path(__file__).resolve().parent / "third-party-licenses"
    parser.add_argument("--overrides", type=Path, default=default_root / "overrides.json")
    parser.add_argument("--spdx", type=Path, default=default_root / "spdx/manifest.json")
    args = parser.parse_args()
    report = collect_rust_licenses(args.manifest_path, args.target, args.output,
                                  load_sources(args.overrides, "crate_license_overrides"), load_sources(args.spdx, "license_texts"))
    print(json.dumps({k: v for k, v in report.items() if k != "crates"}, indent=2))


if __name__ == "__main__":
    main()
