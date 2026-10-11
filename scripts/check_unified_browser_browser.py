#!/usr/bin/env python3
"""One bounded actual Host/Pixel Browser SDK story; preparation is not qualification."""
import argparse
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
import sys
import time
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[1]
WASM_RECEIPT = Path('/tmp/mio83-current-replica-gate-wasm-first-20261006/receipt.json')
WASM_PIN = '7a9d39cd1a95bf9778fa3841877b46e512436fbddd1db7f63426738b837b6d72'
HOST_CHROME = Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome')
OWNED = ['scripts/check_unified_browser_browser.py', 'scripts/host_browser_runtime.py',
         'browser-probe/harness/mio75-browser-pair-page.js', 'browser-probe/harness/mio75-host-browser.mjs']
HELPERS = ['scripts/android_pair_runtime.py', 'scripts/host_native_runtime.py',
           'scripts/check_unified_admission.py', 'scripts/check_unified_browser_native.py',
           'scripts/wan_environment_preflight.py', 'browser-probe/harness/relay-cdp-observer.mjs',
           'browser-probe/harness/package.json', 'browser-probe/harness/package-lock.json',
           'browser-probe/harness/rtc-capability-fixture.mjs', 'scripts/check_browser_relay_capability.py']


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def source_freeze():
    return {name: sha(ROOT/name) for name in OWNED+HELPERS}


def guard_build(receipt, expected, package=None):
    """Actual selected input/copy proof, with no inactive SDK source exception."""
    require(sha(receipt) == expected, 'selected build receipt pin differs')
    build = json.loads(receipt.read_text())
    require(build.get('exit') == 0 and build.get('qualifiedForFrozenRun') is True,
            'successful source-frozen build required')
    before = json.loads((receipt.parent/'sources-before.json').read_text())
    after = json.loads((receipt.parent/'sources-after.json').read_text())
    digest = hashlib.sha256(json.dumps(after, sort_keys=True).encode()).hexdigest()
    require(before == after and digest == build.get('sourceDigestBefore') == build.get('sourceDigestAfter'),
            'captured source snapshots differ')
    selected = json.loads((receipt.parent/'selected-dependency-inputs.json').read_text())
    require(selected.get('inputs') and selected.get('copiedRepositoryInputs'), 'selected compiler proof missing')
    require(len(selected['inputs']) == build.get('actualSelectedInputCount')
        and len(selected['copiedRepositoryInputs']) == build.get('actualSelectedRepositoryInputCount'),
        'actual selected compiler input/copy count differs from the build receipt')
    for name, expected_hash in selected['inputs'].items():
        require(Path(name).is_file() and sha(name) == expected_hash, 'actual selected SDK input differs: '+name)
    for name, expected_hash in selected['copiedRepositoryInputs'].items():
        require(sha(receipt.parent/'selected-source-copies'/name) == expected_hash,
                'captured selected SDK copy differs: '+name)
    # Retained .d receipts are evidence. The shared target may be regenerated,
    # so validate the captured depfiles, never another graph's mutable target.
    for name, expected_hash in selected.get('depfiles', {}).items():
        original = Path(name)
        require(original.is_relative_to(ROOT/'target'), 'compiler depfile escapes the shared target')
        captured = receipt.parent/'depfiles'/original.relative_to(ROOT/'target')
        require(captured.is_file() and sha(captured) == expected_hash, 'captured compiler depfile differs')
    require(selected.get('depfiles'), 'captured compiler depfiles missing')
    if package:
        for name, expected_hash in build['artifacts'].items():
            require(sha(package/name) == expected_hash, 'loaded Browser package differs: '+name)
    else:
        require(build.get('nativePlatform') == 'host', 'provisioner must be the selected Host artifact')
        deployed = build['deployment']['unified_admission_story']
        require(deployed['transformation'].get('exit') == 0
            and sha(deployed['compiledPath']) == deployed['compiledSha256']
            and sha(deployed['deployedPath']) == deployed['deployedSha256'], 'provisioner artifact chain differs')
    return {'receiptSha256': expected, 'actualSelectedInputCount': len(selected['inputs']),
            'selectedRepositoryCopies': len(selected['copiedRepositoryInputs']),
            'capturedDepfileCount': len(selected['depfiles']),
            'selectedManifestSha256': sha(receipt.parent/'selected-dependency-inputs.json'),
            'allSelectedInputsMatch': True, 'inactiveSdkExceptions': [], 'artifacts': build['artifacts']}


def same_material(before, after):
    return (all(before.get(name) == after.get(name) and before.get(name) is not None
                for name in ('id', 'request', 'authorization', 'generation'))
            and after.get('stage') == 'Complete' and after.get('authorization_saved') is True
            and after.get('secret_saved') is True
            and set(before.get('records', [])).issubset(after.get('records', [])))


