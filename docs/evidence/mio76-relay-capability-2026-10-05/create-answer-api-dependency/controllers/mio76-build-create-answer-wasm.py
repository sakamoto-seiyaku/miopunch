import hashlib
import json
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path('/home/js/Git/miopunch')
sys.path.insert(0, str(ROOT / 'scripts'))
from build_android_pair import sources, digest

old = Path('/tmp/mio74-current-role-positive-build-20261005')
out = Path('/tmp/mio76-create-answer-current-wasm-build-20261005')
out.mkdir(mode=0o700, exist_ok=False)
target = ROOT / 'target'
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
old_receipt = json.loads((old / 'receipt.json').read_text())
old_selected = json.loads((old / 'selected-dependencies.json').read_text())
assert old_receipt['exit'] == 0 and old_receipt['sourceStable'] is True
native = old_selected['native']
assert len(native) == 276
assert all(sha(ROOT / name) == row['sha256'] for name, row in native.items())
wasm_delta = {name: {'compiledSha256': row['sha256'], 'currentSha256': sha(ROOT / name)}
              for name, row in old_selected['wasm'].items() if sha(ROOT / name) != row['sha256']}
assert set(wasm_delta) == {'browser-probe/src/rtc_capability.js'}, wasm_delta
extra = [ROOT / 'browser-probe/Cargo.toml', *(ROOT / 'browser-probe/src').rglob('*'),
         Path(__file__), Path('/tmp/mio76-create-answer-startup.mjs'), Path('/tmp/mio76-run-create-answer-startup.py')]
before = sources(extra)
(out / 'sources-before.json').write_text(json.dumps(before, indent=2) + '\n')
report = {'scope': 'CURRENT76 affected WASM necessary-createAnswer correction; reused Native276 exact selected inputs only; no new Native build/qualification, member/physical/public/STUN run',
          'targetDirectory': str(target), 'commands': [], 'priorWasmSelectedDelta': wasm_delta,
          'nativeReused': {'originalBuildReceipt': str(old / 'receipt.json'), 'originalBuildReceiptSha256': sha(old / 'receipt.json'),
                           'actualCompiledNativeInputCount': 276, 'all276CurrentCompiledInputsMatch': True,
                           'newNativeBuildPerformed': False, 'newNativeQualificationClaimed': False}}

def save():
    (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')

def run(name, command):
    start = time.monotonic()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    report['commands'].append({'name': name, 'command': command, 'exit': result.returncode, 'seconds': time.monotonic() - start})
    save()
    return result.returncode

code = run('wasm', ['rtk', 'proxy', 'cargo', 'build', '--locked', '--offline', '-p', 'miopunch-browser-probe',
                    '--features', 'rtc-probe', '--target', 'wasm32-unknown-unknown', '--jobs', '2', '--target-dir', str(target)])
if code == 0:
    code = run('bindgen', ['rtk', 'proxy', '/tmp/miopunch-wasm-tools/bin/wasm-bindgen', '--target', 'web',
                          '--out-dir', str(out / 'pkg'), str(target / 'wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm')])
after = sources(extra)
(out / 'sources-after.json').write_text(json.dumps(after, indent=2) + '\n')
report.update(exit=code, sourceStable=before == after, sourceDigestBefore=digest(before), sourceDigestAfter=digest(after))
if code == 0:
    binary = target / 'wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm'
    shutil.copy2(binary, out / binary.name)
    dep = binary.with_suffix('.d')
    shutil.copy2(dep, out / 'wasm.d')
    paths = shlex.split(dep.read_text().split('\n\n', 1)[0].split(': ', 1)[1].replace('\\\n', ' '))
    wasm = {}
    for raw in paths:
        source = Path(raw)
        source = (source if source.is_absolute() else ROOT / source).resolve()
        if not source.is_file() or not source.is_relative_to(ROOT):
            continue
        name = str(source.relative_to(ROOT))
        value = sha(source)
        copied = out / 'compiled-input-snapshots/wasm' / name
        copied.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, copied)
        wasm[name] = {'sha256': value, 'matchedBefore': before.get(name) == value, 'matchedAfter': after.get(name) == value}
    assert len(wasm) == 187 and all(v['matchedBefore'] and v['matchedAfter'] for v in wasm.values())
    assert all(sha(ROOT / name) == row['sha256'] for name, row in native.items())
    selected = {'wasm': wasm, 'native': native}
    (out / 'selected-dependencies.json').write_text(json.dumps(selected, indent=2) + '\n')
    shutil.copy2(old / 'native.d', out / 'native-reused.d')
    artifacts = {str(out / binary.name): {'compiledPath': str(binary), 'sha256': sha(out / binary.name)}}
    for name in ['browser_wss_member_native', 'browser-probe-native']:
        path = old / name
        assert sha(path) == old_receipt['artifacts'][str(path)]['sha256']
        artifacts[str(path)] = {'sha256': sha(path), 'reusedUnchanged': True, 'newBuild': False}
    for path in (out / 'pkg').rglob('*'):
        if path.is_file():
            artifacts[str(path)] = {'sha256': sha(path)}
    assert sha(next((out / 'pkg/snippets').rglob('rtc_capability.js'))) == sha(ROOT / 'browser-probe/src/rtc_capability.js')
    report.update(artifacts=artifacts, selectedCounts={'wasm': len(wasm), 'native': 276}, allSelectedInputsMatched=True,
                  snippetRtcCapabilityMatchesCurrentSource=True, broadSavedInputCount=len(before))
save()
print(json.dumps({key: report.get(key) for key in ['exit', 'sourceStable', 'selectedCounts', 'allSelectedInputsMatched', 'snippetRtcCapabilityMatchesCurrentSource']}))
raise SystemExit(code or (0 if before == after and report.get('allSelectedInputsMatched') else 2))
