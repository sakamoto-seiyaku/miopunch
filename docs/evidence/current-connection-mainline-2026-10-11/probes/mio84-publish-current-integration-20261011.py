"""Publish scalar/hash evidence; retain original private SDK output unchanged."""
import ast
import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path('/home/js/Git/miopunch')
HERE = Path('/tmp/mio84-current-integration-20261011')
OUT = ROOT / 'docs/evidence/current-connection-mainline-2026-10-11'
OUT.mkdir(parents=True, exist_ok=True)
sys.dont_write_bytecode = True

def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

def read(p):
    return json.loads(Path(p).read_text())

def write(name, value):
    p = OUT / name
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')

ENUMS = {'initiator', 'responder', 'connect', 'accept', 'client', 'server',
    'Offerer', 'Answerer', 'None', 'Browser', 'Native', 'Complete', 'RTC',
    'DataPathUnavailable', 'DeadlineExceeded', 'PreferDirect', 'recovered',
    'permitted-relay', 'permitted-data-relay', 'configured-mqtt', 'configured-relay',
    'terminal-event-pending', 'not-observed', 'retired', 'verified', 'observe',
    'host', 'pixel', 'miopunch/session/member/1', 'miopunch/session/admission/1',
    'miopunch/session/improve/1', 'member-business', 'member-data', 'udp',
    'mqtt', 'relay', 'rtc', '0600'}
DROP = object()

def scalar_tree(value):
    if value is None or type(value) in (bool, int, float):
        return value
    if isinstance(value, str):
        return value if value in ENUMS else DROP
    if isinstance(value, list):
        return [x for v in value if (x := scalar_tree(v)) is not DROP]
    if isinstance(value, dict):
        return {k: x for k, v in value.items() if (x := (
            v if (k == 'sha256' or k.endswith('Sha256')) and isinstance(v, str)
            and re.fullmatch('[a-f0-9]{64}', v) else scalar_tree(v))) is not DROP}
    return DROP

def select(r, fields):
    return scalar_tree({k: r[k] for k in fields if k in r})

cases = {
    'public-relay-no-rtc': ('case-v6-public-relay-no-rtc', ['callerBudgetMs',
        'nativeCallerBudget', 'admissionWire', 'completeMaterials', 'noData',
        'identitiesAndGrantReopened', 'nativeCommon', 'browserCommon',
        'browserCommonAfterBusiness', 'member', 'generationCleanup',
        'nativeGenerationCleanup', 'cleanup', 'newWireQualified',
        'runtimeSourcesAndSelectedSdkArtifactsStable']),
    'mqtt-rtc-both': ('case-v6-mqtt-rtc-both', ['sdkCallerBudgetMs', 'directions',
        'bothVerifiedPeerMembershipGrantsSaved', 'publicMqttStoryQualified',
        'admissionWire', 'independentIdentities', 'cleanup', 'sdkGenerationCleanup',
        'controllerFrozenDuringRun', 'selectedSdkInputsAndArtifactsStableAfter']),
    'carrier-repair': ('binding-v4-carrier-repair', ['beforePhysicalCut', 'physicalCut',
        'currentPublicRepair', 'afterRepairCapBusinessEvidence',
        'originalOpenImmediatelyAfterCut', 'physicalCarrierRetirementBeforeCaller',
        'nativeRetirementDiagnostics', 'actualBrowserRepairProgress',
        'browserRecoveryEvidence', 'cleanup', 'nativeGenerations', 'inputsStable']),
    'relay-improve': ('binding-v5-relay-improve', ['relayBeforePreparation', 'positive',
        'cleanup', 'nativeGenerations', 'inputsStable', 'positiveQualified']),
    'rtc-no-path': ('binding-v6-rtc-no-path', ['relayBeforePreparation', 'networkFailure',
        'cleanup', 'nativeGenerations', 'inputsStable', 'networkFailureQualified']),
}
index = {}
for name, (folder, fields) in cases.items():
    directory = HERE / folder
    rawp = directory / 'runtime/receipt.json'
    raw, execution = read(rawp), read(directory / 'execution.json')
    assert raw['passed'] and execution['passed'] and execution['runtimeExit'] == 0
    assert sha(rawp) == execution['receiptSha256']
    public = {'case': name, 'qualification': 'finite-current-SDK-story',
        'rawReceipt': str(rawp), 'rawReceiptSha256': sha(rawp),
        'command': raw['command'], 'execution': execution,
        'facts': select(raw, fields), 'sdkCalls': scalar_tree(raw.get('sdkCalls', [])),
        'phases': [{k: v for k, v in phase.items() if k in
            ('name', 'ok', 'phaseWallMs', 'elapsedMs')} for phase in
            raw.get('phases', raw.get('stages', []))],
        'physicalWanQualified': False, 'rawPrivateContentPublished': False}
    write(f'{name}.json', public)
    index[name] = {'passed': True, 'runtimeExit': 0, 'wholeWallMs': execution['wholeWallMs'],
        'rawReceiptSha256': sha(rawp), 'publicFile': f'{name}.json'}