def business_facts(value, role):
    coordinator, bootstrap = value.get('coordinator') or {}, value.get('bootstrap') or {}
    require(value.get('newProtocol') is True and value.get('alpn') == 'miopunch/session/member/1'
        and value.get('ordinaryHandle') is True and value.get('expectedPeerMatched') is True
        and value.get('independentExpectedPeerMatched') is True and value.get('memberAuthorized') is True
        and value.get('originalMUnchanged') is True and value.get('sameDiagnosticsM') is True,
        'current Business M identity/member/new-wire proof missing')
    require(coordinator.get('event') == 'unified-path-ready' and coordinator.get('version') == 1
        and coordinator.get('purposeCode') == 1 and coordinator.get('purpose') == 'member-business'
        and coordinator.get('runtime') == 'Browser' and coordinator.get('peerRuntime') == 'Browser'
        and coordinator.get('role') == ('initiator' if role == 'connect' else 'responder')
        and coordinator.get('rtcRole') == ('Offerer' if role == 'connect' else 'Answerer')
        and coordinator.get('freshAck') is True and coordinator.get('stableUnchanged') is True
        and coordinator.get('bindingUnchanged') is True, 'common Browser Business role proof missing')
    initial, final = bootstrap.get('initialProbeWatermark'), bootstrap.get('finalReleaseProbeWatermark')
    require(type(initial) is int and type(final) is int and final > initial
        and bootstrap.get('initialFreshAck') is True and bootstrap.get('finalReleaseFreshAck') is True
        and bootstrap.get('finalized') is True and bootstrap.get('switched') is True
        and bootstrap.get('quicRole') == ('client' if role == 'connect' else 'server'),
        'initial and strictly new final exact-RTC ACK missing')
    paths = [path for path in value.get('rtcPaths', []) if path.get('pathId') == bootstrap.get('pathId')]
    require(len(paths) == 1 and coordinator.get('pathId') == paths[0].get('pathId')
        and paths[0].get('isRtc') is True and paths[0].get('selected') is True
        and paths[0].get('isRelay') is False and paths[0].get('isSignal') is False,
        'actual selected exact RTC path missing')
    channel = bootstrap.get('rtc', {}).get('channel', {})
    require(channel.get('id') == 0 and channel.get('ordered') is False
        and channel.get('negotiated') is True and channel.get('maxRetransmits') == 0,
        'actual RTC channel profile differs')
    resources = value.get('rtcHandshakeLease') or {}
    require(resources.get('active') is True and resources.get('qualified') is True
        and type(resources.get('owned_pump_tasks')) is int and resources['owned_pump_tasks'] > 0,
        'handshake exact lease/pump proof missing')
    live = value.get('liveOwnedRtc') or {}
    require(live.get('sameOwnedPeer') is True and live.get('pcState') == 'connected'
        and live.get('channelState') == 'open' and live.get('channelId') == 0
        and live.get('localDescriptionType') == ('offer' if role == 'connect' else 'answer')
        and live.get('remoteDescriptionType') == ('answer' if role == 'connect' else 'offer')
        and live.get('negotiated') is True and live.get('ordered') is False
        and live.get('maxRetransmits') == 0 and live.get('observerDetached') is False,
        'live original-M owned SDK PC/DC observation missing')
    return {'newWire': True, 'expectedPeerMatched': True, 'currentMemberGate': True,
        'runtime': 'Browser', 'peerRuntime': 'Browser', 'role': role, 'quicRole': bootstrap['quicRole'],
        'rtcRole': coordinator['rtcRole'], 'originalMUnchanged': True, 'sameDiagnosticsM': True,
        'initialFreshAck': True, 'initialWatermark': initial, 'newFinalFreshAck': True, 'finalWatermark': final,
        'exactRtcSelected': True, 'channelId': 0, 'handshakeOwnedPumpTasks': resources['owned_pump_tasks'],
        'liveOwnedPeerMatched': True,
        'pathCounters': {name: paths[0].get(name) for name in ('sentBytes', 'receivedBytes',
            'streamFramesSent', 'streamFramesReceived')},
        'liveChannelCounters': {name: live.get(name) for name in ('sentPackets', 'receivedPackets', 'sentBytes', 'receivedBytes')}}


def require_business_delta(before, after):
    for category in ('pathCounters', 'liveChannelCounters'):
        for name, old in before[category].items():
            new = after[category][name]
            require(type(old) is int and type(new) is int and new > old,
                    'real bidirectional original-M RTC stream/pump delta missing: '+name)


class EvidenceCheckFailure(RuntimeError):
    def __init__(self, check):
        self.check = check
        super().__init__('required evidence failed: '+check)


def evidence(condition, check):
    if not condition:
        raise EvidenceCheckFailure(check)


def remember_first_failure(report, error, stage, *, actor='controller', check=None):
    """Only bounded caller-owned names and exception classes cross the public boundary."""
    safe = lambda value: isinstance(value, str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9:_-]{0,95}', value)
    evidence(safe(stage) and safe(actor), 'failure-location-identifier')
    identifier = check or getattr(error, 'check', 'operation-failed')
    evidence(safe(identifier), 'failure-check-identifier')
    if 'firstFailure' not in report:
        report['firstFailure'] = {'actor': actor, 'stage': stage, 'check': identifier,
                                  'failureClass': type(error).__name__}
        return True
    return False


def rtc_retirement_facts(observed, *, expected_count=None, expected_ids=None, require_late=False):
    """Validate exact privately correlated leases, then publish only resource facts."""
    reports = observed.get('retiredMemberReports')
    evidence(isinstance(reports, dict), 'rtc-retired-report-map-present')
    if expected_ids is not None:
        evidence(isinstance(expected_ids, set) and all(isinstance(key, str) for key in expected_ids),
                 'rtc-owned-lease-identities-present')
        evidence(expected_ids.issubset(reports), 'rtc-owned-lease-reports-present')
        selected = {key: reports[key] for key in expected_ids}
    else:
        evidence(type(expected_count) is int and expected_count >= 0, 'rtc-generation-allocation-count-known')
        selected = {key: report for key, report in reports.items() if isinstance(report, dict) and 'rtcCleanup' in report}
        evidence(len(selected) == expected_count, 'rtc-generation-owned-lease-report-count')
    leases = []
    for key, report in selected.items():
        evidence(isinstance(report, dict) and isinstance(report.get('rtcCleanup'), list)
                 and bool(report['rtcCleanup']), 'rtc-owned-lease-samples-present')
        samples = report['rtcCleanup']
        phases = [sample.get('phase') for sample in samples if isinstance(sample, dict)]
        evidence(len(phases) == len(samples) and phases.count('immediate') == 1
                 and phases.count('late') <= 1 and set(phases).issubset({'immediate', 'late'}),
                 'rtc-owned-lease-sample-phases')
        if require_late:
            evidence(phases.count('late') == 1, 'rtc-owned-lease-late-sample-present')
        published = []
        for sample in samples:
            lease = sample.get('lease')
            evidence(isinstance(lease, dict) and key.isdecimal()
                and type(lease.get('original_stable_id')) is int
                and str(lease['original_stable_id']) == key, 'rtc-owned-lease-original-m-matched')
            phase = sample['phase']
            facts = {name: lease.get(name) for name in ('active', 'qualified', 'owned_pump_tasks', 'tx_bytes', 'rx_bytes')}
            facts.update(phase=phase, pcClosed=sample.get('pcClosed'), dataChannelState=sample.get('dataChannelState'))
            evidence(facts['active'] is False and facts['qualified'] is False
                and type(facts['tx_bytes']) is int and facts['tx_bytes'] == 0
                and type(facts['rx_bytes']) is int and facts['rx_bytes'] == 0,
                'rtc-owned-lease-retired-empty-queues')
            evidence(type(facts['owned_pump_tasks']) is int and facts['owned_pump_tasks'] >= 0
                and (phase != 'late' or facts['owned_pump_tasks'] == 0), 'rtc-owned-lease-pump-settled')
            evidence(facts['pcClosed'] is True and facts['dataChannelState'] in ('closing', 'closed')
                and (phase != 'late' or facts['dataChannelState'] == 'closed'), 'rtc-owned-pc-dc-retired')
            published.append(facts)
        leases.append({'samples': published})
    return leases


