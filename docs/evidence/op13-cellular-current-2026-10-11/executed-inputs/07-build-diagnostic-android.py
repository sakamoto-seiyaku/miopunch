"""Build the current SDK once for the returned cellular Android pair."""
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
HERE = Path(__file__).resolve().parent
UTILITY = Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
UTILITY_SHA = '2b7e969a1f90fc93eacc80d8b62890f0bb0ef8f40dc140d9398ef1a6029671e1'
WASM = Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json')
NDK = Path('/mnt/c/Android/SDK/ndk/29.0.14206865')
BINS = ('irc_diagnostic_native',)
TRIPLE = 'aarch64-linux-android'
sys.path.insert(0, str(ROOT/'scripts'))
import cargo_cache
import build_android_pair as android

def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

def write(p, value):
    p.write_text(json.dumps(value, indent=2)+'\n')

assert sha(UTILITY) == UTILITY_SHA
spec = importlib.util.spec_from_file_location('m84_returned_android_capture', UTILITY)
sdk = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = sdk
spec.loader.exec_module(sdk)

def snapshot():
    sources = sdk.current_sources()
    raw = json.loads(WASM.read_text())
    assert raw['exit'] == 0 and raw['qualifiedForFrozenRun']
    sdk.verify_captured_graph(json.loads((WASM.parent/'selected-dependency-inputs.json').read_text()), WASM.parent, sources)
    assert all(sha(p) == h for p, h in raw['artifacts'].items())
    props = NDK/'source.properties'
    assert 'Pkg.Revision = 29.0.14206865' in props.read_text()
    compiler, archiver = android.compiler(NDK, HERE/'toolchain-probe')
    assert compiler.is_file() and archiver.is_file()
    return {'sources': sources, 'sourceDigest': sdk.json_digest(sources),
        'tools': sdk.tool_snapshot(), 'utilitySha256': sha(UTILITY),
        'driverSha256': sha(__file__), 'wasmReceiptSha256': sha(WASM),
        'compiler': str(compiler), 'archiver': str(archiver),
        'extraToolHashes': {str(p): sha(p) for p in [compiler.resolve(), archiver.resolve(), props]},
        'ndkRevision': '29.0.14206865', 'bins': list(BINS),
        'selectorPins':{str(p):sha(p) for p in [HERE/'diagnosticSelector/Cargo.toml',HERE/'diagnosticSelector/Cargo.lock',HERE/'relay_current_native.rs',HERE/'irc_diagnostic_native.rs',Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/irc_current_native.rs')]}}

def guard(expected):
    assert Path.cwd() == ROOT
    assert all(os.environ.get(k) in (None, '') for k in
        ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET'))
    assert not any(k.startswith(('CARGO_PROFILE_DEV_', 'CARGO_PROFILE_TEST_', 'CARGO_PROFILE_RELEASE_')) for k in os.environ)
    assert os.environ.get('CARGO_INCREMENTAL') in (None, '0')
    assert snapshot() == expected
    import tomllib
    selector_rows = tomllib.loads((HERE/'diagnosticSelector/Cargo.lock').read_text())['package']
    original_rows = tomllib.loads(Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/nativeSelector/Cargo.lock').read_text())['package']
    assert [r for r in selector_rows if r['name']!='mio83-replica-red-selected']==[r for r in original_rows if r['name']!='mio83-replica-red-selected']
    a=next(r for r in selector_rows if r['name']=='mio83-replica-red-selected');o=next(r for r in original_rows if r['name']=='mio83-replica-red-selected')
    assert sorted(a['dependencies'])==sorted(o['dependencies']+['socket2'])
    current_rows = tomllib.loads((ROOT/'Cargo.lock').read_text())['package']
    pin=lambda r:(r['name'],r['version'],r.get('source'),r.get('checksum'))
    assert all(pin(r) in {pin(x) for x in current_rows} for r in selector_rows if r['name']!='mio83-replica-red-selected')
    assert json.loads((HERE/'extra-selector-preparation-v3.json').read_text())['relayOnlyFixtureSha256']==sha(HERE/'relay_current_native.rs')

action = sys.argv[1]
if action == 'prepare':
    assert not (HERE/'diagnostic-build-context.json').exists()
    write(HERE/'diagnostic-build-context.json', snapshot())
    print(json.dumps({'prepared':True,'cargoStarted':False,'wasmReusedCurrent':True,'bins':BINS}))
elif action == 'build':
    expected = json.loads((HERE/'diagnostic-build-context.json').read_text())
    guard(expected)
    out = HERE/'diagnostic-android'
    assert not out.exists()
    arguments = ['build','--locked','--offline','--manifest-path',str(HERE/'diagnosticSelector/Cargo.toml'),'--target',TRIPLE,'--features','rtc-probe']
    for name in BINS:
        arguments += ['--bin', name]
    arguments += ['--jobs','2']
    target = cargo_cache.target_directory(ROOT)
    command = cargo_cache.cargo_command(arguments,target)
    env = cargo_cache.build_environment(target)
    env.update({'CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER':expected['compiler'],
        'CC_aarch64_linux_android':expected['compiler'], 'AR_aarch64_linux_android':expected['archiver'],
        'CARGO_BUILD_JOBS':'2'})
    with cargo_cache.cache_lease(target) as disk:
        guard(expected)
        out.mkdir(mode=0o700)
        write(out/'preconditions.json', {'sourceLockToolArtifactGuardsPassedBeforeCargo':True,
            'driverSha256':sha(__file__),'contextSha256':sha(HERE/'diagnostic-build-context.json'),
            'diskBefore':disk,'command':command})
        write(out/'sources-before.json', expected['sources'])
        started = time.monotonic()
        with (out/'cargo.jsonl').open('w') as stdout, (out/'cargo.stderr.log').open('w') as stderr:
            result = subprocess.run(command,cwd=ROOT,env=env,stdout=stdout,stderr=stderr)
        report = {'exit':result.returncode,'cargoExit':result.returncode,'command':command,
            'cargoSeconds':time.monotonic()-started,'nativePlatform':'android',
            'targetDirectory':str(target),'ndkRevision':expected['ndkRevision'],
            'qualifiedForFrozenRun':False,'sourceFrozenDuringBuild':False,
            'artifacts':{},'deployment':{}}
        write(out/'receipt.json',report)
        if result.returncode:
            print(json.dumps({'cargoExit':result.returncode,'qualified':False}))
            raise SystemExit(result.returncode)
        guard(expected)
        artifacts = sdk.parse_cargo_artifacts(out/'cargo.jsonl')
        selected = sdk.capture_inputs(artifacts,target,out,expected['sources'])
        pinned = out/'pinned'
        pinned.mkdir()
        for name in BINS:
            rows = [r for r in artifacts if r.get('executable') and r['target']['name']==name]
            assert len(rows)==1
            compiled = Path(rows[0]['executable'])
            shutil.copy2(compiled,pinned/name)
            assert sha(compiled)==sha(pinned/name)
            report['artifacts'][str(pinned/name)]=sha(pinned/name)
        report['deployment'] = android.prepare_deployment([pinned/n for n in BINS],out/'deploy')
        assert all(v['transformation']['exit']==0 and v['compiledSha256']==sha(pinned/n)
            and v['deployedSha256']==sha(v['deployedPath']) for n,v in report['deployment'].items())
        guard(expected)
        after = sdk.current_sources()
        write(out/'sources-after.json',after)
        sdk.verify_captured_graph(selected,out,after)
        report.update(qualifiedForFrozenRun=True,sourceFrozenDuringBuild=True,
            sourceDigestBefore=sdk.json_digest(expected['sources']),sourceDigestAfter=sdk.json_digest(after),
            freshArtifacts=sum(r.get('fresh') is True for r in artifacts),
            rebuiltArtifacts=sum(r.get('fresh') is False for r in artifacts),
            actualSelectedInputCount=len(selected['inputs']),
            actualSelectedRepositoryInputCount=len(selected['copiedRepositoryInputs']),
            actualDepfileCount=len(selected['depfiles']),diskAfter=cargo_cache.disk_state(target))
        write(out/'receipt.json',report)
        print(json.dumps({k:report[k] for k in ['exit','cargoSeconds','freshArtifacts','rebuiltArtifacts',
            'actualSelectedInputCount','actualSelectedRepositoryInputCount','actualDepfileCount','qualifiedForFrozenRun']}))
else:
    raise SystemExit('prepare or build required')
