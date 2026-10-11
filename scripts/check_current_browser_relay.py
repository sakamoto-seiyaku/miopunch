#!/usr/bin/env python3
"""One current M76 SDK capability/no-relay/restoration story; no implicit execution.

Only --execute touches an authorized device or the explicit EMQX/APS1 facilities.
The shared BrowserPage/HostNativeActor own actual SDK RPC, storage and processes.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / 'browser-probe/harness/mio76-relay-capability-page.mjs'
RELAY = 'https://aps1-1.relay.n0.iroh.link./'
MQTT_WSS = 'wss://broker.emqx.io:8084/mqtt'
ALPN = 'miopunch/session/member/1'
MODES = {'missing-answer': 'api-missing', 'pc-denied': 'peer-connection-rejected',
         'dc-denied': 'data-channel-rejected'}
RUNTIME_FILES = [Path(__file__).resolve(), PAGE,
    ROOT/'browser-probe/harness/rtc-capability-fixture.mjs',
    ROOT/'browser-probe/harness/android-pair-page.js',
    ROOT/'scripts/android_pair_runtime.py', ROOT/'scripts/host_native_runtime.py',
    ROOT/'scripts/check_unified_admission.py', ROOT/'scripts/check_unified_browser_native.py',
    ROOT/'scripts/check_browser_relay_capability.py', ROOT/'scripts/wan_environment_preflight.py',
    ROOT/'browser-probe/harness/relay-cdp-observer.mjs',
    ROOT/'browser-probe/harness/package.json', ROOT/'browser-probe/harness/package-lock.json']


class EvidenceFailure(RuntimeError):
    """A fixed controller check name, never a raw SDK error."""


def require(condition, check):
    if not condition:
        raise EvidenceFailure(check)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def exact_zero(value):
    return type(value) is int and value == 0


def finite_ms(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def private_write(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'w') as handle:
        json.dump(value, handle, indent=2); handle.write('\n')
    return {'privateFile': path.name, 'sha256': sha(path), 'mode': '0600'}


class PrivateRpcs:
    """Capture the exact first-returned DTO/exception before any later SDK getter."""
    def __init__(self, output):
        self.output, self.ordinal = output, 0

    def record(self, operation, *, reply=None, error=None, arguments=None):
        self.ordinal += 1
        value = {'ordinal': self.ordinal, 'operation': operation,
                 'arguments': arguments, 'reply': reply}
        if error is not None:
            value['exception'] = {'class': type(error).__name__, 'text': str(error)}
        return private_write(self.output/f'private-rpc-{self.ordinal:04d}.json', value)


def runtime_hashes():
    return {str(path.relative_to(ROOT)): sha(path) for path in RUNTIME_FILES}


def guard_build(receipt, pin, *, binary=None, binary_pin=None, package=None, package_pin=None):
    # Reuse the M83 actual-input guard; add exact counts/retained depfiles and
    # explicit artifact pins. Broad historical captures are not current inputs.
    from check_unified_admission import validate_sdk_build_sources
    require(sha(receipt) == pin, 'build-receipt-pin')
    build = json.loads(receipt.read_text())
    require(exact_zero(build.get('exit')) and build.get('qualifiedForFrozenRun') is True
        and build.get('sourceFrozenDuringBuild') is True, 'successful-frozen-build')
    binding = validate_sdk_build_sources(receipt)
    selected = json.loads((receipt.parent/'selected-dependency-inputs.json').read_text())
    for field, values in [('actualSelectedInputCount', selected.get('inputs')),
            ('actualSelectedRepositoryInputCount', selected.get('copiedRepositoryInputs')),
            ('actualDepfileCount', selected.get('depfiles'))]:
        require(isinstance(values, dict) and bool(values) and type(build.get(field)) is int
            and len(values) == build[field], 'actual-compiler-counts')
    for name, digest in selected['depfiles'].items():
        source = Path(name)
        require(source.is_relative_to(ROOT/'target'), 'captured-depfile-scope')
        captured = receipt.parent/'depfiles'/source.relative_to(ROOT/'target')
        require(captured.is_file() and sha(captured) == digest, 'captured-depfile-pin')
    if binary is not None:
        require(build.get('nativePlatform') == 'host', 'native-host-artifact')
        deployed = build['deployment']['unified_admission_story']
        require(Path(deployed['deployedPath']).resolve() == binary.resolve()
            and sha(binary) == binary_pin == deployed['deployedSha256']
            and sha(deployed['compiledPath']) == deployed['compiledSha256']
            and exact_zero(deployed['transformation'].get('exit')), 'native-artifact-chain')
    else:
        artifacts = build['artifacts']
        actual = {str(path.relative_to(package)): sha(path) for path in package.rglob('*')
                  if path.is_file() and path.suffix in {'.js', '.wasm', '.ts'}}
        catalog_pin = hashlib.sha256(json.dumps(actual, sort_keys=True).encode()).hexdigest()
        require(actual == artifacts and catalog_pin == package_pin, 'complete-browser-package-pin')
    return {'receiptSha256': pin, 'actualSelectedInputCount': binding['actualSelectedInputCount'],
        'selectedRepositoryCopies': len(selected['copiedRepositoryInputs']),
        'capturedDepfileCount': len(selected['depfiles']), 'allSelectedInputsMatch': True,
        'capturedBroadSourcesEqualCurrent': binding['capturedBroadSourcesEqualCurrent'],
        'unselectedSourceDeltaNames': sorted(binding['unselectedRuntimeControllerDeltas']),
        'inactiveSdkExceptions': []}


def native_caller_budget_facts(ready, reopened, released):
    """Keep the Native fixture's saved caller policy across reopen and acceptance."""
    budget = ready.get('callerBudgetMs')
    require(type(budget) is int and budget > 0, 'native-configured-caller-budget-known')
    require(type(reopened.get('callerBudgetMs')) is int and reopened['callerBudgetMs'] == budget,
            'native-reopened-caller-budget-preserved')
    require(type(released.get('callerBudgetMs')) is int and released['callerBudgetMs'] == budget,
            'native-member-caller-budget-preserved')
    return {'callerBudgetMs': budget, 'sameSavedPolicyAfterReopen': True,
            'samePolicyAtMemberAcceptance': True}