def sdk_cleanup_facts(value):
    witness = value.get('ownedRtcAllocationWitness')
    evidence(isinstance(witness, dict) and type(witness.get('allocatedPeerCount')) is int
        and witness['allocatedPeerCount'] >= 0 and isinstance(witness.get('knownStableIds'), list)
        and type(witness.get('generation')) is int and witness['generation'] >= 0
        and witness['generation'] == value.get('generation')
        and all(type(stable) is int for stable in witness['knownStableIds']), 'rtc-generation-allocation-witness')
    count = witness['allocatedPeerCount']
    known = {str(stable) for stable in witness['knownStableIds']}
    evidence(len(known) <= count, 'rtc-generation-known-m-count')
    if value.get('networkRequired') is False:
        objects = value.get('realRtcObjects') or {}
        evidence(count == 0 and not known and objects.get('openPeers') == 0 and objects.get('openChannels') == 0,
                 'unstarted-network-no-owned-rtc-allocation')
        return {'scope': 'no-started-sdk-network', 'sdkNetworkObservationsRequired': False,
            'ownedRtcAllocationCount': 0, 'knownOriginalMCount': 0,
            'realRtcObjects': objects, 'cleanupSucceeded': value.get('cleanupSucceeded'),
            'networkClosed': value.get('networkClosed'), 'networkFreed': value.get('networkFreed'),
            'storage': value.get('storage'), 'errors': value.get('errors', [])}
    evidence(value.get('networkRequired') is True, 'rtc-generation-network-required-known')
    diagnostics = value.get('networkDiagnostics')
    evidence(isinstance(diagnostics, list), 'rtc-generation-network-samples-present')
    samples = []
    for sample in diagnostics:
        observed, objects = sample.get('observed', {}), sample.get('realRtcObjects', {})
        delay = sample.get('delayMs')
        evidence(type(delay) is int and delay in (0, 100), 'rtc-generation-sample-time')
        evidence(type(observed.get('activeAssociations')) is int and observed['activeAssociations'] == 0
            and type(observed.get('activePumpTasks')) is int and observed['activePumpTasks'] == 0
            and type(objects.get('openPeers')) is int and objects['openPeers'] == 0,
            'rtc-generation-no-live-owned-rtc')
        evidence(type(objects.get('openChannels')) is int and type(objects.get('closingChannels')) is int
            and (objects['openChannels'] == 0 if delay == 100 else objects['openChannels'] == objects['closingChannels']),
            'rtc-generation-channel-retirement')
        evidence(type(observed.get('cleanupObservers')) is int and observed['cleanupObservers'] >= 0
            and (delay != 100 or observed['cleanupObservers'] == 0), 'rtc-generation-late-observers-settled')
        peers = objects.get('peers')
        evidence(isinstance(peers, list) and len(peers) == count and all(peer.get('closed') is True
            and peer.get('callbacksDetached') is True and peer.get('ownObserversDetached') is True
            and peer.get('dataChannelCallbacksDetached') is True for peer in peers),
            'rtc-generation-owned-peer-callbacks-detached')
        leases = rtc_retirement_facts(observed, expected_count=count, require_late=delay == 100)
        reports = observed['retiredMemberReports']
        evidence(known.issubset(reports) and all('rtcCleanup' in reports[key] for key in known),
                 'rtc-generation-known-original-ms-retired')
        samples.append({'delayMs': delay, 'activeAssociations': observed['activeAssociations'],
            'activePumpTasks': observed['activePumpTasks'], 'cleanupObservers': observed['cleanupObservers'],
            'realRtcObjects': objects, 'retiredLeases': leases})
    evidence([sample['delayMs'] for sample in samples] == [0, 100], 'rtc-generation-immediate-and-late-samples')
    return {'cleanupSucceeded': value.get('cleanupSucceeded'), 'networkClosed': value.get('networkClosed'),
        'networkFreed': value.get('networkFreed'), 'ownedMemberHandles': value.get('ownedMemberHandles'),
        'generation': witness.get('generation'), 'ownedRtcAllocationCount': count,
        'knownOriginalMCount': len(known), 'lateHandleRetirements': value.get('lateHandleRetirements'),
        'samples': samples, 'storage': value.get('storage'), 'errors': value.get('errors', [])}


def cancelled_lease_facts(baseline, observed, objects, *, late):
    """C's new exact M records, not interchangeable aggregate zeros for H and C."""
    previous = baseline.get('retiredMemberReports')
    current = observed.get('retiredMemberReports')
    evidence(isinstance(previous, dict) and isinstance(current, dict), 'cancel-lease-map-observed')
    owned_c = set(current)-set(previous)
    evidence(len(owned_c) == 1, 'cancel-one-new-owned-m-lease')
    evidence(type(objects.get('openPeers')) is int and objects['openPeers'] == 0
        and type(objects.get('openChannels')) is int and type(objects.get('closingChannels')) is int
        and (objects['openChannels'] == 0 if late else objects['openChannels'] == objects['closingChannels'])
        and isinstance(objects.get('peers'), list) and len(objects['peers']) == 1
        and all(peer.get('closed') is True and peer.get('callbacksDetached') is True
                and peer.get('ownObserversDetached') is True and peer.get('dataChannelCallbacksDetached') is True
                for peer in objects['peers']),
        'cancel-real-owned-peer-retirement')
    for field in ('activeAssociations', 'activePumpTasks'):
        evidence(type(observed.get(field)) is int and type(baseline.get(field)) is int
            and baseline[field] > 0 and observed[field] == baseline[field], 'cancel-healthy-resource-baseline-'+field)
    evidence(type(observed.get('cleanupObservers')) is int and observed['cleanupObservers'] >= 0
        and (not late or observed['cleanupObservers'] == 0), 'cancel-late-observers-settled')
    leases = rtc_retirement_facts(observed, expected_ids=owned_c, require_late=late)
    return {'ownedCancelledLeaseCount': 1, 'retiredLeases': leases,
        'newRealPeerClosed': True, 'callbacksDetached': True,
        'activeAssociations': observed['activeAssociations'], 'activePumpTasks': observed['activePumpTasks'],
        'cleanupObservers': observed['cleanupObservers']}


