#!/usr/bin/env python3
"""Reject bundled Mach-O files newer than the app's declared macOS baseline."""
import argparse
import json
import plistlib
import re
import subprocess
from pathlib import Path

MACH_MAGICS = {
    bytes.fromhex(value) for value in (
        'feedface', 'cefaedfe', 'feedfacf', 'cffaedfe',
        'cafebabe', 'bebafeca', 'cafebabf', 'bfbafeca',
    )
}


def version(value):
    if not re.fullmatch(r'\d+(?:\.\d+){0,2}', value):
        raise ValueError(f'Invalid macOS version: {value!r}')
    parts = tuple(map(int, value.split('.')))
    return parts + (0,) * (3 - len(parts))


def inspect(app):
    with (app / 'Contents/Info.plist').open('rb') as source:
        info = plistlib.load(source)
    baseline = info['LSMinimumSystemVersion']
    target = version(baseline)
    if target != version('14.0'):
        raise ValueError(f'Expected supported baseline 14.0, got {baseline}')
    executable = app / 'Contents/MacOS' / info['CFBundleExecutable']
    files, errors = [], []
    for path in sorted((app / 'Contents').rglob('*')):
        if not path.is_file():
            continue
        with path.open('rb') as source:
            magic = source.read(4)
        if magic not in MACH_MAGICS:
            continue
        arches = subprocess.check_output(['lipo', '-archs', str(path)], text=True).split()
        output = subprocess.check_output(['otool', '-arch', 'all', '-l', str(path)], text=True)
        commands = re.split(r'Load command \d+\n', output)[1:]
        minima = []
        for command in commands:
            if re.search(r'\bcmd LC_BUILD_VERSION\b', command):
                platform = re.search(r'^\s*platform (\S+)', command, re.M)
                if not platform or platform[1] not in ('1', 'MACOS', 'macos'):
                    errors.append(f'{path}: non-macOS build platform')
                match = re.search(r'^\s*minos (\S+)', command, re.M)
            elif re.search(r'\bcmd LC_VERSION_MIN_MACOSX\b', command):
                match = re.search(r'^\s*version (\S+)', command, re.M)
            else:
                continue
            if match:
                minima.append(match[1])
        if len(minima) != len(arches):
            errors.append(f'{path}: missing minimum OS metadata for one or more slices')
        if 'arm64' not in arches:
            errors.append(f'{path}: missing arm64 slice ({arches})')
        for minimum in minima:
            if version(minimum) > target:
                errors.append(f'{path}: requires macOS {minimum}, app declares {baseline}')
        files.append(dict(path=str(path.relative_to(app)), architectures=arches, minimum_os=minima))
    if not any(item['path'] == str(executable.relative_to(app)) for item in files):
        errors.append(f'Missing Mach-O executable: {executable}')
    return dict(app=str(app), minimum_os=baseline, files=files, errors=errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    try:
        report = inspect(args.app.resolve())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        report = dict(app=str(args.app), errors=[str(error)])
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    for error in report['errors']:
        print(f'COMPATIBILITY FAIL: {error}')
    if report['errors']:
        return 1
    print(f"Checked {len(report['files'])} Mach-O files: arm64, macOS {report['minimum_os']}")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