def websocket_retirement_facts(immediate, late):
    """Keep a quiescent Relay terminal-close tail distinct from retired callbacks."""
    relay = urlsplit(RELAY)
    relay_origin = ('wss' if relay.scheme == 'https' else 'ws', relay.hostname,
                    relay.port or (443 if relay.scheme == 'https' else 80))
    mqtt = urlsplit(MQTT_WSS)
    mqtt_origin = (mqtt.scheme, mqtt.hostname, mqtt.port or 443)
    published = []
    previous_sockets = None
    for sample in (immediate, late):
        sockets, witnesses = sample.get('sockets'), sample.get('webSocketCloseWitnesses')
        require(isinstance(sockets, list) and isinstance(witnesses, list)
            and bool(sockets) and len(sockets) == len(witnesses), 'real-websocket-witness-set')
        if previous_sockets is not None:
            require(len(previous_sockets) == len(sockets), 'same-websocket-retirement-count')
        states, callbacks, pending_relay_close, socket_facts = [], 0, 0, []
        for index, (socket, witness) in enumerate(zip(sockets, witnesses)):
            state = socket.get('readyState')
            require(type(state) is int and state in (2, 3), 'no-active-websocket')
            require(type(witness.get('socketIndex')) is int and witness['socketIndex'] == index
                and type(witness.get('readyState')) is int and witness['readyState'] == state,
                'same-websocket-close-witness')
            require(type(witness.get('closeCalls')) is int and witness['closeCalls'] > 0
                and exact_zero(witness.get('closeFailures')) and finite_ms(witness.get('firstCloseAtMs'))
                and type(witness.get('stateBeforeFirstClose')) is int
                and witness['stateBeforeFirstClose'] in (0, 1, 2, 3)
                and type(witness.get('stateAfterFirstClose')) is int
                and witness['stateAfterFirstClose'] in (2, 3), 'real-websocket-close-initiated')
            require(exact_zero(witness.get('bufferedBytes'))
                and witness.get('callbackTrackingComplete') is True
                and exact_zero(witness.get('sdkRegisteredListeners')), 'websocket-buffer-callbacks-quiescent')
            callback_count = witness.get('sdkPropertyCallbacks')
            require(type(callback_count) is int and 0 <= callback_count <= 4,
                    'websocket-callback-count-known')
            require(type(socket.get('origin')) is str, 'websocket-origin-known')
            try:
                origin = urlsplit(socket['origin'])
                port = origin.port
                actual_origin = (origin.scheme, origin.hostname,
                    port if port is not None else (443 if origin.scheme == 'wss' else 80))
            except (TypeError, ValueError):
                raise EvidenceFailure('websocket-origin-known') from None
            require(actual_origin in (mqtt_origin, relay_origin)
                and origin.path in ('', '/') and not origin.query and not origin.fragment
                and origin.username is None and origin.password is None
                and finite_ms(socket.get('createdAtMs')), 'only-configured-websocket-resource')
            if callback_count:
                # The pinned Relay WsStream::poll_close awaits its real Closed
                # event; dropping these callbacks would strand that future.
                require(state == 2 and actual_origin == relay_origin and callback_count == 4,
                        'only-configured-relay-terminal-close-callbacks')
                pending_relay_close += 1
            callbacks += callback_count
            socket_facts.append({'socketIndex': index,
                'resource': 'configured-relay' if actual_origin == relay_origin else 'configured-mqtt',
                'readyState': state, 'sdkCallbacks': callback_count,
                'terminalClosePending': state == 2 and callback_count == 4,
                'bufferedBytes': 0, 'registeredListeners': 0})
            for name in ('sentBytes', 'receivedBytes', 'sentFrames', 'receivedFrames'):
                require(type(socket.get(name)) is int and socket[name] >= 0, 'websocket-traffic-count-known')
                if previous_sockets is not None:
                    require(socket[name] == previous_sockets[index][name], 'no-post-retirement-websocket-traffic')
            if previous_sockets is not None:
                require(socket.get('createdAtMs') == previous_sockets[index].get('createdAtMs')
                    and socket.get('origin') == previous_sockets[index].get('origin'),
                    'same-cumulative-websocket-sample')
                require(state >= previous_sockets[index]['readyState'], 'websocket-close-state-not-regressed')
                require(callback_count <= immediate['webSocketCloseWitnesses'][index]['sdkPropertyCallbacks'],
                        'no-new-post-close-websocket-callbacks')
            states.append(state)
        require(type(sample.get('openWebSockets')) is int
            and sample['openWebSockets'] == states.count(2), 'actual-nonclosed-websocket-count')
        published.append({'activeWebSockets': 0, 'closingWebSockets': states.count(2),
            'closedWebSockets': states.count(3), 'allClosedObserved': states.count(2) == 0,
            'bufferedBytes': 0, 'sdkCallbacks': callbacks, 'realCloseInitiated': True,
            'pendingRelayCloseObserved': pending_relay_close > 0,
            'pendingRelayCloseSocketCount': pending_relay_close,
            'gracefulRelayCloseObserved': False if pending_relay_close else None,
            'relayCloseTaskState': 'terminal-event-pending' if pending_relay_close else 'not-observed',
            'relayTrafficLoopState': 'not-observed', 'relayReceiveQueuePackets': None,
            'allSdkTerminalTasksRetiredProven': False,
            'sockets': socket_facts,
            'socketCount': len(states)})
        previous_sockets = sockets
    require(len(immediate['sockets']) == len(late['sockets']), 'same-websocket-retirement-count')
    return {'immediate': published[0], 'late': published[1], 'trafficUnchangedAt100ms': True}


