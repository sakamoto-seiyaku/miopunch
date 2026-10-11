#!/usr/bin/env python3
"""Root-owned genuine encoded ordinary PONG production pending discard on actual public resource retirement.

Exact actual final SDK receipts and closure must pass before any actor starts.
The actual public invocation requires Root's fresh current user authorization.
Historical other case branches are unreachable under the single-case guard.
"""
import argparse
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time
import traceback
import threading
from urllib.parse import urlsplit

ROOT = Path("/home/js/Git/miopunch").resolve()
sys.path.insert(0, str(ROOT / "scripts"))
PAGE = Path(__file__).resolve().with_name("mio77-maintained-page-60s.js")
MEMBER_ALPN = "miopunch/session/member/1"
NEGATIVE_CASES = frozenset(("mio81-browser-trigger", "mio81-native-trigger"))
INJECTOR = Path(__file__).resolve().with_name("active-candidate-send-fixture.js")
PRIVATE_PACKET_OBSERVER = Path(__file__).resolve().with_name("private-rtc-packet-observer.js")
PHYSICAL_CUT = Path(__file__).resolve().with_name("owned-browser-physical-carrier-cut.js")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class EvidenceFailure(RuntimeError):
    def __init__(self, check, reason):
        super().__init__(reason)
        self.check, self.safe_reason = check, reason


class CaseCompleted(Exception):
    """Stop one selected case while preserving shared cleanup and source guards."""


def require(condition, check, reason):
    if not condition:
        raise EvidenceFailure(check, reason)


def stable_failure_reason(reply):
    value = str(reply.get("failure", ""))
    for reason in ("IdentityOrAdmissionRejected", "ProtocolUnsupported", "InvalidControl",
                   "DataPathUnavailable", "DeadlineExceeded", "Cancelled"):
        if reason in value:
            return reason

    # Browser JsValue errors arrive as a string, so the page's generic
    # exception DTO has no useful `name`/`failureClass`.  Keep this mapping
    # narrow and stable: these are the Display strings of the shared
    # ConnectError variants (plus the caller-deadline wording emitted by the
    # browser adapter).  A transport-specific or unknown message remains
    # unqualified instead of being promoted to a bounded path failure.
    normalized = " ".join(value.casefold().split())
    display_reasons = {
        "identity or admission rejected": "IdentityOrAdmissionRejected",
        "unified connection protocol unsupported": "ProtocolUnsupported",
        "invalid protected connection control": "InvalidControl",
        "no currently permitted data path": "DataPathUnavailable",
        "connection deadline exceeded": "DeadlineExceeded",
        "connection attempt cancelled": "Cancelled",
        "active improvement deadline exceeded": "DeadlineExceeded",
        "browser connection caller deadline exceeded": "DeadlineExceeded",
    }
    if normalized in display_reasons:
        return display_reasons[normalized]
    if value == ('active improvement failed: Custom { kind: Other, error: '
                 'ReadError(ConnectionLost(ApplicationClosed(ApplicationClose { '
                 'error_code: 0, reason: b"active improvement failed" }))) }'):
        # Distinct actual peer candidate rejection, not an inferred timeout.
        # The selected negative case must additionally prove exact-M failed
        # candidate cleanup, one failed peer attempt and bounded return.
        return "PeerCandidateControlClosed"
    return reply.get("failureClass") or "UnknownFailure"


def integer(value, minimum=0):
    return type(value) is int and value >= minimum


def require_active_commit_proof(after_active, original_stable_id):
    """Current active DTO: finalized adapter proof plus exact common commit."""
    bootstrap = after_active.get("bootstrap") or {}
    coordinator = after_active.get("coordinator") or {}
    require(bootstrap.get("switched") is True
            and bootstrap.get("finalized") is True
            and bootstrap.get("freshAck") is True
            and bootstrap.get("finalReleaseFreshAck") is True
            and bootstrap.get("stableUnchanged") is True
            and bootstrap.get("bindingUnchanged") is True,
        "positive-bilateral-retain", "active C lacks fresh ACK and bilateral retained proof")
    require(coordinator.get("event") == "active-improvement-committed"
            and coordinator.get("freshAck") is True
            and coordinator.get("bothReady") is True
            and integer(original_stable_id)
            and coordinator.get("stableId") == original_stable_id,
        "positive-coordinator-commit", "common active coordinator commit proof is missing")


def remember_failure(report, error, phase):
    if "firstFailure" not in report:
        report["firstFailure"] = {"phase": phase, "failureClass": type(error).__name__,
            "check": error.check if isinstance(error, EvidenceFailure) else phase,
            "safeReason": error.safe_reason if isinstance(error, EvidenceFailure)
                else "operation raised; details retained privately"}


class PrivateEvidence:
    """Exclusive0600 raw evidence exists before any preflight/actor operation."""
    def __init__(self, output):
        self.path = output / "private-operation-evidence.jsonl"
        descriptor = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        self.stream = os.fdopen(descriptor, "w", encoding="utf-8")
        self.records = 0

    def write(self, operation, value):
        self.stream.write(json.dumps({"operation": operation, "observedUtcNs": time.time_ns(),
            "value": value}, ensure_ascii=False, default=str) + "\n")
        self.stream.flush()
        self.records += 1

    def exception(self, phase, error):
        self.write(phase, {"failureClass": type(error).__name__, "failure": str(error),
            "traceback": traceback.format_exc()})

    def close(self):
        if not self.stream.closed:
            self.stream.flush()
            os.fsync(self.stream.fileno())
            self.stream.close()
        return {"privateFile": self.path.name, "sha256": sha(self.path),
            "recordCount": self.records, "rawContentPublished": False}


def native_snapshot(actor):
    if actor is None:
        return None
    with actor.condition:
        return {"events": list(actor.events), "stderr": list(actor.stderr),
            "runs": json.loads(json.dumps(actor.runs, default=str)), "artifacts": dict(actor.artifacts)}


def object_counts_retired(value):
    return all(integer(value.get(name)) and value[name] == 0 for name in (
        "openPeerConnections", "openDataChannels", "openWebSockets"))


def _websocket_origin(value):
    try:
        parsed = urlsplit(value)
        if parsed.scheme not in {"ws", "wss", "http", "https"} or not parsed.hostname:
            return None
        scheme = {"http": "ws", "https": "wss"}.get(parsed.scheme, parsed.scheme)
        port = parsed.port or (443 if scheme == "wss" else 80)
        return f"{scheme}://{parsed.hostname}:{port}"
    except (TypeError, ValueError):
        return None


def browser_terminal_socket_tail(value, expected_wss=None, expected_relay=None):
    """Project a known, quiescent WebSocket close tail without calling it retired.

    M76's pinned relay stack can retain the callbacks needed by its terminal
    close future while the SDK has already closed/freed its endpoint and page
    resources.  The M77 page has only passive socket state/traffic witnesses,
    so this accepts a configured WSS socket in CLOSING and reports the pending
    tail explicitly.  Unknown origins, active sockets, changing traffic, or a
    missing socket witness remain failures.  No callback/task completion is
    inferred from this DTO.
    """
    sdk = value.get("sdk", {}).get("result", {})
    immediate = sdk.get("rtcObjectsImmediate", {})
    late = sdk.get("rtcObjectsLate", {})
    sockets_before = immediate.get("sockets")
    sockets_after = late.get("sockets")
    witnesses_before = immediate.get("webSocketCloseWitnesses")
    witnesses_after = late.get("webSocketCloseWitnesses")
    if not isinstance(sockets_before, list) or not isinstance(sockets_after, list):
        return None
    if len(sockets_before) != len(sockets_after) or not sockets_before:
        return None
    if (not isinstance(witnesses_before, list) or not isinstance(witnesses_after, list)
            or len(witnesses_before) != len(sockets_before)
            or len(witnesses_after) != len(sockets_after)):
        return None
    allowed = {_websocket_origin(expected_wss), _websocket_origin(expected_relay)}
    allowed.discard(None)
    if not allowed:
        return None
    pending = []
    relay_pending = []
    mqtt_pending = []
    for index, (before, after) in enumerate(zip(sockets_before, sockets_after)):
        witness_before = witnesses_before[index]
        witness_after = witnesses_after[index]
        if (not isinstance(before, dict) or not isinstance(after, dict)
                or not isinstance(witness_before, dict) or not isinstance(witness_after, dict)):
            return None
        origin = _websocket_origin(before.get("origin"))
        if origin not in allowed or _websocket_origin(after.get("origin")) != origin:
            return None
        state_before, state_after = before.get("readyState"), after.get("readyState")
        if type(state_before) is not int or type(state_after) is not int:
            return None
        if state_before not in (2, 3) or state_after not in (2, 3) or state_after < state_before:
            return None
        for name in ("sentBytes", "receivedBytes", "sentFrames", "receivedFrames"):
            if (not integer(before.get(name)) or not integer(after.get(name))
                    or before[name] != after[name]):
                return None
        if (witness_before.get("socketIndex") != index
                or witness_after.get("socketIndex") != index
                or witness_before.get("readyState") != state_before
                or witness_after.get("readyState") != state_after
                or not integer(witness_before.get("closeCalls"), 1)
                or not integer(witness_after.get("closeCalls"), 1)
                or not integer(witness_before.get("closeFailures"))
                or not integer(witness_after.get("closeFailures"))
                or witness_before["closeFailures"] != 0
                or witness_after["closeFailures"] != 0
                or not integer(witness_before.get("bufferedBytes"))
                or not integer(witness_after.get("bufferedBytes"))
                or witness_after["bufferedBytes"] != 0
                or type(witness_before.get("firstCloseAtMs")) not in (int, float)
                or type(witness_after.get("firstCloseAtMs")) not in (int, float)
                or witness_before["firstCloseAtMs"] < 0
                or witness_after["firstCloseAtMs"] < 0
                or type(witness_before.get("stateBeforeFirstClose")) is not int
                or type(witness_after.get("stateBeforeFirstClose")) is not int
                or witness_before["stateBeforeFirstClose"] not in (0, 1, 2, 3)
                or witness_after["stateBeforeFirstClose"] not in (0, 1, 2, 3)
                or witness_before.get("stateAfterFirstClose") not in (2, 3)
                or witness_after.get("stateAfterFirstClose") not in (2, 3)
                or witness_before.get("callbackTrackingComplete") is not True
                or witness_after.get("callbackTrackingComplete") is not True
                or witness_before.get("sdkRegisteredListeners") != 0
                or witness_after.get("sdkRegisteredListeners") != 0):
            return None
        callbacks_before = witness_before.get("sdkPropertyCallbacks")
        callbacks_after = witness_after.get("sdkPropertyCallbacks")
        if (not integer(callbacks_before) or not integer(callbacks_after)
                or callbacks_before != callbacks_after):
            return None
        if state_after == 2:
            pending.append(index)
            if origin == _websocket_origin(expected_relay):
                if callbacks_after not in (0, 4):
                    return None
                relay_pending.append(index)
            elif origin == _websocket_origin(expected_wss):
                if callbacks_after != 0:
                    return None
                mqtt_pending.append(index)
            else:
                return None
        elif callbacks_after != 0:
            return None
    if immediate.get("openWebSockets") != len(
            [socket for socket in sockets_before if socket.get("readyState") == 2]):
        return None
    if late.get("openWebSockets") != len(
            [socket for socket in sockets_after if socket.get("readyState") == 2]):
        return None
    if not pending:
        return {
            "observed": True,
            "pendingSocketCount": 0,
            "pendingRelaySocketCount": 0,
            "pendingMqttSocketCount": 0,
            "trafficUnchangedAt100Ms": True,
            "gracefulRelayCloseObserved": None,
            "allSdkTerminalTasksRetiredProven": None,
        }
    if not relay_pending and not mqtt_pending:
        return None
    return {
        "observed": True,
        "pendingSocketCount": len(pending),
        "pendingRelaySocketCount": len(relay_pending),
        "pendingMqttSocketCount": len(mqtt_pending),
        "trafficUnchangedAt100Ms": True,
        "gracefulRelayCloseObserved": False if relay_pending else None,
        "allSdkTerminalTasksRetiredProven": False,
    }


