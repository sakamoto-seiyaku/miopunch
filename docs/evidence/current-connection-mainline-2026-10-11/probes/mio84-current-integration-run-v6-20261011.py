"""One selected current SDK story at a time; immutable receipts remain intact."""
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
NATIVE = HERE / 'provisioner-metadata/receipt.json'
WASM_RAW = Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json')
WASM_META = HERE / 'wasm-metadata/receipt.json'
PACKAGE = WASM_RAW.parent / 'pkg'
FACILITY = Path('/tmp/mio76-explicit-public-emqx-facility-20261005.json')
EMPTY_PROFILE = Path('/tmp/mio83-current-purpose-implementation-20261006/browser-ice-disabled.json')
PUBLIC_PROFILE = Path('/tmp/mio74-public-actual-gathered-candidates-preparation-20261006/browser-profile-actual-gathered.json')
CHROME = Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome')
UTILITY = Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
BUILD_DRIVER = Path('/tmp/mio84-current-integration-build-20261011.py')
sys.path.insert(0, str(ROOT / 'scripts'))

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module

sdk = load('m84_run_capture_utility', UTILITY)
build_context = json.loads((HERE / 'build-context.json').read_text())
assert sha(UTILITY) == build_context['utilitySha256']
assert sdk.current_sources() == build_context['sources'] and sdk.tool_snapshot() == build_context['tools']

def metadata():
    assert not WASM_META.parent.exists()
    WASM_META.parent.mkdir(mode=0o700)
    raw = json.loads(WASM_RAW.read_text())
    selected = json.loads((WASM_RAW.parent / 'selected-dependency-inputs.json').read_text())
    assert raw['qualifiedForFrozenRun'] and raw['exit'] == 0
    sdk.verify_captured_graph(selected, WASM_RAW.parent, build_context['sources'])
    files = {str(p.relative_to(PACKAGE)): sha(p) for p in PACKAGE.rglob('*') if p.is_file()}
    assert {str(PACKAGE / name): digest for name, digest in files.items()} == raw['artifacts']
    adapted = {**raw, 'artifacts': files, 'actualDepfileCount': len(selected['depfiles']),
               'metadataProjectionOnly': True, 'sdkRebuilt': False,
               'rawReceipt': str(WASM_RAW), 'rawReceiptSha256': sha(WASM_RAW)}
    for name in ('selected-dependency-inputs.json', 'depfiles', 'selected-source-copies', 'sources-before.json', 'sources-after.json'):
        (WASM_META.parent / name).symlink_to(WASM_RAW.parent / name)
    write(WASM_META, adapted)
    print(json.dumps({'wasmMetadataOnly': True, 'rawReceiptUnchanged': True, 'selectedSourcesCurrent': True}))

def guard():
    assert sdk.current_sources() == build_context['sources'] and sdk.tool_snapshot() == build_context['tools']
    result = {}
    for label, receipt in (('native', NATIVE), ('wasm', WASM_META)):
        raw = json.loads(receipt.read_text())
        assert raw['exit'] == 0 and raw['qualifiedForFrozenRun']
        selected = json.loads((receipt.parent / 'selected-dependency-inputs.json').read_text())
        sdk.verify_captured_graph(selected, receipt.parent, build_context['sources'])
        for path, digest in raw['artifacts'].items():
            source = Path(path) if Path(path).is_absolute() else PACKAGE / path
            assert sha(source) == digest
        result[label] = sha(receipt)
    assert sha(WASM_RAW) == json.loads(WASM_META.read_text())['rawReceiptSha256']
    result['files'] = {str(p): sha(p) for p in [Path(__file__), BUILD_DRIVER, HERE / 'build-context.json',
        FACILITY, EMPTY_PROFILE, PUBLIC_PROFILE, CHROME,
        *sorted((ROOT / 'scripts').glob('*.py')),
        *sorted((ROOT / 'browser-probe/harness').glob('*.js')),
        *sorted((ROOT / 'browser-probe/harness').glob('*.mjs'))]}
    return result

def command(case, output):
    if case == 'mqtt-rtc-both':
        return ['rtk', 'proxy', 'python3', str(ROOT / 'scripts/check_unified_browser_browser.py'),
            '--case', 'both', '--member-caller-ms', '60000', '--actors', 'host-pixel', '--pixel-transport', 'usb',
            '--native-build-receipt', str(NATIVE), '--native-build-sha256', sha(NATIVE),
            '--wasm-build-receipt', str(WASM_META), '--wasm-build-sha256', sha(WASM_META),
            '--package', str(PACKAGE), '--facility', str(FACILITY),
            '--native-profile', str(PUBLIC_PROFILE), '--browser-profile', str(PUBLIC_PROFILE),
            '--host-chrome', str(CHROME), '--host-warp', 'unknown', '--pixel-overlay', 'unknown',
            '--output', str(output)]
    if case == 'public-relay-no-rtc':
        pin = Path(json.loads(NATIVE.read_text())['deployment']['unified_admission_story']['deployedPath'])
        catalog = json.loads(WASM_META.read_text())['artifacts']
        return ['rtk', 'proxy', 'python3', str(ROOT / 'scripts/check_current_browser_relay.py'),
            '--execute', '--mode', 'missing-answer',
            '--native-build-receipt', str(NATIVE), '--native-build-receipt-sha256', sha(NATIVE),
            '--native-binary', str(pin), '--native-binary-sha256', sha(pin),
            '--browser-build-receipt', str(WASM_META), '--browser-build-receipt-sha256', sha(WASM_META),
            '--package', str(PACKAGE), '--package-sha256', hashlib.sha256(json.dumps(catalog, sort_keys=True).encode()).hexdigest(),
            '--facility', str(FACILITY), '--native-profile', str(EMPTY_PROFILE), '--browser-profile', str(EMPTY_PROFILE),
            '--output', str(output)]
    raise ValueError('unknown selected case')

def run(case):
    before = guard()
    root = HERE / ('case-v6-' + case)
    assert not root.exists()
    root.mkdir(mode=0o700)
    output = root / 'runtime'
    argv = command(case, output)
    write(root / 'before.json', before)
    write(root / 'command.json', argv)
    started = time.monotonic()
    with (root / 'controller.stdout.log').open('w') as stdout, (root / 'controller.stderr.log').open('w') as stderr:
        child = subprocess.Popen(argv, cwd=ROOT, stdout=stdout, stderr=stderr)
        write(root / 'process.json', {'pid': child.pid, 'command': argv, 'startedNs': time.time_ns()})
        code = child.wait()
    stable = guard() == before
    receipt = output / 'receipt.json'
    raw = json.loads(receipt.read_text()) if receipt.exists() else {}
    result = {'case': case, 'runtimeExit': code, 'passed': raw.get('passed') is True,
              'inputsAndArtifactsStableAfterCleanup': stable,
              'wholeWallMs': round((time.monotonic() - started) * 1000, 3),
              'receipt': str(receipt), 'receiptSha256': sha(receipt) if receipt.exists() else None,
              'firstFailure': raw.get('firstFailure'), 'physicalWanQualified': raw.get('physicalWanQualified', False)}
    write(root / 'execution.json', result)
    print(json.dumps(result))
    return code if stable else 1

if sys.argv[1] == 'metadata':
    metadata()
elif sys.argv[1] in ('mqtt-rtc-both', 'public-relay-no-rtc'):
    raise SystemExit(run(sys.argv[1]))
else:
    raise SystemExit('metadata or one named SDK case required')