def fixture_facts(value, mode, generations):
    require(value.get('cumulative') is True and exact_zero(value.get('resetAttempts')),
            'cumulative-observer-never-reset')
    require(value.get('injection', {}).get('mode') == mode, 'owned-injection-mode')
    if mode == 'missing-answer':
        require(value['injection'].get('necessaryAnswerMissing') is True, 'real-answer-api-absent')
    expected = generations if mode == 'dc-denied' else 0
    for name in ('peerCreated', 'peerCloseCalls'):
        require(type(value.get(name)) is int and value[name] == expected, 'cumulative-peer-count')
    for name in ('channelCreated', 'channelCloseCalls', 'openPeerConnections', 'openDataChannels'):
        require(exact_zero(value.get(name)), 'no-owned-rtc-object')
    require(type(value.get('openWebSockets')) is int and value['openWebSockets'] >= 0,
            'websocket-count-known')
    return {name: value[name] for name in ('cumulative', 'resetAttempts', 'peerCreated',
        'peerCloseCalls', 'channelCreated', 'channelCloseCalls', 'openPeerConnections',
        'openDataChannels', 'openWebSockets')}


def retirement_facts(value, mode, generations, *, final=False):
    require(value.get('cleanupSucceeded') is True and value.get('networkClosed') is True
        and value.get('networkFreed') is True and exact_zero(value.get('ownedMemberHandles'))
        and value.get('errors') == [], 'sdk-close-free-retirement')
    for name in ('ownedMemberCount', 'memberCloseCount', 'memberFreeCount'):
        require(type(value.get(name)) is int and value[name] >= 0, 'owned-member-count-known')
    require(value['memberCloseCount'] == value['memberFreeCount'] == value['ownedMemberCount'],
            'owned-members-closed-freed')
    snapshots = value.get('networkDiagnostics')
    require(isinstance(snapshots, list) and [item.get('delayMs') for item in snapshots] == [0, 100]
        and all(type(item.get('delayMs')) is int for item in snapshots), 'sdk-immediate-late-observations')
    for item in snapshots:
        require(all(exact_zero(item.get('observed', {}).get(name))
            for name in ('activeAssociations', 'activePumpTasks')), 'sdk-rtc-pumps-retired')
    objects = value.get('mio76Objects', {})
    require(objects.get('afterSdkCloseFree') is True and objects.get('finalShutdown') is final
        and objects.get('requestedLateDelayMs') == 100 and finite_ms(objects.get('observedLateDelayMs'))
        and objects['observedLateDelayMs'] >= 100, 'real-object-late-observation')
    immediate = fixture_facts(objects.get('immediate', {}), mode, generations)
    late = fixture_facts(objects.get('late', {}), mode, generations)
    websockets = websocket_retirement_facts(objects['immediate'], objects['late'])
    sdk_probe = value.get('mio76SdkRetirement', {})
    sdk_record = sdk_probe.get('record') or {}
    require(sdk_probe.get('installed') is True and sdk_probe.get('observationFailureClass') is None
        and type(sdk_record.get('index')) is int and sdk_record['index'] == generations
        and sdk_record.get('closeReturned') is True and sdk_record.get('freeReturned') is True
        and sdk_record.get('freeMatchedCloseNetwork') is True
        and sdk_record.get('queueGetterSucceeded') is True, 'actual-sdk-close-free-queue-witness')
    carriers = sdk_record.get('carriers')
    require(isinstance(carriers, list) and len(carriers) == 1
        and all(carrier.get('ready') is False and carrier.get('closed') is True
            and all(exact_zero(carrier.get(name)) for name in ('queuedPackets', 'pendingBytes'))
            and type(carrier.get('bufferedBytes')) is int and carrier['bufferedBytes'] >= 0
                for carrier in carriers), 'actual-sdk-carrier-queues-retired')
    require(finite_ms(value.get('cleanupElapsedMs')), 'sdk-cleanup-timing-known')
    return {'networkClosed': True, 'networkFreed': True, 'ownedMemberHandles': 0,
        'activeAssociationsImmediateAndLate': 0, 'activePumpsImmediateAndLate': 0,
        'objectsImmediate': immediate, 'objectsLate': late,
        'websocketRetirement': websockets, 'sdkCarrierQueuesRetired': True,
        'ownershipCleanupScope': 'member handles, MQTT queues/callbacks and RTC objects; Relay terminal close measured separately',
        'carrierTransmitBufferedBytesAfterCloseBeforeFree': [carrier['bufferedBytes'] for carrier in carriers],
        'objectLateDelayMs': objects['observedLateDelayMs'], 'finalShutdown': final,
        'sdkCleanupElapsedMs': value['cleanupElapsedMs']}


def carrier_facts(value):
    carriers = value.get('carriers')
    require(isinstance(carriers, list) and len(carriers) == 1
        and carriers[0].get('protocol') == 'mqtt' and carriers[0].get('ready') is True
        and carriers[0].get('closed') is False, 'actual-mqtt-contact-ready')
    traffic = carriers[0].get('traffic', {})
    require(exact_zero(traffic.get('sent_bytes')) and exact_zero(traffic.get('received_bytes')),
            'no-quic-business-bytes-on-mqtt')
    return {'mqttContactReady': True, 'mqttQuicBytesSent': 0, 'mqttQuicBytesReceived': 0,
            'ircConfigured': False}