def rtc_objects_retired(value, socket_tail=None):
    if not all(integer(value.get(name)) and value[name] == 0 for name in (
            "openPeerConnections", "openDataChannels")):
        return False
    if integer(value.get("openWebSockets")) and value["openWebSockets"] != 0:
        return socket_tail is not None
    return integer(value.get("openWebSockets")) and value["openWebSockets"] == 0


def public_cleanup(value):
    """Whitelist cleanup facts; opaque SDK maps/IDs/errors remain private."""
    allowed = {
        "nativePlatform", "forced", "commandExit", "ownershipRefused", "leaderWaitedFor",
        "ownedHostProcessAbsent", "ownedHostProcessAbsentLate",
        "ownedProcessGroupAbsentImmediately", "ownedProcessGroupAbsentLate",
        "localReadersEnded", "localPipesClosed", "ownedDirectoryRemoved",
        "elapsedMs", "lateDelayMs", "observed", "processCount", "socketCount", "ownership",
        "sdkRequired", "tabRequired", "forwardRequired", "reverseRequired",
        "forwardRemoveExit", "reverseRemoveExit", "forwardRuleAbsent", "reverseRuleAbsent",
        "localListenerClosed", "cleanupSucceeded", "ok", "ownedTargetAbsent",
        "openedByRunner", "networkClosed", "networkFreed", "ownedMemberCount",
        "memberCloseCount", "memberFreeCount", "ownedMemberHandles", "cleanupElapsedMs",
        "generation", "delayMs", "activeAssociations", "activePumpTasks", "cleanupObservers",
        "resourceAllowed", "mode", "injected", "peerCreated", "peerCloseCalls",
        "channelCreated", "channelCloseCalls", "openPeerConnections", "openDataChannels",
        "openWebSockets", "required", "ownedDatabaseClaimed", "exclusiveStorageLock",
        "deleted", "outcome", "terminal", "pending", "ownedPageTerminationObserved",
        "activeRtcAssociations", "activeRtcPumps", "socketsBefore", "socketsLate",
        "socketsImmediate", "cleanupMs", "maintenanceTaskJoined", "maintenanceStopped", "applicationHandlers",
        "applicationStreamHandlers",
        "driver_active", "queued_packets", "sdkRetirementMissing",
    }
    containers = {
        "sdk", "result", "tab", "storage", "cleanupHistory", "networkDiagnostics",
        "memberDiagnostics", "rtcObjectsImmediate", "rtcObjectsLate", "sdkRetirement",
        "immediate", "late", "traffic", "ownedProcessGroupBeforeClose",
        "ownedProcessGroupImmediate", "ownedProcessGroupLate",
    }
    if isinstance(value, list):
        return [public_cleanup(child) for child in value]
    if isinstance(value, dict):
        observed = {key: public_cleanup(child) for key, child in value.items()
            if key in allowed or key in containers}
        if "errors" in value:
            observed["errorCount"] = len(value["errors"]) if isinstance(value["errors"], list) else None
        if "publishedRoutes" in value:
            observed["publishedRtcRouteCount"] = len(value["publishedRoutes"]) \
                if isinstance(value["publishedRoutes"], list) else None
        return observed
    return value


def native_retired(value, final=True):
    from host_native_runtime import host_native_cleanup_succeeded
    if not host_native_cleanup_succeeded(value, final):
        return False
    if not integer(value.get("commandExit")) or value["commandExit"] != 0:
        return False
    sdk = value.get("sdkRetirement", {})
    if not all(integer(sdk.get(name)) and sdk[name] == 0
               for name in ("activeRtcAssociations", "activeRtcPumps")):
        return False
    if not all(integer(sdk.get(name)) for name in ("socketsBefore", "socketsLate")):
        return False
    if sdk["socketsLate"] > sdk["socketsBefore"]:
        return False
    if "maintenanceTaskJoined" in sdk:
        if sdk["maintenanceTaskJoined"] is not True or sdk.get("maintenanceStopped") is not True:
            return False
        for phase in ("immediate", "late"):
            sample = sdk.get(phase, {})
            if not all(integer(sample.get(name)) and sample[name] == 0 for name in (
                    "activeRtcAssociations", "activeRtcPumps", "applicationHandlers",
                    "applicationStreamHandlers")):
                return False
            if not all(integer(sample.get("traffic", {}).get(name))
                    and sample["traffic"][name] == 0 for name in ("driver_active", "queued_packets")):
                return False
    return True


def browser_retired(value, expected_wss=None, expected_relay=None):
    from android_pair_runtime import browser_cleanup_succeeded
    if not browser_cleanup_succeeded(value):
        return False
    sdk = value.get("sdk", {}).get("result", {})
    socket_tail = browser_terminal_socket_tail(value, expected_wss, expected_relay)
    for history in sdk.get("cleanupHistory", []):
        if not integer(history.get("ownedMemberHandles")) or history["ownedMemberHandles"] != 0:
            return False
        samples = history.get("networkDiagnostics", [])
        if len(samples) != 2 or [sample.get("delayMs") for sample in samples] != [0, 100]:
            return False
        if any(not all(integer(sample.get("observed", {}).get(name))
                and sample["observed"][name] == 0 for name in (
                    "activeAssociations", "activePumpTasks"))
                or not integer(sample.get("observed", {}).get("cleanupObservers"))
                for sample in samples):
            return False
        if samples[1].get("observed", {}).get("cleanupObservers") != 0:
            return False
        if any(sample.get("observed", {}).get("publishedRoutes") != [] for sample in samples):
            return False
    return (bool(sdk.get("cleanupHistory"))
        and rtc_objects_retired(sdk.get("rtcObjectsImmediate", {}), socket_tail)
        and rtc_objects_retired(sdk.get("rtcObjectsLate", {}), socket_tail))


def compiled_inputs(receipt):
    import check_unified_maintained as current_guard
    return current_guard.compiled_inputs(receipt)



def current_sources():
    paths = {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'build.rs',
        ROOT / '.cargo/config', ROOT / '.cargo/config.toml', ROOT / 'scripts/build_android_pair.py',
        ROOT / 'scripts/android/MioPublicShell.java', ROOT / 'scripts/cargo_cache.py'}
    for directory in (ROOT / 'src', ROOT / 'vendor'):
        paths.update(path for path in directory.rglob('*')
            if not {'.git', 'target', 'node_modules', '__pycache__'}.intersection(path.parts))
    paths.update(path for path in (ROOT / 'browser-probe').rglob('*')
        if path.suffix == '.rs' or path.name in {'Cargo.toml', 'Cargo.lock', 'build.rs'})
    paths.update((ROOT / 'tests').rglob('*.rs'))
    paths.update(ROOT / name for name in ('scripts/check_unified_maintained.py',
        'scripts/test_maintained_story_evidence.py'))
    return dict(sorted((str(path.relative_to(ROOT)), sha(path)) for path in paths if path.is_file()))


def validate_inputs(args):
    from types import SimpleNamespace
    import check_unified_maintained as current_guard
    values = dict(vars(args)); values['case'] = 'positive'
    result, facility = current_guard.validate_inputs(SimpleNamespace(**values))
    freeze = json.loads(args.runtime_freeze.read_text())
    for path in [Path(__file__).resolve(), PAGE, INJECTOR, PRIVATE_PACKET_OBSERVER, PHYSICAL_CUT]:
        require(freeze['files'].get(str(path)) == sha(path), 'current-carrier-observer-pin', 'directly loaded current observer input not frozen')
    return result, facility



def rtc_prelude():
    path = ROOT / "browser-probe/harness/rtc-capability-fixture.mjs"
    script = "import {rtcCapabilityPrelude} from " + json.dumps(path.as_uri()) \
        + ";process.stdout.write(rtcCapabilityPrelude('observe'));"
    return subprocess.run(["rtk", "proxy", "node", "--input-type=module", "-e", script],
        check=True, capture_output=True, text=True, timeout=10).stdout + "\n" + PHYSICAL_CUT.read_text() + "\n" + PRIVATE_PACKET_OBSERVER.read_text() + "\n" + INJECTOR.read_text()


STAGES = frozenset(('peer-committed-observed', 'post-committed-original-checked',
    'post-committed-control-check-failed', 'post-committed-control-checked',
    'post-committed-send-finish-failed', 'rollback-disarmed', 'common-committed'))