fallbackp = HERE / 'entry-fallback/receipt.json'
fallback = read(fallbackp)
assert fallback['passed'] and fallback['commandExit'] == 0
case = fallback['cases']['unavailable_member_contact_falls_back_with_original_deadline']
assert case['exit'] == 0
write('entry-fallback.json', {'case': 'entry-fallback', 'rawReceipt': str(fallbackp),
    'rawReceiptSha256': sha(fallbackp), 'passed': True,
    'physicalWanQualified': False, 'publicTraffic': False,
    'checks': fallback['checks'], 'facts': scalar_tree(case),
    'singleOriginalAbsoluteDeadlineMs': fallback['singleOriginalAbsoluteDeadlineMs'],
    'sourcesFrozenDuringBuild': fallback['sourcesFrozenDuringBuild'],
    'sourcesFrozenDuringRun': fallback['sourcesFrozenDuringRun'],
    'artifactFrozenDuringRun': fallback['artifactFrozenDuringRun'],
    'artifactSha256AfterRun': fallback['artifactSha256AfterRun']})
index['entry-fallback'] = {'passed': True, 'runtimeExit': 0, 'callerMs': 523,
    'cleanupMs': 152, 'publicFile': 'entry-fallback.json', 'rawReceiptSha256': sha(fallbackp)}

