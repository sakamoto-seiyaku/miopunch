#!/usr/bin/env python3
"""Start this directory's configured demo independently of the SSH session."""
import json
from pathlib import Path
import subprocess
import urllib.request

root = Path(__file__).resolve().parent
config = json.loads((root / 'config.json').read_text())
url = 'http://' + config['listen']
try:
    with urllib.request.urlopen(url + '/api/status', timeout=2) as response:
        status = json.load(response)
    if status['local'].get('hostState') in ('stopped', 'failed'):
        request = urllib.request.Request(url + '/api/start', data=b'', headers={'Origin': url})
        with urllib.request.urlopen(request, timeout=30) as response:
            json.load(response)
    print(json.dumps({'running': True, 'pid': status['local']['pid'], 'url': url}))
except OSError:
    with (root / 'stdout.log').open('ab') as out, (root / 'stderr.log').open('ab') as err:
        process = subprocess.Popen(
            [str(root / 'miopunch-network-demo'), 'run', str(root / 'node'), str(root / 'config.json')],
            stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True,
        )
    print(json.dumps({'started': True, 'pid': process.pid, 'url': url}))