REASONS = frozenset(('explicit-route-close', 'resource-or-registry-invalid',
    'qualification-or-attempt-invalid', 'original-M-closed', 'pump-guard-exit',
    'registry-shutdown', 'connection-route-replaced', 'resource-permission-changed',
    'receive-buffer-too-small', 'prepared-path-rollback', 'candidate-cancellation-cleanup'))

def project_retirement_events(events, original_stable_id, control_stable_id):
    projected = []
    for event in events:
        name = event.get('event')
        if name == 'unified-active-committed' and event.get('stableId') == original_stable_id and event.get('controlStableId') == control_stable_id:
            if event.get('stage') not in STAGES:
                raise ValueError('unrecognized commit diagnostic stage')
            selected = {'event': name, 'stage': event['stage']}
            for key in ('originalOpen', 'controlOpen', 'originalAuthorized', 'controlAuthorized', 'freshAck', 'bothReady'):
                if key in event:
                    if type(event[key]) is not bool:
                        raise ValueError('invalid diagnostic boolean')
                    selected[key] = event[key]
        elif name == 'unified-active-result' and event.get('controlStableId') == control_stable_id:
            if event.get('failureClass') not in (None, 'timed-out', 'permission-denied', 'invalid-data', 'other'):
                raise ValueError('unrecognized fixed failure class')
            if type(event.get('ok')) is not bool or type(event.get('controlOpen')) is not bool:
                raise ValueError('invalid handler result boolean')
            selected = {'event': name, 'ok': event['ok'], 'controlOpen': event['controlOpen'],
                        'failureClass': event['failureClass']}
        elif name == 'rtc-pump-ended' and event.get('stableId') == original_stable_id:
            diagnostic = event.get('retirementDiagnostic')
            if diagnostic is None:
                selected = {'event': name, 'retirement': 'UNOBSERVED'}
            else:
                if diagnostic.get('reason') not in REASONS or type(diagnostic.get('monotonicMs')) is not int:
                    raise ValueError('unrecognized retirement diagnostic')
                selected = {'event': name, 'retirementReason': diagnostic['reason'],
                            'retirementMonotonicMs': diagnostic['monotonicMs']}
        else:
            continue
        if type(event.get('monotonicMs')) is not int:
            raise ValueError('invalid event monotonic timestamp')
        selected['monotonicMs'] = event['monotonicMs']
        projected.append(selected)
        if len(projected) > 16:
            raise ValueError('bounded single-attempt diagnostic projection exceeded')
    return projected


class NativeProbeBridge:
    """Only actual samples from the existing observer; never initiates actor work."""
    def __init__(self):
        self.lock, self.latest, self.sampled_at = threading.Lock(), None, None

    @staticmethod
    def project(status, index, expected_id):
        ordinary = status.get("ordinary") or {}
        paths = [{"pathId":path.get("pathId"), "streamSent": path.get("streamSent"), "streamReceived": path.get("streamReceived"),
                  "datagramSent":path.get("datagramSent"), "datagramReceived":path.get("datagramReceived")}
                 for path in ordinary.get("paths", []) if path.get("isRtc") is True]
        return {"sampleIndex": index, "originalStableIdMatched": ordinary.get("stableId") == expected_id,
                "open": ordinary.get("open"), "memberAuthorized": ordinary.get("memberAuthorized"),
                "stableUnchanged": ordinary.get("stableUnchanged"), "bindingUnchanged": ordinary.get("bindingUnchanged"),
                "rtcPaths": paths, "probe":status.get("actualNativeProbeEvent"), "rtcResource":status.get("rtcResource"), "nativeDatagramFrames": "measured per actual path"}

    def publish(self, status, index, expected_id):
        with self.lock:
            self.latest = self.project(status, index, expected_id)
            self.sampled_at = time.monotonic()

    def reply(self, handler, page):
        if urlsplit(handler.path).path != "/native-probe-observation":
            return False
        if handler.path != "/native-probe-observation?actor=" + page.actor or \
                handler.headers.get("Host") != "127.0.0.1:" + str(page.port):
            handler.send_error(404)
            return True
        with self.lock:
            value = {"observation": self.latest, "sampleAgeMs":
                     (time.monotonic() - self.sampled_at) * 1000 if self.sampled_at is not None else None}
        handler.send_bytes(json.dumps(value).encode(), "application/json")
        return True


def observe_native_during_call(actor, call, evidence, bridge=None, expected_id=None, command_lock=None):
    """Bounded read-only actual Native status during this one real Browser SDK call."""
    stop = threading.Event()
    snapshots, errors = [], []
    def observe():
        while not stop.is_set() and len(snapshots) < 320:
            try:
                with command_lock if command_lock is not None else nullcontext():
                    value = actor.command(json.dumps({"op":"status"}), "status")
                status = dict(value["status"])
                if bridge is not None:
                    with actor.condition:
                        markers = [event for event in actor.events if event.get("event") == "rtc-active-progress"
                            and event.get("stableId") == expected_id]
                    if markers:
                        status["actualNativeProbeEvent"] = dict(markers[-1])
                        if not snapshots or snapshots[-1].get("actualNativeProbeEvent") != markers[-1]:
                            print(json.dumps({"nativeFlow":markers[-1]["stage"],"watermark":markers[-1]["probeWatermark"],
                                "elapsedMs":markers[-1]["elapsedMs"],"frames":markers[-1].get("frames")}),flush=True)
                snapshots.append(status)
                if bridge is not None:
                    bridge.publish(status, len(snapshots) - 1, expected_id)
            except BaseException as error:
                errors.append(type(error).__name__)
                break
            stop.wait(0.25)
    thread = threading.Thread(target=observe, name="m77-owned-passive-native-status")
    thread.start()
    try:
        result = call()
    finally:
        stop.set()
        thread.join(timeout=21)
        evidence.write("passive:Native-status-during-one-improve", {"snapshots":snapshots,
            "errorClasses":errors,"threadEnded":not thread.is_alive()})
        require(not thread.is_alive() and not errors and len(snapshots) < 320,
            "native-passive-observer-retired", "actual passive observer failed or did not retire")
    return result, snapshots


class ResourcePermissionBridge:
    """One actor-scoped real public operation; shared command lock prevents pipe races."""
    def __init__(self, command_lock):
        self.command_lock = command_lock
        self.lock = threading.Lock()
        self.actor = self.original_id = None
        self.applied = None
        self.calls = []

    def arm(self, actor, original_id):
        with self.lock:
            require(self.actor is None, "resource-one-scope", "resource operation already scoped")
            self.actor, self.original_id = actor, original_id

    def reply(self, handler, page):
        if urlsplit(handler.path).path != "/native-resource-disable":
            return False
        if handler.path != "/native-resource-disable?actor=" + page.actor or \
                handler.headers.get("Host") != "127.0.0.1:" + str(page.port) or \
                handler.headers.get("Origin") != "http://127.0.0.1:" + str(page.port):
            handler.send_error(404)
            return True
        try:
            length = int(handler.headers.get("Content-Length", "0"))
            if not 0 < length <= 4096:
                handler.send_error(413)
                return True
            body = json.loads(handler.rfile.read(length))
            if not isinstance(body, dict) or set(body) != {"pathId", "probeWatermark"} or \
                    not isinstance(body["pathId"],str) or not 0 < len(body["pathId"]) <= 64 or \
                    not integer(body["probeWatermark"],1):
                handler.send_error(400)
                return True
            with self.lock:
                if self.actor is None or len(self.calls) >= 32:
                    handler.send_error(409)
                    return True
                if self.applied is not None:
                    value = self.applied
                else:
                    request = {"op":"set-rtc-resource-allowed","allowed":False,
                        "originalStableId":self.original_id,"pathId":body["pathId"]}
                    started = time.monotonic()
                    # No page/probe/registry lock is held; observer and op use the
                    # same pipe lock, and the actor never waits for HTTP/page work.
                    with self.command_lock:
                        value = self.actor.command(json.dumps(request),"rtc-resource-permission-updated")
                    self.calls.append({"request":request,"probeWatermarkObservedByPage":body["probeWatermark"],
                        "actualResult":value,"operationWallMs":(time.monotonic()-started)*1000})
                    if value.get("applied") is True:
                        self.applied = value
            handler.send_bytes(json.dumps(value).encode(),"application/json")
        except BaseException as error:
            with self.lock:
                self.calls.append({"failureClass":type(error).__name__,"privateFailure":str(error)})
            # The immutable SDK/actor failure is collected by the main case;
            # no raw error, route, record or identity is printed from HTTP.
            handler.send_error(500)
        return True


def resource_source_order_proof(native_receipt):
    """Proof from exact retained compiled source, never infer scheduler order from samples."""
    source = native_receipt.parent / "selected-source-copies/src/rtc_transport.rs"
    text = source.read_text()
    start = text.index("    fn close_with_reason(&self, reason:")
    end = text.index("\n    }\n}",start)
    body = text[start:end]
    tokens = ("self.retired.swap(true", "self.ready.store(false", "self.qualified.store(false",
        "registry.routes.lock().unwrap().remove(&self.connection_id)", "registry.refresh()",
        "self.tx.close()", "self.rx.close()", "self.retirement.wake()")
    positions = [body.index(token) for token in tokens]
    return {"compiledSourceSha256":sha(source),"retiredReadyQualifiedThenExactWithdrawRefreshThenQueueWakes":
        positions == sorted(positions),"proofKind":"retained actual compiled source statement order",
        "schedulerOrderInferredFromSnapshots":False}