def no_path_class(reply):
    # These exact literals are the current ConnectError/control_error variants;
    # arbitrary timeout/authorization errors cannot masquerade as no-data.
    failure = reply.get('failure')
    accepted = {'no currently permitted data path', 'Error: no currently permitted data path',
                'unified/DataPathUnavailable', 'Error: unified/DataPathUnavailable'}
    require(reply.get('ok') is False and failure in accepted, 'typed-data-path-unavailable')
    return 'DataPathUnavailable'


def common_facts(value, role, relay):
    require(value.get('event') == 'unified-path-ready' and type(value.get('version')) is int
        and value['version'] == 1 and type(value.get('purposeCode')) is int
        and value['purposeCode'] == 1 and value.get('purpose') == 'member-business'
        and value.get('role') == role and value.get('rtcRole') == 'None'
        and value.get('freshAck') is True and value.get('stableUnchanged') is True
        and value.get('bindingUnchanged') is True and value.get('selected') is True
        and value.get('selectedRoute') == 'relay:'+relay, 'strict-original-m-common-proof')
    require(type(value.get('stableId')) is int and value['stableId'] >= 0, 'actual-original-m-known')
    runtime, peer_runtime = ('Browser', 'Native') if role == 'initiator' else ('Native', 'Browser')
    require(value.get('runtime') == runtime and value.get('peerRuntime') == peer_runtime,
            'actual-common-runtime-roles')
    for name in ('supported', 'permitted', 'peerSupported', 'peerPermitted'):
        require(type(value.get(name)) is int and value[name] >= 0 and value[name] & 8 == 0,
                'rtc-not-advertised')
    require(value['permitted'] == value['peerPermitted'] == 4, 'relay-only-mutual-permission')
    return {name: value[name] for name in ('version', 'purposeCode', 'role', 'rtcRole',
        'freshAck', 'stableUnchanged', 'bindingUnchanged', 'selected', 'supported',
        'permitted', 'peerSupported', 'peerPermitted')}


def native_retirement_facts(value, *, directory_required=True):
    from host_native_runtime import host_native_cleanup_succeeded
    require(host_native_cleanup_succeeded(value, directory_required), 'owned-host-retirement')
    require(exact_zero(value.get('commandExit')), 'native-natural-exit-exact-zero')
    require(finite_ms(value.get('elapsedMs')) and type(value.get('lateDelayMs')) is int
        and value['lateDelayMs'] == 100, 'native-independent-cleanup-timing')
    sdk = value.get('sdkRetirement', {})
    require(sdk.get('nodeCloseReturned') is True and sdk.get('handlersDrained') is True
        and exact_zero(sdk.get('abortedHandlers')) and exact_zero(sdk.get('activeRtcAssociations'))
        and exact_zero(sdk.get('activeRtcPumps')), 'native-sdk-natural-retirement')
    for key in ('ownedProcessGroupImmediate', 'ownedProcessGroupLate'):
        require(value.get(key, {}).get('observed') is True
            and exact_zero(value[key].get('processCount')) and exact_zero(value[key].get('socketCount')),
            'owned-host-group-sockets-retired')
    for phase in ('immediate', 'late'):
        sample = sdk.get(phase, {})
        require(exact_zero(sample.get('activeRtcAssociations')) and exact_zero(sample.get('activeRtcPumps'))
            and exact_zero(sample.get('traffic', {}).get('driver_active'))
            and exact_zero(sample.get('traffic', {}).get('queued_packets')), 'native-pumps-queues-retired')
    require(all(type(sdk.get(name)) is int and sdk[name] >= 0
        for name in ('socketsBefore', 'socketsImmediate', 'socketsLate'))
        and sdk['socketsImmediate'] <= sdk['socketsBefore'] and sdk['socketsLate'] <= sdk['socketsBefore'],
        'native-sdk-sockets-baseline')
    return {name: value.get(name) for name in ('forced', 'commandExit', 'ownedHostProcessAbsent',
        'ownedHostProcessAbsentLate', 'ownedProcessGroupAbsentImmediately', 'ownedProcessGroupAbsentLate',
        'localReadersEnded', 'localPipesClosed', 'ownedDirectoryRemoved', 'elapsedMs')} | {
        'ownedProcessSocketsImmediate': 0, 'ownedProcessSocketsLate': 0,
        'sdkSocketsBefore': sdk['socketsBefore'], 'sdkSocketsImmediate': sdk['socketsImmediate'],
        'sdkSocketsLate': sdk['socketsLate'], 'activeRtcAssociations': 0,
        'activeRtcPumps': 0, 'mqttDriverAndQueuedPacketsLate': 0}


def page_prelude(mode):
    script = 'import {relayCapabilityPrelude} from '+json.dumps(PAGE.as_uri())+';'
    script += 'process.stdout.write(relayCapabilityPrelude(process.argv[1]));'
    return subprocess.run(['rtk', 'proxy', 'node', '--input-type=module', '-e', script, mode],
                          check=True, capture_output=True, text=True).stdout