def run(args):
    # Every source/artifact guard precedes Device construction/listener/profile/SDK.
    proof = {'browser': guard_build(args.wasm_build_receipt, args.wasm_build_sha256, args.package),
             'provisioner': guard_build(args.native_build_receipt, args.native_build_sha256)}
    freeze = source_freeze()
    facility = json.loads(args.facility.read_text())
    browser_profile = json.loads(args.browser_profile.read_text())
    public_wss = urlparse(facility.get('wss_url', ''))
    require(public_wss.scheme == 'wss' and public_wss.hostname
        and public_wss.hostname not in ('localhost', '127.0.0.1', '::1') and not public_wss.username,
        'existing public MQTT/WSS normal-trust facility required')
    wss_host = '['+public_wss.hostname+']' if ':' in public_wss.hostname else public_wss.hostname
    wss_origin = 'wss://'+wss_host+((':'+str(public_wss.port)) if public_wss.port not in (None, 443) else '')
    require(not args.output.exists(), 'output exists; preserve original first failure')
    require(args.host_chrome.is_file() and os.access(args.host_chrome, os.X_OK), 'selected Host Chrome unavailable')
    args.output.mkdir(mode=0o700, parents=True)
    private_path = args.output/'private-sdk.jsonl'
    private_path.touch(mode=0o600)
    from android_pair_runtime import Device, browser_cleanup_succeeded
    from host_native_runtime import HostNativeActor, host_native_cleanup_succeeded
    from host_browser_runtime import PairPage, HostBrowserPage, PixelCdpObserver
    from check_unified_admission import require_admission_wire, require_complete, require_replica, native_command, \
        current_native_events, current_native_stderr, persist_browser_start_observation
    from check_unified_browser_native import close_native
    actor, pages, device = None, {}, None
    pixel_observer = None
    native_replies = []
    observation_sequence = 0
    named_attempts = {}
    phase = 'setup'
    host_pair = args.actors == 'host-host'
    report = {'scope': ('actual two owned Host Browsers common SDK story' if host_pair
                       else 'actual Host Browser and Pixel Browser common SDK story'), 'passed': False,
        'actors': {'host': 'Host Browser A' if host_pair else 'Host Browser',
                   'pixel': 'Host Browser B' if host_pair else 'Pixel Browser'},
        'actorLabels': 'stable controller aliases; see actors for actual endpoints',
        'deviceOperation': not host_pair,
        'physicalWanQualified': False, 'sdkCallerBudgetMs': args.member_caller_ms, 'case': args.case,
        'sourceHashesBefore': freeze, 'compiledInputs': proof,
        'command': ['rtk', 'proxy', 'python3', str(Path(__file__).resolve()), *sys.argv[1:]],
        'configHashes': {name: sha(getattr(args, name)) for name in ('facility', 'browser_profile', 'native_profile')},
        'hostChromeSha256': sha(args.host_chrome), 'publicMqttStoryQualified': False,
        'publicMqttOrigin': wss_origin,
        'environment': {'hostWarp': args.host_warp, 'pixelOverlay': None if host_pair else args.pixel_overlay,
            'secondHostWarp': args.host_warp if host_pair else None,
            'disclosureSource': 'caller; no route/VPN mutation or inferred NAT proof',
            'physicalSeparateNetworks': False if host_pair else None,
            'physicalUdpSocketUidRoute': None}, 'stages': [], 'sdkCalls': []}

    def save():
        (args.output/'receipt.json').write_text(json.dumps(report, indent=2)+'\n')

    def raw(label, value):
        with private_path.open('a') as stream:
            stream.write(json.dumps({'label': label, 'value': value})+'\n')

    def stage(label, operation):
        nonlocal phase
        phase = label
        started = time.monotonic()
        try:
            result = operation()
        except Exception as error:
            actor_label = label.split('-')[0] if label.split('-')[0] in ('host', 'pixel', 'provider') else 'controller'
            first = remember_first_failure(report, error, label, actor=actor_label)
            raw('first-failure:'+label, {'first': first, 'failure': str(error)})
            report['stages'].append({'name': label, 'ok': False})
            save(); raise
        report['stages'].append({'name': label, 'ok': True,
            'controllerElapsedMs': round((time.monotonic()-started)*1000, 3)})
        save(); print(json.dumps({'event': 'stage', 'name': label, 'ok': True}), flush=True)
        return result

    def keep_start_observation(label):
        nonlocal observation_sequence
        observation_sequence += 1
        observer = pages[label] if isinstance(pages[label], HostBrowserPage) else pixel_observer
        try:
            facts = persist_browser_start_observation(args.output, pages[label], observer,
                label+'-failed-start-'+str(observation_sequence))
            report.setdefault('browserStartObservations', []).append({'actor': label, **facts})
        except Exception as error:
            raw(label+'-start-observation-retention-failure', {'failure': str(error)})
            report.setdefault('startObservationRetentionFailures', []).append({'actor': label, 'failureClass': type(error).__name__})

    def sdk(label, operation, values=None):
        nonlocal phase
        phase = label+':'+operation
        try:
            reply = pages[label].call(operation, values)
        except Exception as error:
            raw(phase+'-rpc-exception', {'failureClass': type(error).__name__, 'failure': str(error)})
            remember_first_failure(report, error, phase, actor=label, check='sdk-rpc-exception')
            if operation == 'start':
                keep_start_observation(label)
            save(); raise
        raw(phase, reply)
        report['sdkCalls'].append({'actor': label, 'operation': operation,
            'ok': reply.get('ok') is True, 'controlRpcElapsedMs': reply.get('callerElapsedMs')})
        if reply.get('ok') is not True:
            error = EvidenceCheckFailure('sdk-returned-failure-dto')
            remember_first_failure(report, error, phase, actor=label)
            raw(phase+'-native-observation', {'events': current_native_events(actor), 'stderr': current_native_stderr(actor)})
            if operation == 'start':
                keep_start_observation(label)
            save(); raise error
        if operation in ('begin_connect', 'begin_accept'):
            named_attempts.setdefault(label, []).append(values['name'])
        return reply['result']

    def command(operation, expected, **values):
        try:
            event = native_command(actor, operation, expected, native_replies, **values)
        except Exception as error:
            raw('native:'+operation+'-failure', {'failure': str(error), 'replies': native_replies,
                'events': current_native_events(actor), 'stderr': current_native_stderr(actor)})
            raise
        raw('native:'+operation, event)
        return event

    def config(target):
        return {'relays': [], 'disabled': [], 'irc': [], 'rtc_enabled': True, 'mqtt': [facility],
            'rtc_ice': browser_profile, 'contact_routes': [{'contact': target['id'],
                'facility': facility['logical_id'], 'address': target['route']}]}

    def route(label):
        carriers = sdk(label, 'path_diagnostics').get('carriers', [])
        ready = [carrier for carrier in carriers if isinstance(carrier.get('route'), str)
                 and carrier.get('ready') is True and carrier.get('closed') is False]
        require(len(ready) == 1, 'one current real MQTT ready route required')
        return {'id': sdk(label, 'status')['id'], 'route': ready[0]['route']}

    def stop(label):
        cleanup = sdk(label, 'stop')
        require(cleanup.get('cleanupSucceeded') is True and cleanup.get('ownedMemberHandles') == 0
            and cleanup.get('networkClosed') is True and cleanup.get('networkFreed') is True,
            'owned SDK stop failed')
        report.setdefault('sdkGenerationCleanup', []).append({'actor': label,
            'generation': cleanup.get('generation'), 'sdkCleanupElapsedMs': cleanup.get('cleanupElapsedMs'),
            'resources': sdk_cleanup_facts(cleanup), 'storePreserved': True})
        return cleanup

    def wait_result(label, operation, values, predicate, timeout=None):
        if timeout is None:
            timeout = args.member_caller_ms / 1000 + 2
        deadline = time.monotonic()+timeout
        while time.monotonic() < deadline:
            result = sdk(label, operation, values)
            if predicate(result):
                return result
            time.sleep(0.02)
        raise TimeoutError('local settlement observation expired; SDK remains bound to its selected caller')

    def establish(dialer, answerer, name):
        args75 = {'name': name, 'callerMs': args.member_caller_ms}
        sdk(answerer, 'begin_accept', {**args75, 'expectedPeer': identities[dialer]})
        sdk(dialer, 'begin_connect', {**args75, 'expectedPeer': identities[answerer]})
        facts = {}
        for label, role in ((dialer, 'connect'), (answerer, 'accept')):
            wait_result(label, 'attempt_status', {'name': name}, lambda result: result['settled'])
            settled = sdk(label, 'finish_attempt', {'name': name})
            require(settled.get('returned') is True and settled.get('expectedPeerMatched') is True
                and settled.get('cancelled') is False and settled.get('retirementErrors') == []
                and settled.get('callerBudgetMs') == args.member_caller_ms
                and settled.get('callerElapsedMs', args.member_caller_ms) < args.member_caller_ms,
                'ordinary SDK handle did not qualify within the selected original caller')
            facts[label] = {key: settled[key] for key in ('role', 'callerElapsedMs', 'callerBudgetMs', 'expectedPeerMatched')}
            facts[label]['business'] = business_facts(sdk(label, 'member_diagnostics', {'name': name}), role)
        return facts

    exchange_sequence = 0

    def streams(dialer, answerer, name):
        nonlocal exchange_sequence
        before = {label: business_facts(sdk(label, 'member_diagnostics', {'name': name}), role)
            for label, role in ((dialer, 'connect'), (answerer, 'accept'))}
        for client, server in ((dialer, answerer), (answerer, dialer)):
            exchange_sequence += 1
            operation = 'stream-'+str(exchange_sequence)
            message = 'MIO75 '+operation+' '+client+' to '+server
            sdk(server, 'begin_serve_exchange', {'name': name, 'operation': operation})
            require(sdk(client, 'exchange', {'name': name, 'message': message}) == message,
                    'genuine original-M business reply differs')
            wait_result(server, 'serve_status', {'operation': operation}, lambda result: result['settled'])
            require(sdk(server, 'finish_serve_exchange', {'operation': operation})['message'] == message,
                    'genuine SDK server receive differs')
        after = {label: business_facts(sdk(label, 'member_diagnostics', {'name': name}), role)
            for label, role in ((dialer, 'connect'), (answerer, 'accept'))}
        for label in before:
            require_business_delta(before[label], after[label])
        return {'bidirectionalStreamsMatched': True, 'originalMBusinessAfter': after}

    def reopen_dialer(dialer, answerer):
        live = route(answerer)
        stop(dialer)
        require(sdk(dialer, 'start', {'configuration': config(live)})['id'] == identities[dialer],
                'dialer persistent identity changed')
        require(same_material(materials[dialer], sdk(dialer, 'material_receipt')), 'dialer persistent grant/materials changed')
        return {'dialerOnlyReopened': True, 'liveAnswererRoute': True, 'sameIdentity': True, 'sameMaterials': True}

    try:
        actor = stage('owned-current-provisioner', HostNativeActor)
        build = json.loads(args.native_build_receipt.read_text())
        actor.deploy(build['deployment']['unified_admission_story']['deployedPath'], 'provisioner', executable=True)
        facility_path = actor.deploy(args.facility, 'facility.json')
        native_profile_path = actor.deploy(args.native_profile, 'ice-profile.json')
        actor.start([actor.directory+'/provider', facility_path, native_profile_path, '--allow-unqualified-preparation'])
        ready = stage('provider-ready', lambda: actor.wait_event('ready'))
        raw('provider-ready', ready)
        require(ready.get('normalPublicTrust') is True, 'provisioner normal public trust missing')
        if not host_pair:
            device = stage('authorized-Pixel-selection', lambda: Device('Pixel_6a', args.pixel_transport))
        fixture = ROOT/'browser-probe/harness/rtc-capability-fixture.mjs'
        code = 'import {rtcCapabilityPrelude} from '+json.dumps(fixture.as_uri())+";process.stdout.write(rtcCapabilityPrelude('observe'));"
        carrier_prelude = subprocess.check_output(['rtk', 'proxy', 'node', '--input-type=module', '-e', code], text=True)
        pages['host'] = HostBrowserPage(args.package, args.host_chrome, prelude=carrier_prelude)
        pages['pixel'] = (HostBrowserPage(args.package, args.host_chrome, prelude=carrier_prelude)
                          if host_pair else PairPage(device, args.package, prelude=carrier_prelude))
        identities, approvals, materials = {}, {}, {}
        for label in ('host', 'pixel'):
            stage(label+'-owned-page-ready', pages[label].start)
            if label == 'pixel' and not host_pair:
                pixel_observer = PixelCdpObserver(pages[label])
                raw('pixel-CDP-observer-start', pixel_observer.start())
            identities[label] = stage(label+'-independent-SDK-start',
                lambda label=label: sdk(label, 'start', {'configuration': config(ready)}))['id']
            require(identities[label] != ready['id'], 'Browser and provisioner identities must differ')
            require(stage(label+'-register-request', lambda label=label: sdk(label, 'request_admission',
                {'invitation': ready['invitation']})) == 'AwaitingApproval', 'initial request phase differs')
            accepted = actor.wait_event('admission-accepted'); raw(label+'-admission-accepted', accepted)
            report.setdefault('admissionWire', {})[label] = require_admission_wire(sdk(label, 'path_diagnostics'), accepted)
        require(identities['host'] != identities['pixel'], 'Browser identities must be independent')
        for label in ('host', 'pixel'):
            pending = command('admission-status', 'admission-status')
            requests = [entry for entry in pending['requests'] if entry['applicant'] == identities[label]]
            require(len(requests) == 1, 'stable independent applicant request missing')
            approvals[label] = stage(label+'-approve', lambda label=label: command('approve-request', 'approved',
                applicant=identities[label], request=requests[0]['request']))['authorization']
        # Both approved before delivery. Each PendingMaterials is completed immediately.
        for label in ('host', 'pixel'):
            require(stage(label+'-approved-authorization', lambda label=label: sdk(label, 'request_admission',
                {'invitation': ready['invitation']})) == 'PendingMaterials', 'approved material phase differs')
            require(stage(label+'-immediate-material-completion', lambda label=label: sdk(label, 'complete_admission'))
                == 'Complete', 'immediate durable material delivery failed')
            materials[label] = sdk(label, 'material_receipt'); require_complete(materials[label])
        for label, peer in (('host', 'pixel'), ('pixel', 'host')):
            if approvals[peer] not in materials[label]['records']:
                native_ice = json.loads(args.native_profile.read_text())
                if not native_ice.get('stun_servers') and not ready.get('permittedRelay'):
                    report['inputGap'] = {'actor': label, 'stage': 'peer-membership-history',
                        'check': 'formal-replica-needs-permitted-provider-data', 'sdkReplicaInvoked': False,
                        'providerStunConfigured': False, 'providerRelayPermitted': False,
                        'nativeProfileSha256': sha(args.native_profile)}
                    raw(label+'-missing-peer-history', {'missingPeerAuthorization': approvals[peer], 'provider': ready})
                    raise EvidenceCheckFailure('peer-history-replica-data-input-missing')
                before = materials[label]
                outcome = stage(label+'-formal-missing-peer-replica', lambda label=label: sdk(label, 'sync_replica', {'target': ready['id']}))
                materials[label] = sdk(label, 'material_receipt')
                report.setdefault('replica', {})[label] = require_replica(outcome, approvals[peer], before, materials[label])
            require(approvals[label] == materials[label]['authorization'] and approvals[peer] in materials[label]['records'],
                    'verified saved peer membership grant missing')
        report['independentIdentities'] = True
        report['bothVerifiedPeerMembershipGrantsSaved'] = True
        report['providerCleanupBeforeBusiness'] = stage('retire-provisioner-before-BB-business', lambda: close_native(actor))
        raw('provider-command-and-cleanup', actor.runs)
        raw('provider-operator-replies', native_replies)
        report['providerCommand'] = {'commandReceiptSha256': hashlib.sha256(json.dumps(actor.runs).encode()).hexdigest(),
            'commandExit': report['providerCleanupBeforeBusiness'].get('commandExit'),
            'sdkCleanupElapsedMs': report['providerCleanupBeforeBusiness'].get('sdkRetirement', {}).get('cleanupMs')}
        require(host_native_cleanup_succeeded(report['providerCleanupBeforeBusiness']), 'provisioner retirement failed')
        actor = None
        schedule = [('host', 'pixel', 'H')]
        if args.case == 'both':
            schedule.append(('pixel', 'host', 'R'))
        for dialer, answerer, name in schedule:
            facts = {'reopen': stage(dialer+'-dialer-only-reopen', lambda: reopen_dialer(dialer, answerer))}
            facts['handles'] = stage(dialer+'-to-'+answerer+'-common-connect-accept', lambda: establish(dialer, answerer, name))
            facts['business'] = stage(dialer+'-to-'+answerer+'-bidirectional-business', lambda: streams(dialer, answerer, name))
            report.setdefault('directions', []).append(facts)
            if args.case == 'cancel':
                baseline = {label: {'objects': sdk(label, 'rtc_objects'), 'sdk': sdk(label, 'rtc_diagnostics')}
                    for label in ('host', 'pixel')}
                sdk(answerer, 'hold_next_answer')
                sdk(answerer, 'begin_accept', {'name': 'C', 'expectedPeer': identities[dialer], 'callerMs': args.member_caller_ms})
                sdk(dialer, 'begin_connect', {'name': 'C', 'expectedPeer': identities[answerer], 'callerMs': args.member_caller_ms})
                stage('real-C-answer-created-and-held', lambda: wait_result(answerer, 'rtc_objects', {},
                    lambda value: (value.get('answerHold') or {}).get('answerCreated') is True))
                report['healthyDuringPendingC'] = stage('healthy-H-streams-during-C', lambda: streams(dialer, answerer, name))
                for label in ('host', 'pixel'):
                    require(sdk(label, 'attempt_status', {'name': 'C'})['settled'] is False,
                        'C already settled before explicit cancellation; cancellation isolation not observed')
                for label in ('host', 'pixel'):
                    sdk(label, 'cancel_attempt', {'name': 'C'})
                report['healthyAfterCancelBeforeLateAnswer'] = stage('healthy-H-after-C-cancel', lambda: streams(dialer, answerer, name))
                sdk(answerer, 'release_answer')
                for label in ('host', 'pixel'):
                    result = wait_result(label, 'attempt_status', {'name': 'C'}, lambda value: value['settled'])
                    require(result.get('cancelled') is True and result.get('returned') is False
                        and result.get('retirementErrors') == [], 'cancelled C returned/revived an ordinary handle')
                samples = []
                for delay in (0, 100):
                    if delay:
                        time.sleep(delay/1000)
                    sample = {'delayMs': delay, 'actors': {}}
                    for label in ('host', 'pixel'):
                        current = sdk(label, 'rtc_diagnostics')
                        objects = sdk(label, 'rtc_objects', {'after': baseline[label]['objects']['pcCreated']})
                        c_facts = cancelled_lease_facts(baseline[label]['sdk'], current, objects, late=delay == 100)
                        require(same_material(materials[label], sdk(label, 'material_receipt')), 'cancel changed saved materials')
                        sample['actors'][label] = {**c_facts, 'savedMaterialsUnchanged': True}
                    samples.append(sample)
                report['cancelIsolation'] = {'immediateAndLate': samples,
                    'healthyAfterLateAnswer': stage('healthy-H-after-real-late-answer', lambda: streams(dialer, answerer, name))}
            for label in (dialer, answerer):
                require(sdk(label, 'close_member', {'name': name}).get('closed') is True, 'owned member close/free failed')
        for label, peer in (('host', 'pixel'), ('pixel', 'host')):
            stage(label+'-post-story-persistence-reopen', lambda label=label, peer=peer: reopen_dialer(label, peer))
        report['publicMqttStoryQualified'] = True
        report['passed'] = True
    except Exception as error:
        actor_label = phase.split(':')[0] if phase.split(':')[0] in ('host', 'pixel', 'provider') else 'controller'
        first = remember_first_failure(report, error, phase, actor=actor_label)
        raw('first-failure', {'stage': phase, 'first': first, 'failure': str(error),
            'nativeEvents': current_native_events(actor), 'nativeStderr': current_native_stderr(actor),
            'nativeOperatorReplies': native_replies})
        for label, page in pages.items():
            if page.ready is not None:
                for name in named_attempts.get(label, []):
                    try:
                        raw(label+'-first-failure-attempt-'+name, page.call('attempt_status', {'name': name}))
                    except Exception as observation_error:
                        raw(label+'-attempt-observation-error', {'failureClass': type(observation_error).__name__,
                            'failure': str(observation_error)})
                for operation in ('status', 'path_diagnostics', 'rtc_diagnostics', 'rtc_objects'):
                    try:
                        raw(label+'-first-failure-'+operation, page.call(operation))
                    except Exception:
                        pass
    finally:
        report['cleanup'] = {}
        if pixel_observer is not None:
            raw('pixel-CDP-observation', pixel_observer.receipt())
            observer_cleanup = pixel_observer.close()
            report['cleanup']['pixelObserver'] = observer_cleanup
            good = (observer_cleanup.get('forced') is False and observer_cleanup.get('commandExit') == 0
                and observer_cleanup.get('readerEnded') is True and observer_cleanup.get('ownershipRefused') is False
                and observer_cleanup.get('processGroupAbsentImmediate') is True
                and observer_cleanup.get('processGroupAbsentLate') is True)
            report['passed'] &= good
            if not good:
                error = EvidenceCheckFailure('pixel-cdp-observer-retirement')
                first = remember_first_failure(report, error, 'final-cleanup', actor='pixel')
                raw('pixel-observer-cleanup-failure', {'first': first, 'failure': str(error)})
        for label, page in pages.items():
            cleanup_check = 'owned-page-shutdown'
            try:
                cleanup = page.close()
                raw(label+'-cleanup', cleanup)
                # Only selected public cleanup fields; raw SDK/history stays private.
                report['cleanup'][label] = {key: cleanup.get(key) for key in ('cleanupSucceeded',
                    'elapsedMs', 'localListenerClosed', 'forwardRequired', 'forwardRuleAbsent',
                    'forwardRemoveExit', 'reverseRequired', 'reverseRuleAbsent', 'reverseRemoveExit', 'ownedProfileRemoved')}
                report['cleanup'][label]['sdkCleanupElapsedMs'] = cleanup.get('sdk', {}).get('result', {}).get('cleanupElapsedMs')
                owned_host = isinstance(page, HostBrowserPage)
                report['cleanup'][label]['hostLifecycle'] = cleanup.get('tab') if owned_host else None
                if owned_host and page.process is not None:
                    command_evidence = {'command': page.process.args, 'exit': cleanup.get('tab', {}).get('commandExit')}
                    raw(label+'-sidecar-command', command_evidence)
                    report['cleanup'][label]['sidecarCommandReceiptSha256'] = hashlib.sha256(
                        json.dumps(command_evidence).encode()).hexdigest()
                cleanup_check = 'public-wss-handshake-witness'
                if owned_host and report['publicMqttStoryQualified']:
                    require(any(event.get('event') == 'handshake-response' and event.get('status') == 101
                        and event.get('origin') == wss_origin for event in cleanup.get('tab', {}).get('observations', [])),
                        'actual Host public MQTT/WSS normal-trust handshake observation missing')
                cleanup_check = 'sdk-resource-retirement'
                report['cleanup'][label]['sdkResources'] = sdk_cleanup_facts(cleanup.get('sdk', {}).get('result', {}))
                cleanup_check = 'caller-listener-rule-reclamation'
                resource_samples = []
                for delay in (0, 100):
                    if delay:
                        time.sleep(delay/1000)
                    sample = {'delayMs': delay, 'localListenerClosed': page.server.socket.fileno() == -1
                              and not page.thread.is_alive()}
                    if label == 'pixel' and not host_pair:
                        for kind, port in (('forward', page.forward), ('reverse', page.reverse)):
                            if port:
                                rules, code = device.run(kind, '--list', required=False)
                                sample[kind+'RuleAbsent'] = code == 0 and not any(
                                    port in line.split() for line in rules.splitlines())
                    resource_samples.append(sample)
                    require(all(value is True for key, value in sample.items() if key != 'delayMs'),
                            'immediate/late local listener or Pixel control rule remains')
                report['cleanup'][label]['callerResourceSamples'] = resource_samples
                cleanup_check = 'caller-cleanup-predicate'
                cleanup_ok = cleanup.get('cleanupSucceeded') is True and browser_cleanup_succeeded(cleanup)
                report['passed'] &= cleanup_ok
                if not cleanup_ok:
                    error = EvidenceCheckFailure(cleanup_check)
                    first = remember_first_failure(report, error, 'final-cleanup', actor=label)
                    raw(label+'-cleanup-predicate-failure', {'first': first, 'failure': str(error), 'observation': cleanup})
            except Exception as error:
                first = remember_first_failure(report, error, 'final-cleanup', actor=label,
                    check=getattr(error, 'check', cleanup_check))
                raw(label+'-cleanup-failure', {'first': first, 'check': cleanup_check, 'failure': str(error)})
                report['cleanup'].setdefault(label, {}).update(cleanupSucceeded=False, failureClass=type(error).__name__)
                report['passed'] = False
        if actor is not None:
            try:
                report['cleanup']['provider'] = close_native(actor)
                raw('provider-final-commands', actor.runs)
                cleanup_ok = host_native_cleanup_succeeded(report['cleanup']['provider'])
                report['passed'] &= cleanup_ok
                if not cleanup_ok:
                    error = EvidenceCheckFailure('provider-cleanup-predicate')
                    first = remember_first_failure(report, error, 'final-cleanup', actor='provider')
                    raw('provider-cleanup-predicate-failure', {'first': first, 'failure': str(error)})
            except Exception as error:
                report['passed'] = False
                first = remember_first_failure(report, error, 'final-cleanup', actor='provider', check='provider-retirement')
                raw('provider-cleanup-failure', {'first': first, 'failure': str(error)})
        if device is not None:
            raw('pixel-control-commands', device.commands)
            report['pixelCommands'] = {'privateReceiptSha256': hashlib.sha256(json.dumps(device.commands).encode()).hexdigest(),
                'count': len(device.commands), 'exits': [entry['exit'] for entry in device.commands]}
        report['sourceHashesAfter'] = source_freeze()
        report['controllerFrozenDuringRun'] = report['sourceHashesAfter'] == freeze
        report['passed'] &= report['controllerFrozenDuringRun']
        if not report['controllerFrozenDuringRun']:
            error = EvidenceCheckFailure('controller-source-stable')
            first = remember_first_failure(report, error, 'final-source-guard')
            raw('final-source-guard-failure', {'first': first, 'failure': str(error)})
        try:
            guard_build(args.wasm_build_receipt, args.wasm_build_sha256, args.package)
            guard_build(args.native_build_receipt, args.native_build_sha256)
            report['selectedSdkInputsAndArtifactsStableAfter'] = True
        except Exception as error:
            report['selectedSdkInputsAndArtifactsStableAfter'] = False
            report['passed'] = False
            first = remember_first_failure(report, error, 'final-input-guard', check='selected-sdk-input-artifact-stable')
            raw('final-input-guard-failure', {'first': first, 'failure': str(error)})
        report['exit'] = 0 if report['passed'] else 1
        save()
    return report['exit']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify-inputs', action='store_true', help='read-only guards; no actors/browser/device/network')
    parser.add_argument('--wasm-build-receipt', type=Path, default=WASM_RECEIPT)
    parser.add_argument('--wasm-build-sha256', default=WASM_PIN)
    parser.add_argument('--package', type=Path, default=WASM_RECEIPT.parent/'pkg')
    parser.add_argument('--native-build-receipt', type=Path, required=True)
    parser.add_argument('--native-build-sha256', required=True)
    parser.add_argument('--facility', type=Path)
    parser.add_argument('--browser-profile', type=Path)
    parser.add_argument('--native-profile', type=Path)
    parser.add_argument('--host-chrome', type=Path, default=HOST_CHROME)
    parser.add_argument('--pixel-transport', choices=('usb', 'network'), default='usb')
    parser.add_argument('--actors', choices=('host-pixel', 'host-host'), default='host-pixel',
                        help='actual endpoints; host-host uses two independent owned Host profiles/stores')
    parser.add_argument('--case', choices=('forward', 'both', 'cancel'), default='forward')
    parser.add_argument('--member-caller-ms', type=int, choices=(12000, 60000), default=12000,
                        help='one explicit member connect/accept caller; existing 12000ms cases remain unchanged')
    parser.add_argument('--host-warp', choices=('active', 'inactive', 'unknown'), default='unknown')
    parser.add_argument('--pixel-overlay', choices=('active', 'inactive', 'unknown'), default='unknown')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.verify_inputs:
        print(json.dumps({'sdkRun': False, 'deviceOperation': False, 'publicNetworkOperation': False,
            'browser': guard_build(args.wasm_build_receipt, args.wasm_build_sha256, args.package),
            'provisioner': guard_build(args.native_build_receipt, args.native_build_sha256),
            'controllerSourceHashes': source_freeze()}, indent=2))
        return 0
    require(all(getattr(args, name) for name in ('facility', 'browser_profile', 'native_profile', 'output')),
            'explicit permitted facility/profile files and new output are required')
    existed_before = args.output.exists()
    try:
        return run(args)
    except Exception as error:
        # Failed source/artifact preconditions precede actors. Preserve their
        # exact command and private cause in a new output without touching an
        # older case. Also retain a rare outer receipt-finalization failure.
        if not existed_before:
            args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
            receipt_path = args.output/'receipt.json'
            report = json.loads(receipt_path.read_text()) if receipt_path.is_file() else {
                'scope': 'M75 runtime precondition failure; no qualification',
                'command': ['rtk', 'proxy', 'python3', str(Path(__file__).resolve()), *sys.argv[1:]],
                'passed': False, 'physicalWanQualified': False, 'publicMqttStoryQualified': False}
            first = remember_first_failure(report, error, 'outer-precondition-or-finalization',
                                           check='runtime-precondition-or-finalization')
            private_path = args.output/'private-sdk.jsonl'
            if not private_path.exists():
                private_path.touch(mode=0o600)
            with private_path.open('a') as stream:
                stream.write(json.dumps({'label': 'outer-failure', 'value': {
                    'first': first, 'failure': str(error)}})+'\n')
            report['passed'] = False
            report['exit'] = 1
            report['outerInputHashObservations'] = {name: sha(path) if path and path.is_file() else None
                for name in ('native_build_receipt', 'wasm_build_receipt', 'facility', 'browser_profile', 'native_profile')
                for path in [getattr(args, name)]}
            receipt_path.write_text(json.dumps(report, indent=2)+'\n')
        raise


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as error:
        # Precondition failure occurs before runtime: never print private input text.
        print(json.dumps({'exit': 1, 'preconditionFailureClass': type(error).__name__}), file=sys.stderr)
        sys.exit(1)