def verify_dequeued_private_capture(args, report, actor, evidence):
    """Independent raw retained encoder/transmit/pump chain after all producers ended."""
    data=report.get("queuedPrivateProofPendingFinalCapture")
    require(isinstance(data,dict),"queued-pending-proof","actual pending identity was not retained")
    stable=data["originalStableId"];digest=data["heldCiphertextSha256"];p=data["encoderProof"]
    private=args.output/"packet-witness.jsonl"
    require(private.is_file() and private.stat().st_mode&0o777==0o600
        and 0<private.stat().st_size<=24*1024*1024,"queued-private-capture-file","exact bounded private Native capture absent")
    records=[json.loads(line) for line in private.read_text().splitlines() if line.strip()]
    require(records and len(records)<=16384 and {r.get("sequence") for r in records}==set(range(1,len(records)+1)),
        "queued-private-capture-sequences","actual private record set is incomplete/duplicate")
    with actor.condition:
        endings=[e for e in actor.events if e.get("event")=="private-packet-capture-ended"]
    require(len(endings)==1 and endings[0].get("result",{}).get("records")==len(records)
        and endings[0]["result"].get("adapterLost")==endings[0]["result"].get("observerLost")==0
        and endings[0]["result"].get("writerNaturallyJoined") is True,
        "queued-actual-native-writer-ended","actual Native callback/writer lost records or did not join naturally")
    byseq={r["sequence"]:r for r in records}
    payload=p["proof"]["payload"];packet=p["proof"]["packet"]
    declaration=byseq.get(payload["declaration"]["sequence"],{})
    appended=byseq.get(payload["appendedSequence"],{})
    encrypted=byseq.get(p["proof"]["encryptedSequence"],{})
    carrier=byseq.get(p["actualBeforeCarrierSequence"],{})
    require(declaration.get("stage")=="ordinary-stream-declaration"
        and declaration.get("fields",{}).get("original_stable_id")==stable
        and declaration["fields"].get("stream_id")==payload["streamId"]
        and declaration["fields"].get("payload_len")==4
        and declaration["fields"].get("payload_sha256")==hashlib.sha256(b"PONG").hexdigest()
        and appended.get("stage")=="actual-payload-appended" and appended.get("connectionStableId")==stable
        and appended.get("fields",{}).get("class")=="STREAM" and appended["fields"].get("stream_id")==payload["streamId"]
        and appended["fields"].get("offset")==0 and appended["fields"].get("payload_len")==4
        and appended["fields"].get("payload_sha256")==declaration["fields"]["payload_sha256"]
        and all(appended["fields"].get(k)==packet.get(k) for k in ("packet_number","path_id","space"))
        and encrypted.get("stage")=="actual-packet-encrypted" and encrypted.get("connectionStableId")==stable
        and encrypted.get("fields")==packet
        and carrier.get("stage")=="before-actual-carrier-call" and carrier.get("connectionStableId")==stable,
        "queued-independent-actual-encoder-chain","raw actual ordinary declaration/encoder/packet/context chain differs")
    raw=bytes.fromhex(carrier.get("actualTransmitCiphertextHex") or "")
    f=carrier.get("fields") or {};begin=packet["packet_offset"];end=begin+packet["packet_len"]
    datagram=p["datagramOffset"];dend=datagram+p["datagramLength"]
    require(len(raw)==f.get("transmit_len")==p["actualTransmitLength"]
        and f.get("segment_size")==p["actualTransmitSegmentSize"]
        and 0<=datagram<=begin<end<=dend<=len(raw)
        and packet["datagram_offset"]==datagram
        and datagram%f["segment_size"]==0 and datagram//f["segment_size"]==packet["datagram_index"]
        and hashlib.sha256(raw[begin:end]).hexdigest()==packet["ciphertext_sha256"]
        and hashlib.sha256(raw[datagram:dend]).hexdigest()==digest,
        "queued-independent-exact-ciphertext-coalescing","raw same actual protected packet/final transmit/segment bytes do not match genuine pending")
    pumps=[r for r in records if r.get("fields",{}).get("original_stable_id")==stable
        and r["fields"].get("ciphertext_sha256")==digest]
    dequeued=[r for r in pumps if r.get("stage")=="actual-pump-dequeued"]
    dropped=[r for r in pumps if r.get("stage")=="actual-pump-pending-drop-after-work"]
    attempts=[r for r in pumps if r.get("stage") in {"actual-pump-try-send-enter","actual-pump-try-send-accepted","actual-pump-try-send-buffer-full"}]
    require(len(dequeued)==len(dropped)==1 and not attempts
        and dequeued[0]["fields"].get("packet_len")==dropped[0]["fields"].get("packet_len")==p["datagramLength"]
        and dropped[0]["fields"].get("route_active") is False
        and carrier["sequence"]<dequeued[0]["sequence"]<dropped[0]["sequence"],
        "queued-actual-pending-no-try-send","actual same pending ciphertext was attempted/accepted or exact dequeue/drop identity unavailable")
    retries=[r for r in records if r.get("stage")=="actual-payload-appended" and r.get("connectionStableId")==stable
        and r["sequence"]>dropped[0]["sequence"] and r.get("fields",{}).get("class")=="STREAM"
        and r["fields"].get("stream_id")==payload["streamId"] and r["fields"].get("offset")==0
        and r["fields"].get("payload_len")==4 and r["fields"].get("payload_sha256")==data["expectedOrdinaryPayloadSha256"]
        and r["fields"].get("packet_number")!=packet["packet_number"] and r["fields"].get("path_id")!=packet["path_id"]]
    actual_retry=[]
    for retry in retries:
        key=tuple(retry["fields"][k] for k in ("packet_number","path_id","space"))
        for encoded in records:
            if encoded.get("stage")!="actual-packet-encrypted" or encoded.get("connectionStableId")!=stable:continue
            e=encoded["fields"]
            if tuple(e[k] for k in ("packet_number","path_id","space"))!=key:continue
            for transmit in records:
                if transmit.get("stage")!="before-actual-carrier-call" or transmit.get("connectionStableId")!=stable:continue
                b=bytes.fromhex(transmit.get("actualTransmitCiphertextHex") or "")
                a=e["packet_offset"];z=a+e["packet_len"]
                if z<=len(b) and hashlib.sha256(b[a:z]).hexdigest()==e["ciphertext_sha256"]:
                    actual_retry.append({"payloadSequence":retry["sequence"],"encodedSequence":encoded["sequence"],
                        "transmitSequence":transmit["sequence"],"distinctPacketNumber":True,"distinctPath":True,
                        "distinctCiphertext":e["ciphertext_sha256"]!=packet["ciphertext_sha256"]})
    require(actual_retry and all(x["distinctCiphertext"] for x in actual_retry),
        "queued-actual-permitted-retransmission","ordinary PONG completion lacks independently observed distinct encrypted retransmission on another allowed path")
    before=report["queuedPrivateReceiveSummary"]
    cleanup=report.get("queuedPrivateObserverRetirement") or {}
    fixed=("factoryCreations","attachedChannels","observedMessages","trustedMessages","untrustedMessages","totalCount",
        "hashStarted","hashSucceeded","hashFailed","channelOverflow","observationOverflow","pendingOverflow",
        "unsupportedTypes","packetBoundFailures","observerErrors","pendingHashes","openTrackedChannels","collectedWithoutClose")
    require(cleanup.get("restored") is True and cleanup.get("factoryRestored") is True
        and cleanup.get("actualFactoryIsOriginal") is True and cleanup.get("attachedListeners")==0
        and cleanup.get("awaitedRejected")==0 and all(cleanup.get(k)==before.get(k) for k in fixed),
        "queued-actual-complete-observer-retirement","actual RTC receive/hash coverage changed after snapshot or listener/factory/hash tasks failed retirement")
    evidence.write("queued:independent-final-native-encoder-pump-verification",{"privateCaptureSha256":sha(private),
        "nativeCaptureEnded":endings[0],"heldCarrierRecord":carrier,"samePendingDequeued":dequeued[0],
        "samePendingDropped":dropped[0],"distinctAllowedRetransmission":actual_retry,"browserObserverRetirement":cleanup})
    report["m80DequeuedBusinessObservation"].update({"noSamePendingTrySendAcceptance":True,
        "actualNoTrySendEntryForHeldCiphertext":True,"actualDistinctEncryptedRetransmissionOverAnotherAllowedPath":True,
        "finalNativeCaptureLossZero":True,"finalPrivateBrowserObserverRetired":True,
        "heldSameCiphertextNeverReceivedOnRtcWithinCompleteOwnedObservation":True})
    report["m80DequeuedBusinessEvidence"]=True


