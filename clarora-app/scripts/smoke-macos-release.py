#!/usr/bin/env python3
"""Launch a packaged Release and retain repeatable smoke-test evidence."""
import argparse
import json
import hashlib
import plistlib
import subprocess
import sys
import time
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('app', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
app = args.app.resolve()
args.output.mkdir(parents=True, exist_ok=True)
report = dict(app=str(app), passed=False)
process = None
try:
    report['os_version'] = subprocess.check_output(['sw_vers', '-productVersion'], text=True).strip()
    if report['os_version'].split('.')[0] != '14':
        raise RuntimeError('Compatibility smoke test must run on macOS 14')
    subprocess.run([
        sys.executable, str(Path(__file__).with_name('verify-macos-compatibility.py')),
        str(app), '--report', str(args.output / 'compatibility.json'),
    ], check=True)
    with (app / 'Contents/Info.plist').open('rb') as source:
        info = plistlib.load(source)
    binary = app / 'Contents/MacOS' / info['CFBundleExecutable']
    report['version'] = info['CFBundleShortVersionString']
    report['build'] = info['CFBundleVersion']
    report['executable_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
    with (args.output / 'launch.log').open('w') as output:
        start = time.monotonic()
        process = subprocess.Popen([str(binary)], stdout=output, stderr=subprocess.STDOUT)
        try:
            report['exit_code'] = process.wait(timeout=10)
            raise RuntimeError(f"Release exited during startup: {report['exit_code']}")
        except subprocess.TimeoutExpired:
            report['alive_seconds'] = time.monotonic() - start
            report['passed'] = True
except Exception as error:
    report['error'] = str(error)
    raise
finally:
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
print('Packaged Release survived startup on macOS 14; evidence:', args.output)
