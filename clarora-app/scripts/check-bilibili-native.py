#!/usr/bin/env python3
"""Run native-process integration cases and retain a JSON report (macOS only)."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('output', type=Path)
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
source = Path(__file__).with_name('check-bilibili-download.mm')
binary = output / 'downloader'
subprocess.run(['xcrun', 'clang++', '-fobjc-arc', '-fblocks', '-framework', 'Foundation', str(source), '-o', str(binary)], check=True)
report = []
with tempfile.TemporaryDirectory(prefix='clarora-download-') as temp:
    tools = Path(temp)
    # Fixtures stand in for external CLI tools; the real process, pipes, group,
    # cancellation, file validation and directory cleanup are production code.
    script = '''#!/usr/bin/python3
import json, os, pathlib, subprocess, sys, time
args = sys.argv[1:]
root = pathlib.Path(args[args.index('--paths') + 1])
path = root / '验证 [BV1xx411c7mD].mp4'
mode = os.environ['CLARORA_DOWNLOAD_CASE']
trace = pathlib.Path(os.environ['CLARORA_DOWNLOAD_TRACE'])
trace.write_text(json.dumps({'directory': str(root)}))
if mode == 'cancel':
    child = subprocess.Popen(['/bin/sleep', '60'])
    trace.write_text(json.dumps({'directory': str(root), 'child': child.pid}))
    time.sleep(60)
if mode == 'outside':
    print(json.dumps('/etc/hosts'))
else:
    path.write_bytes(b'native integration fixture')
    if mode != 'no-result': print(json.dumps(str(path)), flush=True)
if mode == 'failure':
    sys.stderr.write('x' * 200000 + ' fixture failure')
    sys.exit(7)
'''
    for name, body in [('yt-dlp', script), ('ffmpeg', '#!/bin/sh\nexit 0\n')]:
        file = tools / name
        file.write_text(body)
        file.chmod(0o755)
    env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ.get('PATH', ''))
    for case in ['success', 'failure', 'outside', 'no-result', 'cancel', 'invalid']:
        trace = tools / 'trace.json'
        if trace.exists(): trace.unlink()
        env['CLARORA_DOWNLOAD_TRACE'] = str(trace)
        env['CLARORA_DOWNLOAD_CASE'] = case
        command = [str(binary), 'invalid' if case == 'invalid' else 'https://www.bilibili.com/video/BV1xx411c7mD']
        if case == 'cancel': command.append('cancel')
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=20)
        payload = json.loads(result.stdout)
        passed = result.returncode == 0 and 'path' in payload if case == 'success' else result.returncode == 1 and 'error' in payload
        if case == 'cancel': passed = passed and payload['error'] == '已取消下载'
        if trace.exists() and case != 'success':
            details = json.loads(trace.read_text())
            passed = passed and not Path(details['directory']).exists()
            if 'child' in details:
                state = subprocess.run(['ps', '-o', 'stat=', '-p', str(details['child'])], capture_output=True, text=True).stdout.strip()
                passed = passed and (not state or state.startswith('Z'))
        if case == 'success':
            path = Path(payload['path'])
            passed = passed and path.is_file()
            path.unlink()
            path.parent.rmdir()
        report.append(dict(case=case, passed=passed, result=payload))
(output / 'native-integration.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
print(json.dumps(report, ensure_ascii=False, indent=2))
raise SystemExit(0 if all(row['passed'] for row in report) else 1)