def run(args):
    controlled_loopback = getattr(args, "controlled_loopback", False)
    host_browser = getattr(args, "host_chrome", None) is not None
    require(not args.output.exists(), "new-output", "preserve the existing output and first failure")
    args.output.mkdir(mode=0o700, parents=True)
    args.output.chmod(0o700)
    report = {"ticket": "MIOPUNCH-81", "m81Qualified": False,
        "proofPurpose": "same original M/stream recovery via actual common ACTIVE_IMPROVEMENT after sole owned physical RTC close",
        "scope": ("owned Host Native + Host Browser, controlled loopback data relay, local RTC candidate"
                  if host_browser and controlled_loopback else
                  "owned Host Native + Host Browser, existing approved APS1 contact, sole original RTC after old relay path retirement; local RTC/physical WAN unqualified"
                  if host_browser else "fixture-only actual maintained Node + Pixel Browser relay continuity feedback"),
        "actors": {"native":"owned-host-native", "browser":"owned-host-browser" if host_browser else "owned-pixel-browser"},
        "dataRelayScope": "owned local relay contact; old original-M relay paths actually retired before RTC cut; public relay feasibility unqualified",
        "rtcGeography": "local owned Host endpoints; physical WAN unqualified" if host_browser else "unqualified",
        "case": args.case, "passed": False, "m77Qualified": False, "m80Qualified":False,
        "networkFailureQualified": False, "physicalWanQualified": False,
        "command": ["rtk", "proxy", "python3", str(Path(__file__).resolve()), *sys.argv[1:]],
        "phases": [], "cleanup": {}, "sdkCalls": [], "firstRedCaptured": False,
        "firstRedQualified": False, "setupDeadlineSurvivalQualified": False, "fullGoalQualified": False,
        "positiveQualified": False, "causalWitnessQualified": False}
    evidence = PrivateEvidence(args.output)
    evidence.write("command", report["command"])
    case_started = time.monotonic()
    actor = page = browser_device = None
    native_command_lock = threading.RLock()
    resource_permission_bridge = ResourcePermissionBridge(native_command_lock)
    effective_relay = args.relay
    current = "preflight"
    inputs = None
    diagnostic_start_index = None
    diagnostic_original_id = None
    diagnostic_control_id = None

    def capture_native_diagnostics(wait_for_result=False):
        nonlocal diagnostic_control_id
        if actor is None or diagnostic_original_id is None or diagnostic_start_index is None:
            return None
        # Passive actual process output only. No command, actor action or inferred result.
        def read_events():
            with actor.condition:
                return list(actor.events)[diagnostic_start_index:]
        events = read_events()
        if wait_for_result:
            until = time.monotonic() + 2.0
            with actor.condition:
                while time.monotonic() < until:
                    events = list(actor.events)[diagnostic_start_index:]
                    actual_controls = {e.get("controlStableId") for e in events if e.get("event") == "unified-active-committed" and e.get("stableId") == diagnostic_original_id and e.get("stage") == "common-committed"}
                    if len(actual_controls) == 1 and any(e.get("event") == "unified-active-result" and e.get("controlStableId") in actual_controls for e in events):
                        break
                    actor.condition.wait(timeout=min(0.1, max(0, until - time.monotonic())))
        evidence.write("passive:actual-native-active-events", events)
        controls = {e.get("controlStableId") for e in events if e.get("event") == "unified-active-committed" and e.get("stableId") == diagnostic_original_id and e.get("stage") == "common-committed"}
        require(len(controls) <= 1 and all(type(value) is int and value >= 0 for value in controls), "single-exact-native-C", "actual exact-M diagnostics identify multiple or malformed controls")
        if controls:
            observed = next(iter(controls))
            require(diagnostic_control_id is None or diagnostic_control_id == observed, "stable-exact-native-C", "actual control correlation changed")
            diagnostic_control_id = observed
        if diagnostic_control_id is None:
            report["nativeRetirementDiagnostics"] = {"observation": "UNOBSERVED", "commonCommitted": False, "resultObserved": False, "events": []}
            save()
            return report["nativeRetirementDiagnostics"]
        evidence.write("private:exact-native-M-C", {"originalStableId": diagnostic_original_id, "controlStableId": diagnostic_control_id})
        selected = project_retirement_events(events, diagnostic_original_id, diagnostic_control_id)
        results = [e for e in selected if e["event"] == "unified-active-result"]
        require(len(results) <= 1, "single-exact-native-result", "multiple exact-control results observed")
        report["nativeRetirementDiagnostics"] = {"observation": "OBSERVED", "events": selected,
            "commonCommitted": any(e.get("stage") == "common-committed" and e.get("originalOpen") is True and e.get("freshAck") is True and e.get("bothReady") is True for e in selected),
            "resultObserved": bool(results), "resultOk": results[0]["ok"] if results else None,
            "firstRetirement": next((e for e in selected if e["event"] == "rtc-pump-ended"), "UNOBSERVED"),
            "causeConclusion": "UNQUALIFIED"}
        save()
        return report["nativeRetirementDiagnostics"]

    def save():
        (args.output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")

    def phase(name, work):
        nonlocal current
        current = name
        started = time.monotonic()
        try:
            value = work()
            evidence.write(name, value)
            report["phases"].append({"name": name, "ok": True,
                "phaseWallMs": round((time.monotonic() - started) * 1000, 3)})
            save()
            return value
        except BaseException as error:
            evidence.exception(name, error)
            remember_failure(report, error, name)
            report["phases"].append({"name": name, "ok": False,
                "phaseWallMs": round((time.monotonic() - started) * 1000, 3)})
            save()
            raise

    def sdk(operation, values=None):
        nonlocal current
        current = "sdk:" + operation
        evidence.write(current + ":request", values)
        setup_operations = {"start", "request_admission", "request_precode", "complete_admission",
            "open_member", "improve_path", "disable_native_rtc_resource_during_improvement", "sync_replica"}
        harness_timeout = 75 if (args.case in NEGATIVE_CASES or args.case in {"positive", "deadline-survival"}) and operation in setup_operations else 20
        if operation in setup_operations:
            require(time.monotonic() - case_started + harness_timeout < 150,
                "case-setup-reserve", "setup call would consume cleanup reserve")
        try:
            reply = phase(current, lambda: page.call(operation, values, timeout=harness_timeout))
        finally:
            if operation == "improve_path":
                try:
                    capture_native_diagnostics()
                except BaseException as diagnostic_error:
                    evidence.exception("passive-native-diagnostics", diagnostic_error)
                    remember_failure(report, diagnostic_error, "passive-native-diagnostics")
                    report["passed"] = False
                    save()
        report["sdkCalls"].append({"operation": operation, "ok": reply.get("ok") is True,
            "sdkCallerMs": reply.get("callerElapsedMs")})
        require(reply.get("ok") is True, "sdk-operation", "actual Browser SDK operation failed")
        return reply.get("result")

    def sdk_failure(operation, values=None):
        """Capture an expected SDK rejection without turning it into runner failure."""
        nonlocal current
        current = "sdk-failure:" + operation
        evidence.write(current + ":request", values)
        reply = phase(current, lambda: page.call(operation, values, timeout=75 if args.case in NEGATIVE_CASES and operation == "improve_path" else 20))
        reason = stable_failure_reason(reply)
        report["sdkCalls"].append({"operation": operation, "ok": reply.get("ok") is True,
            "expectedFailure": True, "failureClass": reply.get("failureClass"),
            "failureReason": reason, "sdkCallerMs": reply.get("callerElapsedMs")})
        require(reply.get("ok") is False, "candidate-failure", "active candidate unexpectedly committed")
        return reply

    def fetch_active_fault_snapshot():
        """Read <=16 unchanged actual stopped samples per existing local RPC."""
        offset, total, metadata = 0, None, None
        observations = []
        for _ in range(21): # Same existing320 bound: at most20 nonempty parts.
            part = sdk("active_fault_snapshot", {"offset":offset, "limit":16})
            require(isinstance(part, dict) and integer(part.get("totalCount"))
                    and part["totalCount"] <= 320 and integer(part.get("offset"))
                    and part["offset"] == offset and type(part.get("limit")) is int and part["limit"] == 16
                    and part.get("samplerRunning") is False,
                "snapshot-page-header", "actual stopped snapshot count/offset/limit differs")
            facts = {key:part.get(key) for key in ("injection","samplerRunning","sampleStopCalls","sampleErrors","sampleOverflow")}
            require(isinstance(facts["injection"], dict) and integer(facts["sampleStopCalls"])
                    and isinstance(facts["sampleErrors"], list) and type(facts["sampleOverflow"]) is bool,
                "snapshot-page-metadata", "actual stopped error/overflow metadata is malformed")
            if total is None:
                total, metadata = part["totalCount"], facts
            require(part["totalCount"] == total and facts == metadata,
                "snapshot-page-stability", "actual stopped snapshot metadata/total changed between parts")
            chunk = part.get("observations")
            require(isinstance(chunk, list) and all(isinstance(value, dict) for value in chunk)
                    and offset <= total and len(chunk) == min(16, total - offset),
                "snapshot-page-complete", "actual snapshot part is missing or out of order")
            end = offset + len(chunk)
            expected_next = end if end < total else None
            next_offset = part.get("nextOffset")
            require((expected_next is None and next_offset is None)
                    or (type(next_offset) is int and next_offset == expected_next and next_offset > offset),
                "snapshot-page-next", "actual next snapshot offset does not preserve complete order")
            observations.extend(chunk)
            if expected_next is None:
                require(len(observations) == total,
                    "snapshot-all-parts", "complete actual stopped snapshot count differs")
                return {**metadata, "observations":observations}
            offset = next_offset
        raise EvidenceFailure("snapshot-page-bound", "existing320 sample bound exceeded")

    def native(operation, expected, **values):
        request = {"op": operation, **values}
        evidence.write("native:" + operation + ":request", request)
        with native_command_lock:
            return phase("native:" + operation, lambda: actor.command(json.dumps(request), expected))

    def preserve_positive_failure():
        if args.case not in NEGATIVE_CASES or page is None or actor is None or report.get("positiveFailureObservationAttempted") or "first_stream" not in locals_for_failure:
            return
        report["positiveFailureObservationAttempted"] = True
        def observe(name, work):
            try:
                value = work()
                evidence.write("positive-failure:" + name, value)
                return value
            except BaseException as error:
                evidence.exception("positive-failure:" + name, error)
                report.setdefault("positiveFailureObservationErrors", []).append(name)
                return None
        member = observe("member-diagnostics", lambda: sdk("member_diagnostics"))
        objects = observe("rtc-objects", lambda: sdk("rtc_fixture_snapshot"))
        status = observe("native-status", lambda: native("status", "status")["status"])
        survived = observe("same-stream-sequence1", lambda: sdk("continuity_exchange", {"name": "member", "sequence": locals_for_failure.get("next_sequence", 1)}))
        pong = observe("pong", lambda: sdk("exchange", {"name": "member", "message": "PING"}))
        late_objects = observe("rtc-objects-after-continuity", lambda: sdk("rtc_fixture_snapshot"))
        late_status = observe("native-status-after-continuity", lambda: native("status", "status")["status"])
        member = member if isinstance(member, dict) else {}
        progress = (member.get("bootstrap") or {}).get("activeRtcPrepareProgress") or {}
        stages = {"control-stream-open", "answerer-read-offer", "answerer-validate-offer", "rtc-owner-create", "answerer-set-remote-route", "offerer-signaling", "offerer-validate-answer", "answerer-signaling", "data-channel-open", "data-channel-metadata", "lease-activate", "pump-start", "remote-route", "data-path-open", "data-path-present", "probe-start", "probe-ack", "qualification", "stream-finish", "prepared-path-return"}
        original = locals_for_failure["first_stream"]
        prior = ((locals_for_failure.get("before") or {}).get("bootstrap") or {}).get("activeRtcPrepareProgress")
        stage_valid = progress.get("version") == 1 and progress.get("stage") in stages and progress.get("role") in {"offerer", "answerer"} and type(progress.get("completed")) is bool
        stage_changed = stage_valid and progress != prior
        survived = survived if isinstance(survived, dict) else {}
        report["positiveFailureObservation"] = {
            "stageObservation": "OBSERVED" if stage_changed else "UNOBSERVED",
            "lastStage": progress.get("stage") if progress.get("stage") in stages else None,
            "stageRole": progress.get("role") if progress.get("role") in {"offerer", "answerer"} else None,
            "stageCompleted": progress.get("completed") if type(progress.get("completed")) is bool else None,
            "stageLimit": "missing/unchanged marker is UNOBSERVED; raw exact member snapshot retained privately",
            "ordinaryHandle": member.get("ordinaryHandle") is True,
            "memberAuthorized": member.get("memberAuthorized") is True,
            "expectedPeerMatched": member.get("expectedPeerMatched") is True,
            "originalStreamSequence": survived.get("sequence"),
            "originalStreamAcknowledged": survived.get("sequence") == locals_for_failure.get("next_sequence", 1),
            "sameStream": survived.get("streamId") == original.get("streamId"),
            "sameStableId": survived.get("stableUnchanged") is True,
            "sameBinding": survived.get("bindingUnchanged") is True and survived.get("native", {}).get("bindingUnchanged") is True,
            "pong": pong == "PONG",
            "rtcObjects": public_cleanup(objects) if isinstance(objects, dict) else None,
            "nativeStatus": public_cleanup(status) if isinstance(status, dict) else None,
            "rtcObjectsAfterContinuity": public_cleanup(late_objects) if isinstance(late_objects, dict) else None,
            "nativeStatusAfterContinuity": public_cleanup(late_status) if isinstance(late_status, dict) else None,
        }
        try:
            capture_native_diagnostics()
        except BaseException as diagnostic_error:
            evidence.exception("positive-failure:native-diagnostics", diagnostic_error)
            remember_failure(report, diagnostic_error, "positive-failure:native-diagnostics")
        save()

    locals_for_failure = {}

    def retire_native(name, final=False):
        from check_unified_browser_native import close_native
        try:
            value = close_native(actor, remove_directory=final)
            report.setdefault("nativeGenerations", []).append({"name": name, "cleanup": public_cleanup(value)})
            return value
        finally:
            evidence.write(name + ":all-native-evidence", native_snapshot(actor))

    try:
        inputs, facility = phase("exact-input-guard", lambda: validate_inputs(args))
        report["inputs"] = {key: {name: value for name, value in values.items() if name != "binary"}
            for key, values in inputs.items()}
        from host_native_runtime import HostNativeActor
        from android_pair_runtime import BrowserPage, Device

        native_probe_bridge = NativeProbeBridge() if args.case == "mio80-live-resource-disable" else None

        class MaintainedPage(BrowserPage):
            def __init__(self, device, package, prelude):
                super().__init__(device, package, prelude=prelude, environment_rtc_probe=False)
                original = self.server.RequestHandlerClass
                owned_page = self

                class Handler(original):
                    def do_POST(self):
                        if resource_permission_bridge.reply(self, owned_page):
                            return
                        return super().do_POST()

                    def do_GET(self):
                        if native_probe_bridge is not None and native_probe_bridge.reply(self, owned_page):
                            return
                        if self.path == "/android-pair-page.js":
                            return self.send_bytes(PAGE.read_bytes(), "text/javascript; charset=utf-8")
                        return super().do_GET()
                self.server.RequestHandlerClass = Handler

        actor = phase("owned-host-actor", HostNativeActor)
        phase("deploy-provisioner", lambda: actor.deploy(inputs["provider"]["binary"], "provider", executable=True))
        facility_path = phase("deploy-facility", lambda: actor.deploy(args.facility, "facility.json"))
        provider_profile = phase("deploy-provider-profile", lambda: actor.deploy(args.provider_profile, "provider-ice.json"))
        store = actor.directory + "/saved-provider"
        provider_args = [store, facility_path, provider_profile,
            "--allow-unqualified-preparation", "--rtc-disabled"]
        if not controlled_loopback:
            provider_args[4:4] = ["--relay", args.relay]
        phase("start-real-provisioner", lambda: actor.start(provider_args,
            stop_command=json.dumps({"op": "stop"})))
        ready = phase("provisioner-ready", lambda: actor.wait_event("ready"))
        require(ready.get("normalPublicTrust") is True, "normal-trust", "provisioner normal trust was not observed")
        if host_browser:
            from host_browser_runtime import HostBrowserPage

            class MaintainedHostPage(HostBrowserPage):
                def __init__(self, package, executable, prelude):
                    super().__init__(package, executable, prelude=prelude)
                    original = self.server.RequestHandlerClass
                    owned_page = self

                    class Handler(original):
                        def do_POST(inner_self):
                            if resource_permission_bridge.reply(inner_self, owned_page):
                                return
                            return super().do_POST()

                        def do_GET(inner_self):
                            if native_probe_bridge is not None and native_probe_bridge.reply(inner_self, owned_page):
                                return
                            if inner_self.path == "/android-pair-page.js":
                                return inner_self.send_bytes(PAGE.read_bytes(), "text/javascript; charset=utf-8")
                            return super().do_GET()
                    self.server.RequestHandlerClass = Handler

            if not args.host_chrome:
                raise RuntimeError("Host Browser mode requires an owned Host Chrome executable")
            page = phase("owned-host-browser-page", lambda: MaintainedHostPage(
                args.package, args.host_chrome, rtc_prelude()))
            phase("host-browser-page-ready", page.start)
        else:
            browser_device = phase("pixel-device", lambda: Device("Pixel_6a", "usb"))
            page = phase("owned-pixel-page", lambda: MaintainedPage(browser_device, args.package, rtc_prelude()))
            phase("pixel-page-ready", page.start)

        def configuration(target, data=False, relay=None):
            return {"relays": [relay or target.get("permittedRelay") or args.relay] if data else [], "disabled": [], "irc": [],
                "mqtt": [facility], "rtc_enabled": data,
                "rtc_ice": json.loads(args.browser_profile.read_text()) if data
                    else {"stun_servers": [], "local_bind_addresses": [], "candidate_addresses": []},
                "contact_routes": [{"contact": target["id"], "facility": facility["logical_id"],
                    "address": target["route"]}]}

        first = sdk("start", {"configuration": configuration(ready)})
        browser_id = first["id"]
        require(browser_id != ready["id"], "independent-identities", "Native and Browser identities are not independent")
        require(sdk("request_admission", {"invitation": ready["invitation"]}) == "AwaitingApproval",
            "actual-admission", "actual manual admission request phase differs")
        requests = native("admission-status", "admission-status")
        matches = [value for value in requests["requests"] if value["applicant"] == browser_id]
        require(len(matches) == 1, "exact-applicant", "exact authenticated applicant request unavailable")
        native("approve-request", "approved", applicant=browser_id, request=matches[0]["request"])
        require(sdk("request_admission", {"invitation": ready["invitation"]}) == "PendingMaterials",
            "authorization", "approval did not reach its saved authorization boundary")
        require(sdk("complete_admission") == "Complete", "materials", "actual material completion failed")
        materials = sdk("material_receipt")
        require(materials.get("stage") == "Complete" and materials.get("authorization_saved") is True
            and materials.get("secret_saved") is True, "materials", "actual durable Complete is missing")
        stopped = sdk("stop")
        require(stopped.get("cleanupSucceeded") is True and stopped.get("networkClosed") is True
            and stopped.get("networkFreed") is True and integer(stopped.get("ownedMemberHandles"))
            and stopped["ownedMemberHandles"] == 0, "provisioning-browser-retired",
            "provisioning Browser generation did not close/free cleanly")
        value = phase("provisioner-natural-retirement", lambda: retire_native("provisioner"))
        require(native_retired(value, False), "provider-cleanup", "provisioner did not retire naturally")

        phase("deploy-maintained-fixture", lambda: actor.deploy(inputs["maintained"]["binary"], "maintained", executable=True))
        native_profile = phase("deploy-native-profile", lambda: actor.deploy(args.native_profile, "native-ice.json"))
        maintained_args = [store, facility_path, native_profile, args.relay, browser_id,
            "--private-packet-witness", str(args.output / "packet-witness.jsonl")]
        if controlled_loopback:
            maintained_args.append("--controlled-loopback")
        phase("start-maintained-node", lambda: actor.start(maintained_args,
            stop_command=json.dumps({"op": "stop"})))
        maintained = phase("maintained-ready", lambda: actor.wait_event("ready"))
        require(maintained.get("maintainedNode") is True and maintained.get("rtcEnabled") is True
            and maintained["id"] == ready["id"],
            "maintained-owner", "actual saved maintained Node identity/resources differ")
        if controlled_loopback:
            require(maintained.get("controlledLoopback") is True, "controlled-relay-owner",
                "maintained fixture did not report its owned loopback relay")
            effective_relay = maintained.get("permittedRelay")
            require(isinstance(effective_relay, str) and urlsplit(effective_relay).scheme == "http",
                "controlled-relay-url", "owned loopback relay URL is unavailable")
        else:
            require(maintained.get("permittedRelay") == args.relay and maintained.get("publicDataRelayEnabled") is True,
                "existing-relay-contact", "the existing permitted contact facility differs")
        reopened = sdk("start", {"configuration": configuration(maintained, True, effective_relay)})
        require(reopened["id"] == browser_id, "browser-reopen", "saved Browser identity changed")
        reopened_materials = sdk("material_receipt")
        require(all(reopened_materials.get(key) == materials.get(key)
            for key in ("id", "request", "authorization", "stage", "authorization_saved", "secret_saved")),
            "material-reopen", "durable membership/materials changed on reopen")
        initial_open = sdk("open_member", {"target": maintained["id"]})
        report["initialOpenSetupClock"] = initial_open.get("setupClock")
        native_member = phase("real-native-member", lambda: actor.wait_event("member-released"))
        before = sdk("member_diagnostics")
        ordinary = native_member["ordinary"]
        require(native_member.get("identityMatched") is True and ordinary.get("memberAuthorized") is True
            and before.get("expectedPeerMatched") is True and before.get("memberAuthorized") is True
            and ordinary.get("alpn") == before.get("alpn") == MEMBER_ALPN,
            "current-member", "ordinary M failed actual current identity/member gates")
        native_common = phase("real-native-fresh-ack", lambda: actor.wait_event(lambda value:
            value.get("event") == "unified-path-ready" and value.get("purposeCode") == 1
            and value.get("stableId") == ordinary["stableId"]))
        require(native_common.get("freshAck") is True and native_common.get("version") == 1
            and native_common.get("role") == "responder" and native_common.get("stableUnchanged") is True
            and native_common.get("bindingUnchanged") is True
            and before.get("coordinator", {}).get("freshAck") is True
            and before.get("coordinator", {}).get("version") == 1
            and before.get("coordinator", {}).get("role") == "initiator",
            "original-ack", "actual original-M fresh ACK missing")
        # Establish a real stream, improve M, then retire its old relay paths.
        # The contact facility stays ready without a validated backup DATA path on M.
        for paths in (ordinary.get("paths",[]),before.get("paths",[])):
            require(any(p.get("isRelay") is True and p.get("selected") is True for p in paths)
                and not any(p.get("isRtc") for p in paths),
                "initial-real-relay-M", "original stream must begin on its actual permitted relay")
        first_stream = sdk("continuity_start")
        require(first_stream.get("sequence")==0 and first_stream.get("stableUnchanged") is True
            and first_stream.get("bindingUnchanged") is True and first_stream.get("native",{}).get("bindingUnchanged") is True
            and first_stream.get("localStableId")==before["coordinator"]["stableId"]
            and first_stream.get("native",{}).get("stableId")==ordinary["stableId"]
            and first_stream.get("native",{}).get("payload")=="original stream PING 0",
            "actual-original-stream0", "initial same-M stream/binding did not match")
        improved = sdk("improve_path",{"name":"member","callerMs":60000})
        require(improved.get("outcome")=="committed" and improved.get("stableId")==first_stream["localStableId"],
            "initial-actual-RTC", "initial RTC was not committed on original M")
        active_view = sdk("member_diagnostics")
        require_active_commit_proof(active_view,first_stream["localStableId"])
        phase("initial-native-common-committed",lambda:actor.wait_event(lambda e:
            e.get("event")=="unified-active-committed" and e.get("stage")=="common-committed"
            and e.get("stableId")==ordinary["stableId"] and e.get("originalOpen") is True
            and e.get("freshAck") is True and e.get("bothReady") is True))
        retired = native("retire-original-relay-paths","original-relay-paths-retired",originalStableId=ordinary["stableId"])
        require(integer(retired.get("actualCloseCalls"),1) and retired.get("after",{}).get("open") is True,
            "actual-old-relay-path-close", "public Path.close did not retire the old original-M relay paths")
        until = time.monotonic()+1.0
        while True:
            before_cut_browser = sdk("member_diagnostics")
            native_baseline = native("status","status")["status"]
            path_views = (before_cut_browser.get("paths",[]),native_baseline.get("ordinary",{}).get("paths",[]))
            sole = all(len(paths)==1 and paths[0].get("isRtc") is True and paths[0].get("selected") is True
                and not any(p.get("isRelay") or p.get("isIp") or p.get("isMemberData") for p in paths)
                for paths in path_views)
            if sole: break
            require(time.monotonic()<until,"no-prebuilt-backup-after-retirement", "a second original-M DATA path remained after retirement")
            time.sleep(0.02)
        sole_stream = sdk("continuity_exchange",{"name":"member","sequence":1})
        require(sole_stream.get("sequence")==1 and sole_stream.get("streamId")==first_stream["streamId"]
            and sole_stream.get("stableUnchanged") is True and sole_stream.get("bindingUnchanged") is True
            and sole_stream.get("native",{}).get("stableId")==ordinary["stableId"]
            and sole_stream.get("native",{}).get("bindingUnchanged") is True
            and sole_stream.get("native",{}).get("payload")=="original stream PING 1",
            "sole-RTC-original-stream1", "original stream did not actually carry sequence1 on the sole RTC path")
        objects = sdk("rtc_fixture_snapshot");physical = sdk("physical_rtc_snapshot")
        require(objects.get("openPeerConnections")==objects.get("openDataChannels")==physical.get("livePeers")==1
            and physical.get("trackOverflow")==physical.get("eventOverflow")==physical.get("observationErrors")==0
            and native_baseline.get("activeRtcAssociations")==native_baseline.get("activeRtcPumps")==1,
            "sole-actual-RTC-owner", "actual sole RTC owners or coverage unproven")
        evidence.write("private:original-M-and-stream-baselines",{"browser":before_cut_browser,"native":native_baseline,
            "stream":first_stream,"soleRtcStream":sole_stream,"physical":physical,"oldRelayRetirement":retired})
        report["beforePhysicalCut"]={"soleDataPathBoth":True,"prebuiltBackupDataPaths":0,
            "actualOldRelayPathCloseCalls":retired["actualCloseCalls"],"actualOpenBrowserPCs":1,
            "actualNativeAssociations":1,"originalBindingMatchesBoth":True,"originalStreamSequences":[0,1],
            "sequence1OnSoleRTC":True,"contactFacilityAlreadyRunning":True,"dataPath":"RTC",
            "originalQuicRoles":{"browser":"client","native":"server"}}
        cut = sdk("physical_close_single_owned_rtc")
        require(cut.get("closeReturned") is True and cut.get("before", {}).get("connectionState") == "connected"
            and cut.get("after", {}).get("connectionState") == "closed",
            "actual-owned-PC-physical-close", "normal close of actual sole connected PC unobserved")
        report["physicalCut"] = {"normalActualPCCloseReturned": True, "before": "connected", "after": "closed",
            "closeCallMs": cut.get("closeSettledAtMonotonicMs", 0) - cut.get("closeCalledAtMonotonicMs", 0),
            "memberOrStreamCloseUsedAsFault": False, "permissionChangedAsFault": False}
        after_cut = sdk("member_diagnostics")
        evidence.write("private:after-physical-cut-original", after_cut)
        report["originalOpenImmediatelyAfterCut"] = after_cut.get("ordinaryHandle") is True
        fault_observation_started = time.monotonic()
        until = fault_observation_started+1.0
        while True:
            retired_carrier = native("status","status")["status"]
            if retired_carrier.get("activeRtcAssociations")==retired_carrier.get("activeRtcPumps")==0: break
            require(time.monotonic()<until,"old-native-RTC-natural-retirement", "old native RTC owner did not retire after the physical cut")
            time.sleep(0.02)
        require(retired_carrier.get("ordinary",{}).get("open") is True,
            "original-alive-before-repair", "the original native M terminated before explicit repair")
        report["physicalCarrierRetirementBeforeCaller"]={"naturalNativeAssociations":0,"naturalNativePumps":0,
            "originalNativeMStillOpen":True,"observationWallMs":round((time.monotonic()-fault_observation_started)*1000,3),
            "separateFromRepairCallerClock":True}
        with actor.condition:
            diagnostic_start_index = len(actor.events)
        diagnostic_original_id = ordinary["stableId"]
        call_started = time.monotonic()
        if args.case=="mio81-native-trigger":
            result = native("repair-original-member","original-member-repair-result",
                originalStableId=ordinary["stableId"],callerMs=60000)
            same_repair_id = result.get("originalStableId")==ordinary["stableId"]
            caller_elapsed = result.get("sdkCallerMs")
        else:
            result = sdk("recover_path", {"name": "member", "callerMs": 60000})
            same_repair_id = result.get("stableId")==first_stream["localStableId"]
            caller_elapsed = result.get("elapsedMs")
        call_returned = time.monotonic()
        evidence.write("private:actual-current-public-repair-result", result)
        after_repair = sdk("member_diagnostics")
        native_after_repair = native("status", "status")["status"]
        evidence.write("private:actual-after-repair", {"browser":after_repair,"native":native_after_repair})
        capture_native_diagnostics()
        report["currentPublicRepair"] = {"outcome": result.get("outcome"),
            "elapsedMs": caller_elapsed, "callerBudgetMs": result.get("callerBudgetMs"),
            "sdkReturnWallMs": round((call_returned-call_started)*1000,3),
            "controlAlpn": result.get("controlAlpn"),
            "sameStableId": same_repair_id,
            "actualOriginalOpenAfterRepair": after_repair.get("ordinaryHandle") is True}
        progress = (after_repair.get("bootstrap") or {}).get("activeRtcPrepareProgress") or {}
        permitted_stages = {"control-stream-open","answerer-read-offer","answerer-validate-offer","rtc-owner-create",
            "answerer-set-remote-route","offerer-signaling","offerer-validate-answer","answerer-signaling",
            "data-channel-open","data-channel-metadata","lease-activate","pump-start","remote-route",
            "data-path-open","data-path-present","probe-start","probe-ack","qualification","stream-finish","prepared-path-return"}
        report["actualBrowserRepairProgress"] = {"stage":progress.get("stage") if progress.get("stage") in permitted_stages else None,
            "role":progress.get("role") if progress.get("role") in {"offerer","answerer"} else None,
            "completed":progress.get("completed") if type(progress.get("completed")) is bool else None,
            "limit":"missing marker is unobserved; raw snapshots remain private"}
        require(result.get("outcome") == "recovered" and same_repair_id
            and result.get("controlAlpn") == "miopunch/session/improve/1",
            "same-session-common-repair", "actual common repair did not recover the original M within the existing cap")
        require(integer(caller_elapsed) and caller_elapsed <= 3100,
            "repair-effective-cap", "actual SDK repair exceeded the existing 3s cap allowance")
        require_active_commit_proof(after_repair,first_stream["localStableId"])
        native_committed = phase("repair-native-common-committed",lambda:actor.wait_event(lambda e:
            e.get("event")=="unified-active-committed" and e.get("stage")=="common-committed"
            and e.get("stableId")==ordinary["stableId"] and e.get("originalOpen") is True
            and e.get("freshAck") is True and e.get("bothReady") is True))
        original_after = native_after_repair.get("ordinary") or {}
        require(original_after.get("stableUnchanged") is True and original_after.get("bindingUnchanged") is True
            and original_after.get("stableId")==ordinary["stableId"] and original_after.get("open") is True,
            "repair-native-original-binding", "native repair did not retain exact original M/binding")
        for sequence in (2,3):
            reply = sdk("continuity_exchange", {"name":"member","sequence":sequence})
            require(reply.get("sequence")==sequence and reply.get("streamId")==first_stream["streamId"]
                and reply.get("stableUnchanged") is True and reply.get("bindingUnchanged") is True
                and reply.get("localStableId")==first_stream["localStableId"]
                and reply.get("native",{}).get("stableId")==ordinary["stableId"]
                and reply.get("native",{}).get("bindingUnchanged") is True
                and reply.get("native",{}).get("payload")=="original stream PING "+str(sequence),
                "repair-original-stream-continuity", "actual later payload did not arrive on same original M/stream")
        # Successful data ownership outlives this explicit repair's3s clock.
        # A separate real stream operation verifies that expired/cancelled C
        # construction state cannot retire its already committed original-M RTC.
        after_repair_return=time.monotonic()
        time.sleep(3.1)
        late_reply=sdk("continuity_exchange",{"name":"member","sequence":4})
        require(late_reply.get("streamId")==first_stream["streamId"]
            and late_reply.get("stableUnchanged") is True and late_reply.get("bindingUnchanged") is True
            and late_reply.get("localStableId")==first_stream["localStableId"]
            and late_reply.get("native",{}).get("stableId")==ordinary["stableId"]
            and late_reply.get("native",{}).get("bindingUnchanged") is True
            and late_reply.get("native",{}).get("payload")=="original stream PING 4",
            "successful-repair-outlives-operation-cap","actual original stream failed after the construction caller budget ended")
        report["afterRepairCapBusinessEvidence"]={"originalRepairCapMs":3000,
            "observedAfterRepairReturnMs":round((time.monotonic()-after_repair_return)*1000,3),
            "sameOriginalMAndBindingBoth":True,"sameOriginalStreamSequence":4,
            "actualLaterBusinessDelivered":True,"newRepairOrConnectInvoked":False}
        physical_after = sdk("physical_rtc_snapshot")
        require(physical_after.get("livePeers")==1 and physical_after.get("closeInvocations")==1
            and physical_after.get("created",0)>physical.get("created",0),
            "new-physical-carrier-same-stream", "a distinct actual replacement PC was not observed")
        report["browserRecoveryEvidence"] = {"sameOriginalMAndBindingBoth":True,"sameOriginalStreamSequences":[0,1,2,3,4],
            "freshAckBoth":True,"commonReadyCommittedBoth":True,"replacementPCObserved":True,
            "originalQuicRoles":{"browser":"client","native":"server"},"CInitiator":"native" if args.case=="mio81-native-trigger" else "browser",
            "prebuiltBackupDataPaths":0,"physicalCutOnlyOwnedPC":True}
        report["passed"] = True
        raise CaseCompleted()
    except CaseCompleted:
        pass
    except BaseException as error:
        evidence.exception(current, error)
        remember_failure(report, error, current)
        # No post-fault continuity write: preserve raw first failure and retire actors.
    finally:
        try:
            capture_native_diagnostics()
        except BaseException as diagnostic_error:
            evidence.exception("pre-cleanup:native-diagnostics", diagnostic_error)
            remember_failure(report, diagnostic_error, "pre-cleanup:native-diagnostics")
            report["passed"] = False
        if page is not None:
            try:
                value = phase("final-browser-cleanup", page.close)
                report["cleanup"]["browser"] = public_cleanup(value)
                report["queuedPrivateObserverRetirement"] = value.get("sdk",{}).get("result",{}).get("privatePacketObserverRetirement")
                relay_tail = browser_terminal_socket_tail(
                    value, facility.get("wss_url"), effective_relay
                )
                if relay_tail is not None:
                    report["cleanup"]["browserRelayTail"] = relay_tail
                require(
                    browser_retired(value, facility.get("wss_url"), effective_relay),
                    "browser-cleanup",
                    "actual Browser cleanup samples are incomplete",
                )
                physical_retirement = value.get("sdk", {}).get("result", {}).get("physicalCarrierCutRetirement", {})
                require(physical_retirement.get("restored") is True and physical_retirement.get("factoryRestored") is True
                    and physical_retirement.get("attachedListeners") == 0 and physical_retirement.get("livePeers") == 0,
                    "physical-cut-observer-retired", "owned physical observer/factory did not retire naturally")
                report["cleanup"]["physicalCutObserver"] = physical_retirement
                report["cleanup"]["browserQualified"] = True
            except BaseException as error:
                evidence.exception("final-browser-cleanup", error)
                remember_failure(report, error, "final-browser-cleanup")
                report["passed"] = False
                report["cleanup"]["browserFailed"] = True
        if actor is not None:
            try:
                value = phase("final-native-cleanup", lambda: retire_native("final", True))
                report["cleanup"]["native"] = public_cleanup(value)
                require(native_retired(value), "native-cleanup", "actual Native cleanup samples are incomplete")
                report["cleanup"]["nativeQualified"] = True
            except BaseException as error:
                evidence.exception("final-native-cleanup", error)
                remember_failure(report, error, "final-native-cleanup")
                report["passed"] = False
                report["cleanup"]["nativeFailed"] = True
        if report.get("queuedPrivateProofPendingFinalCapture"):
            try:
                phase("queued-independent-final-private-capture",lambda: verify_dequeued_private_capture(args,report,actor,evidence))
            except BaseException as error:
                evidence.exception("queued-independent-final-private-capture",error)
                remember_failure(report,error,"queued-independent-final-private-capture")
                report["passed"]=False
        if inputs:
            try:
                current_inputs, _ = phase("final-input-guard", lambda: validate_inputs(args))
                require(current_inputs == inputs, "stable-artifacts", "source/artifact bindings changed during the case")
                report["inputsStable"] = True
            except BaseException as error:
                evidence.exception("final-input-guard", error)
                remember_failure(report, error, "final-input-guard")
                report["passed"] = False
                report["inputsStable"] = False
        try:
            evidence.write("all-owned-native-generations", native_snapshot(actor))
            evidence.write("all-pixel-commands", browser_device.commands if browser_device else None)
            evidence.write("pixel-terminal", page.terminal if page else None)
            report["privateEvidence"] = evidence.close()
        except BaseException as error:
            remember_failure(report, error, "private-evidence-finalization")
            report["passed"] = False
        report["caseElapsedMs"] = round((time.monotonic() - case_started) * 1000, 3)
        if report["caseElapsedMs"] >= 180000:
            remember_failure(report, EvidenceFailure("case-total-budget", "case including cleanup exceeded three minutes"), "case-total-budget")
            report["passed"] = False
        report["setupDeadlineSurvivalQualified"] = (args.case == "deadline-survival"
            and report.get("setupDeadlineSurvival", {}).get("actualProof") is True
            and report.get("positiveQualified") is True and report["passed"] is True
            and "firstFailure" not in report and report.get("inputsStable") is True
            and report["cleanup"].get("browserQualified") is True
            and report["cleanup"].get("nativeQualified") is True)
        report["m80DequeuedBusinessQualified"] = (args.case in NEGATIVE_CASES
            and report.get("m80DequeuedBusinessEvidence") is True and report["passed"] is True
            and "firstFailure" not in report and report.get("inputsStable") is True
            and report["cleanup"].get("browserQualified") is True
            and report["cleanup"].get("nativeQualified") is True
            and report["cleanup"].get("activeFaultTimersQualified") is True)
        report["fullGoalQualified"] = False
        if "firstFailure" in report:
            report["passed"] = False
        report["commandExit"] = 0 if report["passed"] else 1
        report["firstRedQualified"] = (report["firstRedCaptured"] and report.get("inputsStable") is True
            and report["cleanup"].get("browserQualified") is True
            and report["cleanup"].get("nativeQualified") is True)
        report["physicalOsTuple"] = {"nativeSdkSocket": None, "browserPeerIpPort": None, "uidRouting": None}
        save()
    return report["commandExit"]


def development_checks():
    # Real filesystem/private error retention checks; no mock SDK success matrix.
    with tempfile.TemporaryDirectory(prefix="mio77-evidence-check-") as name:
        directory = Path(name)
        directory.chmod(0o700)
        evidence = PrivateEvidence(directory)
        report = {}
        first = RuntimeError("PRIVATE_TEST_ERROR_NEVER_PUBLIC")
        evidence.exception("outside-phase", first)
        remember_failure(report, first, "outside-phase")
        pinned = dict(report["firstFailure"])
        later = EvidenceFailure("cleanup", "cleanup observation absent")
        evidence.exception("cleanup", later)
        remember_failure(report, later, "cleanup")
        require(report["firstFailure"] == pinned, "first-error-check", "cleanup replaced the first failure")
        require("PRIVATE_TEST_ERROR_NEVER_PUBLIC" not in json.dumps(report),
            "public-error-check", "private exception escaped into public evidence")
        final = evidence.close()
        require(evidence.path.stat().st_mode & 0o777 == 0o600
                and "PRIVATE_TEST_ERROR_NEVER_PUBLIC" in evidence.path.read_text()
                and final["recordCount"] == 2,
            "private-error-check", "exact private first/later errors were not retained")
        require(not integer(False) and not integer(-1) and integer(0)
            and not object_counts_retired({"openPeerConnections": False, "openDataChannels": 0,
            "openWebSockets": 0}), "typed-count-check", "unknown/bool count qualified as zero")
        filtered = public_cleanup({"activeAssociations": 0, "retiredMemberReports": {
            "PRIVATE_LOCAL_M_ID": {"privateKey": "PRIVATE_TEST_ERROR_NEVER_PUBLIC"}},
            "errors": [{"failure": "PRIVATE_TEST_ERROR_NEVER_PUBLIC"}]})
        require(filtered == {"activeAssociations": 0, "errorCount": 1},
            "cleanup-projection-check", "opaque/private cleanup detail escaped the public projection")
    return {"scope": "actual private evidence/immutable first failure/count boundaries only",
        "sdkRun": False, "deviceOperation": False, "publicNetworkOperation": False,
        "private0600": True, "firstFailurePreserved": True, "boolCountsRejected": True,
        "privateFirstAndCleanupErrorsRetained": True, "temporaryDirectoryRemoved": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--development-checks", action="store_true")
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--case", choices=tuple(sorted(NEGATIVE_CASES)), required=True)
    parser.add_argument("--relay", default="https://aps1-1.relay.n0.iroh.link./")
    parser.add_argument("--controlled-loopback", action="store_true")
    parser.add_argument("--host-chrome", type=Path)
    for name in ("runtime-freeze", "provider-build-receipt", "maintained-build-receipt",
                 "wasm-build-receipt", "package", "facility", "provider-profile",
                 "native-profile", "browser-profile", "output"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--runtime-freeze-sha256")
    args = parser.parse_args()
    if args.development_checks:
        require(not args.execute, "offline-only", "offline checks cannot execute an SDK")
        print(json.dumps(development_checks(), indent=2))
        return 0
    require(args.execute and all(getattr(args, name) for name in (
        "runtime_freeze", "runtime_freeze_sha256", "provider_build_receipt", "maintained_build_receipt",
        "wasm_build_receipt", "package", "facility", "provider_profile", "native_profile", "browser_profile", "output")),
        "root-inputs", "root's exact freeze/artifacts/profiles and explicit execution are required")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
