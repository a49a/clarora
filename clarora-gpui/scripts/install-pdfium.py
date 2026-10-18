#!/usr/bin/env python3
"""Install a pinned PDFium runtime from the same source used by Clarora RN."""
import argparse
import hashlib
import io
from pathlib import Path
import platform
import tarfile
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('output', type=Path)
args = parser.parse_args()
artifacts = {
    ('Darwin', 'arm64'): ('pdfium-mac-arm64.tgz', '013ecc9e0a155dabb8dd006a73a028ea542965f80a4990e5f013de283b0e58a6', 'lib/libpdfium.dylib'),
    ('Windows', 'AMD64'): ('pdfium-win-x64.tgz', 'e307d519e42f2e69b1b531f0c2a32dffcdf3891ec0eba60328ba51a57cec01ed', 'bin/pdfium.dll'),
}
key = (platform.system(), platform.machine())
if key not in artifacts:
    raise SystemExit('This installer supports Apple Silicon macOS and Windows x64. Install matching PDFium manually for other targets.')
name, digest, library = artifacts[key]
url = f'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8057/{name}'
with urllib.request.urlopen(url, timeout=60) as response:
    content = response.read(20 * 1024 * 1024)
if hashlib.sha256(content).hexdigest() != digest:
    raise SystemExit('PDFium archive SHA-256 does not match the pinned release.')
args.output.mkdir(parents=True, exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(content), mode='r:gz') as archive:
    data = archive.extractfile(library)
    if data is None:
        raise SystemExit('Runtime library missing from release.')
    target = args.output / Path(library).name
    target.write_bytes(data.read())
    for member in archive.getmembers():
        relative = Path(member.name)
        if member.isfile() and not relative.is_absolute() and '..' not in relative.parts and ('license' in member.name.lower() or member.name.startswith('licenses/')):
            license_root = args.output.parent / 'Resources' if args.output.name == 'MacOS' and args.output.parent.name == 'Contents' else args.output
            destination = license_root / 'pdfium-licenses' / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(archive.extractfile(member).read())
print(target.resolve())
