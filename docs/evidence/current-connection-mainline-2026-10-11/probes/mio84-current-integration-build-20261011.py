"""Guarded current Native fixture build; reuse the exact current release WASM."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

os.umask(0o077)
sys.dont_write_bytecode = True
ROOT = Path('/home/js/Git/miopunch')
OUT = Path('/tmp/mio84-current-integration-20261011')
CONTEXT = OUT / 'build-context.json'
UTILITY = Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
UTILITY_SHA = '2b7e969a1f90fc93eacc80d8b62890f0bb0ef8f40dc140d9398ef1a6029671e1'
WASM = Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json')
BINS = ('unified_admission_story', 'unified_maintained_story')

def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

assert sha(UTILITY) == UTILITY_SHA
spec = importlib.util.spec_from_file_location('m84_current_capture_utility', UTILITY)
sdk = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = sdk
spec.loader.exec_module(sdk)

def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def wasm_guard():
    r = json.loads(WASM.read_text())
    selected = json.loads((WASM.parent / 'selected-dependency-inputs.json').read_text())
    assert r['exit'] == 0 and r['qualifiedForFrozenRun']
    before = json.loads((WASM.parent / 'sources-before.json').read_text())
    after = json.loads((WASM.parent / 'sources-after.json').read_text())
    assert before == after and sdk.json_digest(after) == r['sourceDigestBefore'] == r['sourceDigestAfter']
    sdk.verify_captured_graph(selected, WASM.parent, sdk.current_sources())
    assert all(sha(p) == h for p, h in r['artifacts'].items())
    assert len(selected['inputs']) == r['actualSelectedInputCount']
    assert len(selected['copiedRepositoryInputs']) == r['actualSelectedRepositoryInputCount']
    return {'receipt': str(WASM), 'receiptSha256': sha(WASM),
            'selectedSha256': sha(WASM.parent / 'selected-dependency-inputs.json'),
            'artifacts': r['artifacts'], 'sources': after}

def snapshot():
    return {'sources': sdk.current_sources(), 'tools': sdk.tool_snapshot(),
            'wasm': wasm_guard(), 'utilitySha256': sha(UTILITY),
            'driverSha256': sha(__file__), 'lockSha256': sha(ROOT / 'Cargo.lock'),
            'manifestSha256': sha(ROOT / 'Cargo.toml'),
            'configSha256': sha(ROOT / '.cargo/config.toml')}

def guard(expected):
    assert Path.cwd().resolve() == ROOT
    assert all(os.environ.get(k) in (None, '') for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET'))
    assert not any(k.startswith(('CARGO_PROFILE_DEV_', 'CARGO_PROFILE_TEST_', 'CARGO_PROFILE_RELEASE_')) for k in os.environ)
    assert os.environ.get('CARGO_INCREMENTAL') in (None, '0')
    assert snapshot() == expected

action = sys.argv[1]
if action == 'prepare':
    assert not OUT.exists()
    OUT.mkdir(mode=0o700)
    frozen = snapshot()
    write(CONTEXT, frozen)
    dirty = subprocess.check_output(['rtk', 'proxy', 'git', 'status', '--porcelain=v1', '--untracked-files=all', '-z'], cwd=ROOT)
    paths = [row[3:].decode() for row in dirty.split(b'\0') if row]
    write(OUT / 'protected-wip-before.json', {p: sha(ROOT / p) for p in paths if (ROOT / p).is_file()})
    print(json.dumps({'prepared': True, 'wasmReusedCurrent': True, 'sourceDigest': sdk.json_digest(frozen['sources']), 'cargoStarted': False}))
elif action == 'build':
    frozen = json.loads(CONTEXT.read_text())
    guard(frozen)
    target = sdk.cargo_cache.target_directory(ROOT)
    out = OUT / 'native'
    assert not out.exists()
    arguments = ['build', '--locked', '--offline', '--features', 'rtc-probe']
    for name in BINS:
        arguments += ['--bin', name]
    arguments += ['--jobs', '2']
    with sdk.cargo_cache.cache_lease(target) as disk:
        guard(frozen)
        out.mkdir(mode=0o700)
        command = sdk.cargo_cache.cargo_command(arguments, target)
        write(out / 'preconditions.json', {'sourceLockToolArtifactGuardsPassedBeforeCargo': True,
            'contextSha256': sha(CONTEXT), 'driverSha256': sha(__file__), 'diskBefore': disk, 'command': command})
        write(out / 'sources-before.json', frozen['sources'])
        started = time.monotonic()
        with (out / 'cargo.jsonl').open('w') as stdout, (out / 'cargo.stderr.log').open('w') as stderr:
            result = subprocess.run(command, cwd=ROOT, env=sdk.cargo_cache.build_environment(target), stdout=stdout, stderr=stderr)
        report = {'exit': result.returncode, 'cargoExit': result.returncode,
                  'cargoSeconds': time.monotonic() - started, 'command': command,
                  'qualifiedForFrozenRun': False, 'sourceFrozenDuringBuild': False,
                  'nativePlatform': 'host', 'artifacts': {}, 'deployment': {}}
        write(out / 'receipt.json', report)
        if result.returncode:
            print(json.dumps({'exit': result.returncode, 'phase': 'cargo', 'receipt': str(out / 'receipt.json')}))
            raise SystemExit(result.returncode)
        guard(frozen)
        artifacts = sdk.parse_cargo_artifacts(out / 'cargo.jsonl')
        selected = sdk.capture_inputs(artifacts, target, out, frozen['sources'])
        (out / 'pinned').mkdir()
        for name in BINS:
            rows = [r for r in artifacts if r.get('executable') and r['target']['name'] == name]
            assert len(rows) == 1
            compiled = Path(rows[0]['executable'])
            pin = out / 'pinned' / name
            shutil.copy2(compiled, pin)
            assert sha(pin) == sha(compiled)
            report['artifacts'][str(pin)] = sha(pin)
            report['deployment'][name] = {'compiledPath': str(pin), 'compiledSha256': sha(pin),
                'deployedPath': str(pin), 'deployedSha256': sha(pin),
                'transformation': {'exit': 0, 'command': 'identity; no strip or mutation'}}
        guard(frozen)
        after = sdk.current_sources()
        sdk.verify_captured_graph(selected, out, after)
        write(out / 'sources-after.json', after)
        report.update(sourceDigestBefore=sdk.json_digest(frozen['sources']), sourceDigestAfter=sdk.json_digest(after),
            sourceFrozenDuringBuild=True, qualifiedForFrozenRun=True,
            freshArtifacts=sum(r.get('fresh') is True for r in artifacts),
            rebuiltArtifacts=sum(r.get('fresh') is False for r in artifacts),
            actualSelectedInputCount=len(selected['inputs']), actualSelectedRepositoryInputCount=len(selected['copiedRepositoryInputs']),
            actualDepfileCount=len(selected['depfiles']), diskAfter=sdk.cargo_cache.disk_state(target))
        write(out / 'receipt.json', report)
        print(json.dumps({k: report[k] for k in ('exit', 'cargoSeconds', 'freshArtifacts', 'rebuiltArtifacts', 'actualSelectedInputCount', 'qualifiedForFrozenRun')}))
else:
    raise SystemExit('prepare or build required')