def run(args):
    from android_pair_runtime import BrowserPage, Device, browser_cleanup_succeeded
    from host_native_runtime import HostNativeActor
    from check_unified_browser_native import close_native
    from check_browser_relay_capability import OwnedCdpObserver
    from check_unified_admission import (native_command, same_grant, require_complete,
        require_admission_wire, persist_command_evidence, persist_browser_start_observation)
    require(not args.output.exists(), 'new-output-required')
    args.output.mkdir(mode=0o700, parents=True)
    report = {'ticket': 'MIOPUNCH-76', 'mode': args.mode, 'passed': False, 'commandExit': None,
        'scope': 'current Host Native / Pixel Browser actual SDK capability and permitted relay member story',
        'physicalWanQualified': False, 'callerBudgetMs': 60000, 'nativeRelayCeilingMs': 6000,
        'browserCallerPolicy': 'explicit 60000ms; current SDK default, no per-stage reset',
        'command': ['rtk', 'proxy', 'python3', str(Path(__file__).resolve()), *sys.argv[1:]],
        'startedUtc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'stages': [], 'sdkCalls': [], 'generationCleanup': [], 'nativeGenerationCleanup': []}
    actor = page = observer = device = None
    phase, generation, body_passed = 'source-artifact-preconditions', 0, False
    failed_start = False
    native_replies = []
    private = PrivateRpcs(args.output)
    frozen = runtime_hashes()
    report['runtimeSources'] = frozen

    def save():
        (args.output/'receipt.json').write_text(json.dumps(report, indent=2)+'\n')

    def fail(error, at):
        report.setdefault('firstFailure', {'stage': at, 'check': str(error)
            if isinstance(error, EvidenceFailure) else 'operation-failed',
            'failureClass': type(error).__name__})
        save()

    def stage(name, operation):
        nonlocal phase
        phase = name; started = time.monotonic()
        try:
            value = operation()
            report['stages'].append({'name': name, 'ok': True,
                                    'callerMs': round((time.monotonic()-started)*1000, 3)})
            save(); return value
        except Exception as error:
            fail(error, name)
            report['stages'].append({'name': name, 'ok': False,
                'callerMs': round((time.monotonic()-started)*1000, 3), 'failureClass': type(error).__name__})
            save(); raise

    def rpc(operation, values=None, *, expected_ok=True):
        nonlocal failed_start
        try:
            reply = page.call(operation, values)
        except Exception as error:
            facts = private.record(operation, error=error, arguments=values)
            report['sdkCalls'].append({'operation': operation, 'generation': generation,
                                       'ok': False, **facts})
            failed_start = operation == 'start'
            fail(error, phase)
            raise
        # This exclusive write precedes checked(), diagnostic getters and cleanup.
        facts = private.record(operation, reply=reply, arguments=values)
        report['sdkCalls'].append({'operation': operation, 'generation': generation,
            'ok': reply.get('ok'), 'callerElapsedMs': reply.get('callerElapsedMs'), **facts})
        save()
        if reply.get('ok') is not expected_ok:
            failed_start = operation == 'start'
        require(reply.get('ok') is expected_ok, 'formal-sdk-rpc-'+operation)
        return reply['result'] if expected_ok else reply

    def command(operation, expected, **values):
        before = len(native_replies)
        try:
            return native_command(actor, operation, expected, native_replies, **values)
        finally:
            if len(native_replies) > before:
                report.setdefault('nativeCommands', []).append(private.record('native:'+operation,
                    reply=native_replies[-1]))
                save()

    def start_browser(ready, *, data):
        nonlocal generation
        config = {'relays': [RELAY] if data else [], 'disabled': [], 'irc': [],
            'mqtt': [facility], 'rtc_enabled': True, 'rtc_ice': browser_profile,
            'maintenance': {'contact_timeout_ms': report['callerBudgetMs']},
            'contact_routes': [{'contact': ready['id'], 'facility': facility['logical_id'],
                                'address': ready['route']}]}
        result = rpc('start', {'configuration': config})
        generation += 1
        return result

    def capability(checkpoint):
        diagnostics = rpc('path_diagnostics')
        cap = diagnostics.get('rtcCapability', {})
        require(cap.get('supported') is False and cap.get('permitted') is True
            and cap.get('effective') is False and cap.get('reason') == MODES[args.mode],
            'runtime-unavailable-permission-retained')
        snapshot = rpc('rtc_fixture_snapshot')
        require(snapshot.get('sdkCloseObserverInstalled') is True, 'actual-sdk-retirement-observer-installed')
        objects = fixture_facts(snapshot, args.mode, generation)
        report.setdefault('capabilityObservations', []).append({'checkpoint': checkpoint,
            'supported': False, 'permitted': True, 'effective': False, 'reason': MODES[args.mode],
            'objects': objects})
        return diagnostics

    def stop_browser(checkpoint):
        value = rpc('stop')
        facts = retirement_facts(value, args.mode, generation)
        require(type(value.get('generation')) is int and value['generation'] == generation,
                'sdk-generation-retirement-matched')
        report['generationCleanup'].append({'checkpoint': checkpoint, 'generation': generation, **facts})
        save()
        return value

    def snapshot_commands(checkpoint, error=None):
        report.setdefault('executionEvidence', []).append(persist_command_evidence(args.output,
            {'browser': device}, actor, checkpoint, stage_error=error, phase=phase,
            command_replies=native_replies))
        save()

    try:
        report['buildBindings'] = {
            'native': guard_build(args.native_build_receipt, args.native_build_receipt_sha256,
                binary=args.native_binary, binary_pin=args.native_binary_sha256),
            'browser': guard_build(args.browser_build_receipt, args.browser_build_receipt_sha256,
                package=args.package, package_pin=args.package_sha256)}
        raw_profiles = {name: getattr(args, name).read_bytes()
            for name in ('facility', 'native_profile', 'browser_profile')}
        facility = json.loads(raw_profiles['facility'])
        native_profile = json.loads(raw_profiles['native_profile'])
        browser_profile = json.loads(raw_profiles['browser_profile'])
        require(facility.get('native_url') == 'mqtts://broker.emqx.io:8883/'
            and facility.get('wss_url') == MQTT_WSS, 'original-normal-emqx-facility')
        require(all(profile == {'stun_servers': [], 'local_bind_addresses': [], 'candidate_addresses': []}
            for profile in (native_profile, browser_profile)), 'existing-empty-ice-profiles')
        report['selectedProfiles'] = {name: hashlib.sha256(raw).hexdigest()
                                      for name, raw in raw_profiles.items()}
        prelude = page_prelude(args.mode)
        report['preludeSha256'] = hashlib.sha256(prelude.encode()).hexdigest()
        actor = stage('owned-host-native', HostNativeActor)
        actor.deploy(args.native_binary, 'unified-admission-story', executable=True)
        facility_remote = actor.deploy(args.facility, 'facility.json')
        profile_remote = actor.deploy(args.native_profile, 'ice-profile.json')
        require(sha(facility_remote) == report['selectedProfiles']['facility']
            and sha(profile_remote) == report['selectedProfiles']['native_profile'],
            'actual-deployed-configuration-pins')
        native_args = [actor.directory+'/saved-provider', facility_remote, profile_remote,
                       '--allow-unqualified-preparation', '--relay', RELAY, '--rtc-disabled']
        actor.start(native_args)
        ready = stage('native-start', lambda: actor.wait_event('ready'))
        require(ready.get('normalPublicTrust') is True and ready.get('rtcEnabled') is False
            and ready.get('permittedRelay') == RELAY, 'native-relay-only-public-trust')
        native_id = ready['id']
        device = stage('pixel-device-selection', lambda: Device('Pixel_6a', 'usb'))
        page = BrowserPage(device, args.package, prelude=prelude, environment_rtc_probe=False)
        stage('owned-browser-page', page.start)
        observer = OwnedCdpObserver(page)
        stage('owned-browser-observer', observer.start)
        started = stage('independent-browser-start', lambda: start_browser(ready, data=False))
        browser_id = started['id']
        require(browser_id != native_id, 'independent-real-sdk-identities')
        stage('admission-runtime-capability', lambda: capability('admission'))
        require(stage('actual-admission-request', lambda: rpc('request_admission',
            {'invitation': ready['invitation']})) == 'AwaitingApproval', 'independent-manual-request')
        accepted = stage('native-actual-admission', lambda: actor.wait_event('admission-accepted'))
        report['admissionWire'] = require_admission_wire(rpc('path_diagnostics'), accepted)
        pending = command('admission-status', 'admission-status')
        requests = [request for request in pending['requests'] if request['applicant'] == browser_id]
        require(len(requests) == 1, 'exact-independent-pending-request')
        approved = stage('one-real-sdk-approval', lambda: command('approve-request', 'approved',
            applicant=browser_id, request=requests[0]['request']))
        require(stage('approved-material-delivery', lambda: rpc('request_admission',
            {'invitation': ready['invitation']})) == 'PendingMaterials', 'approved-materials-delivered')
        require(stage('immediate-material-complete', lambda: rpc('complete_admission')) == 'Complete',
                'actual-material-completion')
        material = rpc('material_receipt')
        report['completeMaterials'] = require_complete(material)
        require(command('admission-status', 'admission-status')['appliedCount'] == approved['after'],
                'once-only-approval')
        stage('complete-browser-retire', lambda: stop_browser('after-admission'))
        retired = stage('provider-retire-before-reopen', lambda: close_native(actor, remove_directory=False))
        entry = {'generation': 1, 'qualified': False,
            **private_write(args.output/'private-native-generation-1-cleanup.json', retired)}
        report['nativeGenerationCleanup'].append(entry)
        save()
        entry.update(native_retirement_facts(retired, directory_required=False), qualified=True)
        snapshot_commands('before-native-reopen')
        actor.start(native_args)
        reopened = stage('same-provider-reopen', lambda: actor.wait_event('ready'))
        require(reopened['id'] == native_id, 'native-saved-identity-retained')
        require(command('admission-status', 'admission-status')['appliedCount'] == approved['after'],
                'native-durable-once-only-approval')
        if args.mode == 'missing-answer':
            restarted = stage('same-browser-no-relay-reopen', lambda: start_browser(reopened, data=False))
            require(restarted['id'] == browser_id, 'browser-saved-identity-retained')
            reopened_material = rpc('material_receipt')
            require_complete(reopened_material)
            require(same_grant(material, reopened_material), 'same-durable-member-grant')
            before = stage('no-relay-runtime-capability', lambda: capability('no-relay'))
            report['noDataBefore'] = carrier_facts(before)
            unavailable = stage('actual-no-data-member-attempt', lambda: rpc('open_member',
                {'target': native_id}, expected_ok=False))
            require(finite_ms(unavailable.get('callerElapsedMs')), 'no-data-caller-timing-known')
            report['noData'] = {'errorClass': no_path_class(unavailable), 'ordinaryHandleReturned': False,
                               'callerElapsedMs': unavailable.get('callerElapsedMs')}
            absent = rpc('member_diagnostics', expected_ok=False)
            require(absent.get('failure') == 'Error: owned member handle does not exist', 'no-ordinary-handle')
            state = rpc('status')
            require(state.get('id') == browser_id and state.get('stage') == 'Complete'
                and state.get('running') is True and state.get('authorization_saved') is True
                and state.get('secret_saved') is True, 'no-data-keeps-online-complete-materials')
            require(same_grant(material, rpc('material_receipt')), 'no-data-keeps-same-grant')
            report['noData'].update(carrier_facts(rpc('path_diagnostics')))
            with actor.condition:
                require(not any(event.get('event') in {'member-released', 'business'} for event in actor.events),
                        'no-data-native-business-event-absent')
            report['noData'].update(nativeBusinessEvents=0, identityMaterialsRunningUnchanged=True)
            stage('no-relay-browser-retire', lambda: stop_browser('after-no-data'))
        # Creation-denied modes qualify only their distinct branch. They do not
        # repeat the already separate no-data/restoration or full M83 stories.
        reopen_stage = 'same-browser-relay-restored' if args.mode == 'missing-answer' else 'same-browser-permitted-relay-reopen'
        restored = stage(reopen_stage, lambda: start_browser(reopened, data=True))
        restored_material = rpc('material_receipt')
        require_complete(restored_material)
        require(restored['id'] == browser_id and same_grant(material, restored_material),
                'relay-restoration-same-durable-identity-grant')
        report['identitiesAndGrantReopened'] = {'native': True, 'browser': True, 'sameGrant': True}
        stage('restored-runtime-capability', lambda: capability('restored'))
        stage('new-ordinary-sdk-member', lambda: rpc('open_member', {'target': native_id}))
        released = stage('native-current-member', lambda: actor.wait_event('member-released'))
        require(released.get('remoteId') == browser_id and released.get('alpn') == ALPN
            and released.get('memberAuthorized') is True and released.get('newProtocol') is True,
            'native-actual-member-peer-current-gate')
        require(type(released.get('stableId')) is int and released['stableId'] >= 0,
                'native-original-m-known')
        report['nativeCallerBudget'] = native_caller_budget_facts(ready, reopened, released)
        common = actor.wait_event(lambda event: event.get('event') == 'unified-path-ready'
            and event.get('purposeCode') == 1 and event.get('stableId') == released['stableId'])
        report['nativeCommon'] = common_facts(common, 'responder', RELAY)
        diagnostics = rpc('member_diagnostics')
        require(diagnostics.get('newProtocol') is True and diagnostics.get('alpn') == ALPN
            and diagnostics.get('memberAuthorized') is True and diagnostics.get('expectedPeerMatched') is True
            and diagnostics.get('ordinaryHandle') is True, 'browser-actual-member-peer-current-gate')
        report['browserCommon'] = common_facts(diagnostics.get('coordinator', {}), 'initiator', RELAY)
        require(any(path.get('isRelay') is True and path.get('selected') is True
                    and path.get('route') == 'relay:'+RELAY for path in diagnostics.get('paths', []))
            and not any(path.get('isRtc') is True for path in diagnostics.get('paths', []))
            and any(path.get('isRelay') is True and path.get('selected') is True
                    and path.get('isSignal') is False for path in released.get('paths', [])),
            'actual-selected-allowed-relay-only')
        require(stage('actual-sdk-bidirectional-business', lambda: rpc('exchange', {'message': 'PING'}))
                == 'PONG', 'actual-nonempty-business-reply')
        business = stage('native-original-m-business', lambda: actor.wait_event('business'))
        require(business.get('stableId') == released['stableId'] and business.get('alpn') == ALPN
            and type(business.get('bytes')) is int and business['bytes'] == 4
            and business.get('memberMessage') is True and business.get('reply') == 'PONG',
            'native-business-original-m-matched')
        after = rpc('member_diagnostics')
        report['browserCommonAfterBusiness'] = common_facts(after.get('coordinator', {}), 'initiator', RELAY)
        require(after['coordinator'].get('stableId') == diagnostics['coordinator'].get('stableId'),
                'same-browser-m-after-business')
        report['member'] = {'alpn': ALPN, 'bothActualPeerIdentitiesMatched': True,
            'bothCurrentGates': True, 'selectedPath': 'permitted-relay', 'newAttemptAfterReopen': True,
            'noDataRestorationQualified': args.mode == 'missing-answer',
            'browserToNativeBytes': 4, 'nativeToBrowserBytes': 4, 'replyMatched': True,
            'nativeOriginalMMatched': True, 'browserOriginalMUnchanged': True}
        stage('business-handle-close', lambda: rpc('close_member'))
        stage('final-sdk-stop', lambda: stop_browser('final'))
        body_passed = True
    except Exception as error:
        fail(error, phase)
        # First DTO/exception is already private. Only now collect failed-start
        # socket metadata and actor commands; no destructive getter precedes it.
        if page is not None and failed_start:
            try:
                report['browserStartObservation'] = persist_browser_start_observation(args.output,
                    page, observer, 'mio76-first-failed-start')
            except Exception as retention_error:
                report['startObservationFailureClass'] = type(retention_error).__name__
        try:
            snapshot_commands('first-failure', error)
        except Exception as retention_error:
            report['executionRetentionFailureClass'] = type(retention_error).__name__
    finally:
        report['cleanup'] = {}
        if observer is not None:
            try:
                result = observer.close(); report['cleanup']['observer'] = result
                require(exact_zero(result.get('commandExit')) and result.get('forced') is False
                    and result.get('readerEnded') is True and finite_ms(result.get('elapsedMs')),
                    'natural-browser-observer-retirement')
            except Exception as error:
                fail(error, 'observer-cleanup')
        if page is not None:
            try:
                result = page.close()
                private_receipt = private_write(args.output/'private-browser-cleanup.json', result)
                report['cleanup']['browser'] = {**private_receipt, 'qualified': False}
                save()
                require(browser_cleanup_succeeded(result), 'owned-browser-page-retirement')
                require(exact_zero(result.get('forwardRemoveExit')) and exact_zero(result.get('reverseRemoveExit')),
                        'adb-forward-reverse-exits-exact-zero')
                require(finite_ms(result.get('elapsedMs')), 'browser-independent-cleanup-timing')
                sdk = result['sdk']['result']
                objects = retirement_facts(sdk, args.mode, generation, final=True)
                report['cleanup']['browser'] = {**private_receipt, **objects, 'qualified': False,
                    'tabAbsent': result['tab']['ownedTargetAbsent'],
                    'forwardRuleAbsent': result['forwardRuleAbsent'],
                    'reverseRuleAbsent': result['reverseRuleAbsent'],
                    'localListenerClosed': result['localListenerClosed'],
                    'ownedStoreDeleted': sdk['storage']['deleted'], 'fullCleanupElapsedMs': result['elapsedMs']}
                history = sdk.get('cleanupHistory', [])
                require(all(type(value.get('generation')) is int for value in history)
                    and [value.get('generation') for value in history] == list(range(1, generation+1)),
                        'every-browser-generation-retired')
                require(all(value.get('cleanupSucceeded') is True for value in history),
                        'every-browser-generation-cleanup-succeeded')
                report['cleanup']['browser']['qualified'] = True
            except Exception as error:
                fail(error, 'browser-cleanup')
        if actor is not None:
            try:
                retired = close_native(actor)
                private_receipt = private_write(args.output/'private-native-cleanup.json', retired)
                report['cleanup']['native'] = {**private_receipt, 'qualified': False}
                entry = {'generation': len(actor.runs), **private_receipt, 'qualified': False}
                report['nativeGenerationCleanup'].append(entry)
                save()
                facts = native_retirement_facts(retired)
                entry.update(facts, qualified=True)
                report['cleanup']['native'] = {**private_receipt, **facts, 'qualified': True}
            except Exception as error:
                fail(error, 'native-cleanup')
        try:
            snapshot_commands('final')
            require(runtime_hashes() == frozen, 'runtime-controller-helper-freeze')
            require(report.get('selectedProfiles') and all(sha(getattr(args, name)) == digest
                for name, digest in report['selectedProfiles'].items()), 'configuration-source-freeze')
            guard_build(args.native_build_receipt, args.native_build_receipt_sha256,
                binary=args.native_binary, binary_pin=args.native_binary_sha256)
            guard_build(args.browser_build_receipt, args.browser_build_receipt_sha256,
                package=args.package, package_pin=args.package_sha256)
            report['runtimeSourcesAndSelectedSdkArtifactsStable'] = True
        except Exception as error:
            fail(error, 'final-source-artifact-guard')
        report['passed'] = body_passed and 'firstFailure' not in report
        report['newWireQualified'] = report['passed']
        report['commandExit'] = 0 if report['passed'] else 1
        report['completedUtc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        save()
    return report['commandExit']


def development_checks():
    """Private evidence and rejection counterexamples only; never mock SDK success."""
    failures = 0
    for value in (False, True, -1, None, 0.0, '0'):
        require(not exact_zero(value), 'boolean-or-unknown-is-not-zero')
        failures += 1
    for reply in ({'ok': False, 'failure': 'Error: connection deadline exceeded'},
                  {'ok': True, 'failure': 'Error: no currently permitted data path'},
                  {'ok': False, 'failure': 'prefix no currently permitted data path private-value'}):
        try:
            no_path_class(reply)
        except EvidenceFailure:
            failures += 1
        else:
            raise EvidenceFailure('unrelated-error-accepted-as-data-path-unavailable')
    empty = {'cumulative': True, 'resetAttempts': 0,
        'injection': {'mode': 'missing-answer', 'necessaryAnswerMissing': True},
        'peerCreated': 0, 'peerCloseCalls': 0, 'channelCreated': 0, 'channelCloseCalls': 0,
        'openPeerConnections': 0, 'openDataChannels': 0, 'openWebSockets': 0}
    for name, replacement in [('resetAttempts', 1), ('peerCreated', False),
            ('openPeerConnections', 1), ('openDataChannels', None), ('openWebSockets', False)]:
        try:
            fixture_facts({**empty, name: replacement}, 'missing-answer', 1)
        except EvidenceFailure:
            failures += 1
        else:
            raise EvidenceFailure('reset-unknown-or-live-resource-accepted')
    for bad in ({'cleanupSucceeded': False}, {'cleanupSucceeded': True, 'networkClosed': False},
                {'cleanupSucceeded': True, 'networkClosed': True, 'networkFreed': True,
                 'ownedMemberHandles': False, 'errors': []}):
        try:
            retirement_facts(bad, 'missing-answer', 1)
        except EvidenceFailure:
            failures += 1
        else:
            raise EvidenceFailure('unretired-or-boolean-resource-count-accepted')
    with tempfile.TemporaryDirectory(prefix='mio76-private-evidence-check-') as temporary:
        output = Path(temporary)
        capture = PrivateRpcs(output)
        sentinel = 'private SDK identity/exporter must remain exact privately'
        facts = capture.record('open_member', reply={'ok': False, 'failure': sentinel})
        recorded = json.loads((output/facts['privateFile']).read_text())
        require(recorded['reply']['failure'] == sentinel
            and (output/facts['privateFile']).stat().st_mode & 0o777 == 0o600
            and sentinel not in json.dumps(facts), 'exact-private-first-dto-public-hash-only')
        try:
            private_write(output/facts['privateFile'], {'overwritten': True})
        except FileExistsError:
            failures += 1
        else:
            raise EvidenceFailure('first-failure-private-evidence-overwritten')
    return {'counterexamplesRejected': failures, 'exactPrivateFirstDtoPreserved': True,
            'sdkExecution': False, 'deviceExecution': False, 'publicExecution': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--development-checks', action='store_true')
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--mode', choices=MODES, default='missing-answer')
    for name in ('native-build-receipt', 'native-binary', 'browser-build-receipt', 'package',
                 'facility', 'native-profile', 'browser-profile', 'output'):
        parser.add_argument('--'+name, type=Path)
    for name in ('native-build-receipt', 'native-binary', 'browser-build-receipt', 'package'):
        parser.add_argument('--'+name+'-sha256')
    args = parser.parse_args()
    if args.development_checks:
        require(not args.execute, 'development-checks-never-execute-sdk')
        print(json.dumps(development_checks(), indent=2)); return 0
    if not args.execute:
        parser.error('explicit --execute is required; --development-checks is offline only')
    for name in ('native_build_receipt', 'native_binary', 'browser_build_receipt', 'package',
                 'facility', 'native_profile', 'browser_profile', 'output',
                 'native_build_receipt_sha256', 'native_binary_sha256',
                 'browser_build_receipt_sha256', 'package_sha256'):
        if getattr(args, name) is None:
            parser.error('--'+name.replace('_', '-')+' is required for actual execution')
    return run(args)


if __name__ == '__main__':
    sys.exit(main())
