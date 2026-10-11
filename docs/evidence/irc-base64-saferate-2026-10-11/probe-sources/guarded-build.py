import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import tomllib

os.umask(0o077)
ROOT = Path('/home/js/Git/miopunch')
HERE = Path(__file__).resolve().parent
context = json.loads((HERE / 'context.json').read_text())
def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
assert sha(context['utility']) == context['utilitySha256']
spec = importlib.util.spec_from_file_location('m84_build_utility', context['utility'])
sdk = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = sdk
spec.loader.exec_module(sdk)
cache = sdk.cargo_cache

def guard():
    assert Path.cwd().resolve() == ROOT
    assert sdk.current_sources() == context['sources']
    assert sdk.tool_snapshot() == context['tools']
    assert all(sha(p) == h for p, h in context['selectorPins'].items())
    current_rows = tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']
    selector_rows = tomllib.loads(Path(context['selector'], 'Cargo.lock').read_text())['package']
    original_rows = tomllib.loads(Path('/tmp/mio83-current-selected-tests-20261006/Cargo.lock').read_text())['package']
    external_rows = lambda rows: [value for value in rows if value['name'] not in {'mio83-replica-red-selected', 'miopunch-native-probe'}]
    assert external_rows(selector_rows) == external_rows(original_rows)
    original_local = next(row for row in original_rows if row['name'] == 'miopunch-native-probe')
    selector_local = next(row for row in selector_rows if row['name'] == 'miopunch-native-probe')
    assert {key: value for key, value in selector_local.items() if key != 'dependencies'} == {key: value for key, value in original_local.items() if key != 'dependencies'}
    assert sorted(selector_local['dependencies']) == sorted(original_local['dependencies'] + ['base64 0.22.1'])
    pin = lambda row: (row['name'], row['version'], row.get('source'), row.get('checksum'))
    assert all(pin(row) in {pin(value) for value in current_rows}
               for row in selector_rows if row['name'] != 'mio83-replica-red-selected')
    for witness in context['witnesses']:
        receipt_path = Path(witness['receipt'])
        assert sha(receipt_path) == witness['receiptSha256']
        receipt = json.loads(receipt_path.read_text())
        assert receipt['exit'] == 0 and receipt['qualifiedForFrozenRun']
        assert all(sha(Path(p) if Path(p).is_absolute() else receipt_path.parent/'pkg'/p) == h
                   for p, h in receipt['artifacts'].items())
        selected_path = receipt_path.parent / 'selected-dependency-inputs.json'
        assert sha(selected_path) == witness['selectedSha256']
        selected = json.loads(selected_path.read_text())
        union = set()
        for raw, expected in selected['depfiles'].items():
            captured = receipt_path.parent / 'depfiles' / Path(raw).resolve().relative_to(ROOT / 'target')
            assert sha(captured) == expected
            first = captured.read_text().replace('\\\n', ' ').splitlines()[0]
            for token in shlex.split(first.split(': ', 1)[1]):
                p = Path(token)
                union.add(str((p if p.is_absolute() else ROOT / p).resolve()))
        assert union == set(selected['inputs'])
        for p, expected in selected['inputs'].items():
            delta = witness['declaredSourceDelta'].get(p)
            if delta:
                assert delta['before'] == expected
            assert sha(p) == (delta['after'] if delta else expected)
        assert all(sha(receipt_path.parent / 'selected-source-copies' / p) == h
                   for p, h in selected['copiedRepositoryInputs'].items())

def build(label, out):
    assert not out.exists()
    guard()
    arguments = context["commands"][label]
    if "--manifest-path" in arguments:
        manifest = Path(arguments[arguments.index("--manifest-path") + 1])
        assert manifest.is_file() and str(manifest) in context["selectorPins"], "unpinned or invalid Cargo manifest"
    target = cache.target_directory(ROOT)
    with cache.cache_lease(target) as disk:
        guard()
        out.mkdir(mode=0o700)
        command = cache.cargo_command(context['commands'][label], target)
        (out/'sources-before.json').write_text(json.dumps(context['sources'],indent=2)+'\n')
        (out / 'preconditions.json').write_text(json.dumps({
            'sourceLockArtifactAndToolPreconditionsPassedBeforeCargo': True,
            'driverSha256': sha(__file__), 'contextSha256': sha(HERE / 'context.json'),
            'command': command, 'diskBefore': disk}, indent=2)+'\n')
        start = time.monotonic()
        with (out / 'cargo.jsonl').open('w') as stdout, (out / 'cargo.stderr.log').open('w') as stderr:
            result = subprocess.run(command, cwd=ROOT, env=cache.build_environment(target),
                                    stdout=stdout, stderr=stderr)
        report = {'exit': result.returncode, 'cargoExit': result.returncode, 'command': command,
                  'cargoSeconds': time.monotonic()-start, 'qualifiedForFrozenRun': False,
                  'sourceFrozenDuringBuild': False, 'artifacts': {}}
        (out / 'receipt.json').write_text(json.dumps(report, indent=2)+'\n')
        if result.returncode:
            print(json.dumps(report))
            return result.returncode
        artifacts = sdk.parse_cargo_artifacts(out / 'cargo.jsonl')
        for item in artifacts:
            if item.get('executable') and (label != 'tests' or item['target']['name'] in context['testNames']):
                source = Path(item['executable'])
                pin = out / 'pinned' / item['target']['name']
                pin.parent.mkdir(exist_ok=True)
                shutil.copy2(source, pin)
                assert sha(source) == sha(pin)
                report['artifacts'][str(pin)] = sha(pin)
        if label == 'wasm':
            source = sdk.wasm_artifact(artifacts)
            pin = out / 'miopunch_browser_probe.wasm'
            shutil.copy2(source, pin)
            assert sha(source) == sha(pin)
            command = ['rtk','proxy',str(sdk.BINDGEN),'--target','web','--out-dir',str(out/'pkg'),str(pin)]
            with (out/'bindgen.stdout.log').open('w') as stdout, (out/'bindgen.stderr.log').open('w') as stderr:
                binding = subprocess.run(command,cwd=ROOT,stdout=stdout,stderr=stderr)
            report['commands'] = [{'label':'wasm','exit':result.returncode},
                                  {'label':'bindgen','command':command,'exit':binding.returncode}]
            report['exit'] = binding.returncode
            report['artifacts'] = {str(p):sha(p) for p in (out/'pkg').rglob('*') if p.is_file()}
            report['pinnedWasmSha256'] = sha(pin)
        selected = sdk.capture_inputs(artifacts, target, out, context['sources'])
        guard()
        (out/'sources-after.json').write_text(json.dumps(sdk.current_sources(),indent=2)+'\n')
        report.update(sourceFrozenDuringBuild=True,qualifiedForFrozenRun=report['exit']==0,
                      freshArtifacts=sum(v.get('fresh') is True for v in artifacts),
                      rebuiltArtifacts=sum(v.get('fresh') is False for v in artifacts),
                      actualSelectedInputCount=len(selected['inputs']),
                      actualSourceCopyCount=len(selected['copiedRepositoryInputs']),actualSelectedRepositoryInputCount=len(selected['copiedRepositoryInputs']),
                      actualCapturedDepfileCount=len(selected['depfiles']),
                      sourceDigestBefore=sdk.json_digest(context['sources']),
                      sourceDigestAfter=sdk.json_digest(sdk.current_sources()))
        (out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report))
        return report['exit']

parser=argparse.ArgumentParser()
parser.add_argument('label',choices=('native','wasm','tests','tests-tcp'))
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args()
raise SystemExit(build(args.label,args.output.resolve()))
