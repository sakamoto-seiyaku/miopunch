"""Execute one preflight-qualified case and preserve outer exit and source stability."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

os.umask(0o077)
sys.dont_write_bytecode = True
ROOT = Path('/home/js/Git/miopunch')
HERE = Path('/tmp/mio84-current-integration-20261011')
case = sys.argv[1]
assert case in {'relay-improve', 'carrier-repair', 'rtc-no-path'}
binding = HERE / ('binding-v4-' + case)
freeze_path = binding / 'runtime-freeze.json'
frozen = json.loads(freeze_path.read_text())
command_path = binding / 'command.json'
command = json.loads(command_path.read_text())
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
sys.path.insert(0, str(ROOT / 'scripts'))
import check_unified_maintained as compiled_guard

def guard():
    assert all(sha(p) == digest for p, digest in frozen['files'].items())
    native = HERE / 'native/receipt.json'
    wasm = HERE / 'wasm-metadata/receipt.json'
    for receipt in (native, wasm):
        compiled_guard.compiled_inputs(receipt)
        value = json.loads(receipt.read_text())
        for name, digest in value['artifacts'].items():
            path = Path(name) if Path(name).is_absolute() else Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/pkg') / name
            assert sha(path) == digest
    return {'runtimeFreezeSha256': sha(freeze_path), 'commandSha256': sha(command_path),
            'executionDriverSha256': sha(__file__), 'compiledGraphsAndArtifactsCurrent': True}

before = guard()
assert not (binding / 'execution.json').exists()
assert not (binding / 'runtime').exists()
(binding / 'outer-preconditions.json').write_text(json.dumps(before, indent=2) + '\n')
started = time.monotonic()
with (binding / 'controller.stdout.log').open('w') as stdout, (binding / 'controller.stderr.log').open('w') as stderr:
    child = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    (binding / 'process.json').write_text(json.dumps({'pid': child.pid, 'command': command}) + '\n')
    code = child.wait()
stable = guard() == before
receipt = binding / 'runtime/receipt.json'
value = json.loads(receipt.read_text()) if receipt.exists() else {}
result = {'case': case, 'runtimeExit': code, 'passed': value.get('passed') is True,
          'sourceAndArtifactPostconditionsPassed': stable,
          'wholeWallMs': round((time.monotonic() - started) * 1000, 3),
          'receipt': str(receipt), 'receiptSha256': sha(receipt) if receipt.exists() else None,
          'firstFailure': value.get('firstFailure'), 'physicalWanQualified': value.get('physicalWanQualified', False)}
(binding / 'execution.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result))
raise SystemExit(code if stable else 1)