utilityp = Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
spec = importlib.util.spec_from_file_location('m84_publish_sdk', utilityp)
sdk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)
context = read(HERE / 'build-context.json')
assert sha(utilityp) == context['utilitySha256']
assert sdk.current_sources() == context['sources'] and sdk.tool_snapshot() == context['tools']
builds = {}
for name, receiptp, reusable in [
    ('native', HERE / 'native/receipt.json', False),
    ('wasm', Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json'), True),
    ('irc-native', Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/native/receipt.json'), True),
]:
    raw = read(receiptp)
    assert raw['exit'] == 0 and raw['qualifiedForFrozenRun']
    graphp = receiptp.parent / 'selected-dependency-inputs.json'
    graph = read(graphp)
    sdk.verify_captured_graph(graph, receiptp.parent, context['sources'])
    for path, digest in raw['artifacts'].items():
        assert sha(path) == digest
    builds[name] = {'rawReceipt': str(receiptp), 'rawReceiptSha256': sha(receiptp),
        'reusedWithoutRebuild': reusable, 'countsScope': 'original Cargo batch',
        'facts': select(raw, ['exit', 'cargoExit', 'cargoSeconds', 'freshArtifacts',
            'rebuiltArtifacts', 'actualSelectedInputCount', 'actualSelectedRepositoryInputCount',
            'actualDepfileCount', 'sourceDigestBefore', 'sourceDigestAfter',
            'qualifiedForFrozenRun', 'sourceFrozenDuringBuild']),
        'artifacts': raw['artifacts'], 'graphSha256': sha(graphp),
        'repositoryCopies': graph['copiedRepositoryInputs'],
        'retainedDepfileCount': len(graph['depfiles']), 'actualInputCount': len(graph['inputs'])}
    shutil.copyfile(graphp, OUT / f'{name}-selected-dependency-inputs.json')
fg = read(HERE / 'entry-fallback/selected-dependency-inputs.json')
sdk.verify_captured_graph(fg, HERE / 'entry-fallback', context['sources'])
assert sha(HERE / 'entry-fallback/contact_member_priority.test-bin') == fallback['artifactSha256AfterRun']
shutil.copyfile(HERE / 'entry-fallback/selected-dependency-inputs.json', OUT / 'entry-fallback-selected-dependency-inputs.json')
builds['entry-fallback'] = {'rawReceiptSha256': sha(fallbackp),
    'cacheReceipt': read(HERE / 'entry-fallback/build.cache.json'),
    'graphSha256': sha(HERE / 'entry-fallback/selected-dependency-inputs.json'),
    'actualInputCount': len(fg['inputs']), 'repositoryCopies': fg['copiedRepositoryInputs'],
    'retainedDepfileCount': len(fg['depfiles']),
    'artifactSha256': fallback['artifactSha256AfterRun'],
    'postRunCapture': read(HERE / 'entry-fallback/actual-input-capture.json')}
write('builds.json', builds)
write('current-cases.json', index)
write('irc-reused-current-sdk.json', {'reusedWithoutPublicRepeat': True,
    'sourceAndArtifactsRevalidated': True, 'sdkSourceDigest': sdk.json_digest(context['sources']),
    'evidence': '../irc-cid-confirmation-2026-10-11/README.md',
    'requestCallerMs': 43080.6, 'callerBudgetMs': 60000,
    'assemblyMs': 8000, 'queueMs': 30000, 'fallbackMs': 600,
    'wholeDefaultProfileQualified': False, 'physicalWanQualified': False,
    'actualApprovalSaveReopenMemberFreshAckAndPingPong': True})

failed = {}
for folder, description in [
    ('case-public-relay-no-rtc', 'source guard failed before actors: sibling compiler input'),
    ('case-v2-public-relay-no-rtc', 'preparation failed: incorrect derived pinned path'),
    ('case-v3-public-relay-no-rtc', 'preparation failed: quoting did not repair pinned path'),
    ('case-v4-public-relay-no-rtc', 'member handle returned; stale Native 12000ms assertion failed'),
    ('case-v5-public-relay-no-rtc', 'functional pass; raw Browser caller label stale; final v6 supersedes'),
    ('case-v5-mqtt-rtc-both', '12000ms member caller expired; underlying cause unknown'),
    ('binding-v3-carrier-repair', 'Native provisioner MQTT start timeout before candidate; substage unknown'),
    ('binding-v3-rtc-no-path', 'observer timeout before original 60000ms SDK caller; specialized runner used'),
    ('binding-v3-relay-improve', 'controller import path failed before actors'),
]:
    directory = HERE / folder
    entry = {'description': description, 'qualifiedAsFinalCurrentCase': False, 'retainedFiles': {}}
    for relative in ['runtime/receipt.json', 'execution.json', 'process.json', 'prepare-abort.json',
        'preparation.json', 'command.json', 'controller.stdout.log', 'controller.stderr.log']:
        p = directory / relative
        if p.is_file():
            entry['retainedFiles'][relative] = {'path': str(p), 'sha256': sha(p)}
    if (directory / 'execution.json').exists():
        entry['execution'] = select(read(directory / 'execution.json'),
            ['passed', 'runtimeExit', 'wholeWallMs', 'inputsAndArtifactsStableAfterCleanup',
             'sourceAndArtifactPostconditionsPassed', 'physicalWanQualified', 'receiptSha256'])
    failed[folder] = entry
write('retained-failures.json', failed)

owned = ['scripts/check_current_browser_relay.py', 'scripts/check_unified_browser_browser.py',
    'scripts/test_current_browser_relay_budget.py', 'browser-probe/harness/mio75-browser-pair-page.js']
probes = [Path('/tmp/mio84-current-integration-build-20261011.py'),
    Path('/tmp/mio84-current-integration-run-v6-20261011.py'),
    Path('/tmp/mio84-bound-case-execute-v6-20261011.py'),
    Path('/tmp/mio84-bound-case-execute-v5-20261011.py'),
    Path('/tmp/mio84-bound-case-execute-v4-20261011.py'),
    Path('/tmp/mio84-capture-fallback-inputs-20261011.py'),
    Path('/tmp/mio84-physical-carrier-repair-current-root-20261008/runner.py'),
    Path('/tmp/mio84-relay-first-prefer-direct-current-root-20261008/runner.py'),
    Path('/tmp/mio84-rtc-no-path-current-root-20261008/runner.py'), Path(__file__)]
manifest = {}
for p in probes:
    relative = f'probes/{p.parent.name}-{p.name}' if p.name == 'runner.py' else f'probes/{p.name}'
    dest = OUT / relative
    dest.parent.mkdir(exist_ok=True)
    shutil.copyfile(p, dest)
    manifest[relative] = {'source': str(p), 'sha256': sha(p)}
write('probe-source-manifest.json', manifest)
write('source-scope.json', {'fixedReviewBase': '29b5a2ceec62b37d6870add3744045a15cd858bb',
    'ownedProbeFiles': {n: sha(ROOT/n) for n in owned}, 'rustProductChanges': False,
    'currentSdkSourceDigest': sdk.json_digest(context['sources']),
    'originalBuildContextSha256': sha(HERE / 'build-context.json'),
    'sdkSourceGraph': context['sources'], 'tools': context['tools']})

protected = read(HERE / 'protected-wip-before.json')
assert all(sha(ROOT/n) == digest for n, digest in protected.items())
write('protected-wip.json', {'count': len(protected), 'allUnchanged': True,
    'originalSnapshotSha256': sha(HERE / 'protected-wip-before.json')})
for n in ['additional-obsolete-tests-review.json', 'additional-obsolete-tests-removal.json']:
    shutil.copyfile(HERE/n, OUT/n)

checks = []
for command in [
    ['rtk', 'proxy', 'python3', '-B', 'scripts/test_current_browser_relay_budget.py'],
    ['rtk', 'proxy', 'python3', '-B', 'scripts/check_current_browser_relay.py', '--development-checks'],
    ['rtk', 'proxy', 'node', '--check', 'browser-probe/harness/mio75-browser-pair-page.js'],
    ['rtk', 'proxy', 'git', 'diff', '--check'],
]:
    run = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    assert run.returncode == 0
    checks.append({'command': command, 'exit': run.returncode,
        'stdout': run.stdout, 'stderr': run.stderr})
for n in owned:
    if n.endswith('.py'):
        ast.parse((ROOT/n).read_text())
write('focused-checks.json', {'pythonAstPassed': True, 'checks': checks,
    'noUnchangedRustRetest': True})
print(json.dumps({'published': str(OUT), 'currentCases': len(index),
    'allSourceArtifactAndProtectedWipGuardsPassed': True}))
