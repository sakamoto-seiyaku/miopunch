#!/usr/bin/env python3
"""M77 fixture-only relay continuity feedback; Root owns actual execution.

Import and --development-checks start no SDK, compiler, device or network.
`--case network-failure` requires an actual RTC peer creation/retirement and
fails qualification when the attempt dies before creating that peer.  The
single `causal-witness` case keeps the same positive empty-ICE fixture but
expects the current active C to fail, then checks the exact original M's
bounded preparation witness and continuity.
"""
import argparse
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
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / "browser-probe/harness/mio77-maintained-page.js"
MEMBER_ALPN = "miopunch/session/member/1"


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
                if callbacks_after != 4:
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
    """Graph-generic selected SDK proof; runtime-only files may evolve independently."""
    build = json.loads(receipt.read_text())
    require(type(build.get("exit")) is int and build["exit"] == 0
            and build.get("qualifiedForFrozenRun") is True,
        "successful-build", "an exact successful source-frozen build is required")
    before = json.loads((receipt.parent / "sources-before.json").read_text())
    after = json.loads((receipt.parent / "sources-after.json").read_text())
    digest = hashlib.sha256(json.dumps(after, sort_keys=True).encode()).hexdigest()
    require(before == after and digest == build.get("sourceDigestBefore") == build.get("sourceDigestAfter"),
        "captured-build-source", "captured build source/lock snapshots differ")
    selected = json.loads((receipt.parent / "selected-dependency-inputs.json").read_text())
    require(selected.get("inputs") and selected.get("copiedRepositoryInputs") and selected.get("depfiles"),
        "selected-input-evidence", "actual compiler input/copy/depfile evidence is missing")
    require(len(selected["inputs"]) == build.get("actualSelectedInputCount")
            and len(selected["copiedRepositoryInputs"]) == build.get("actualSelectedRepositoryInputCount"),
        "selected-input-counts", "compiler input/copy counts differ from the actual build receipt")
    for name, expected in selected["inputs"].items():
        require(Path(name).is_file() and sha(name) == expected,
            "selected-compiler-source", "an actual selected SDK source changed")
    for name, expected in selected["copiedRepositoryInputs"].items():
        require(sha(receipt.parent / "selected-source-copies" / name) == expected,
            "selected-source-copy", "a selected compiler source copy differs")
    parsed = set()
    for name, expected in selected["depfiles"].items():
        original = Path(name)
        require(original.is_relative_to(ROOT / "target"), "selected-depfile-location",
            "a captured compiler depfile is outside the shared target")
        captured = receipt.parent / "depfiles" / original.relative_to(ROOT / "target")
        require(sha(captured) == expected,
            "selected-depfile", "a retained compiler depfile differs")
        line = captured.read_text().replace("\\\n", " ").splitlines()[0]
        require(": " in line, "depfile-format", "retained compiler depfile format differs")
        for source in shlex.split(line.split(": ", 1)[1]):
            path = Path(source)
            path = (path if path.is_absolute() else ROOT / path).resolve()
            if path.is_file():
                parsed.add(str(path))
    require(parsed == {str(Path(path).resolve()) for path in selected["inputs"]},
        "actual-depfile-input-set", "actual retained depfile inputs differ from the selected manifest")
    return {"actualSelectedInputCount": len(selected["inputs"]),
        "selectedRepositoryCopies": len(selected["copiedRepositoryInputs"]),
        "capturedDepfileCount": len(selected["depfiles"]), "allCompiledSourcesMatch": True,
        "inactiveCurrentSourceDeltas": {}, "selectedManifestSha256": sha(receipt.parent / "selected-dependency-inputs.json")}


def validate_inputs(args):
    controlled_loopback = getattr(args, "controlled_loopback", False)
    positive = getattr(args, "case", "network-failure") in {"positive", "causal-witness"}
    require(not positive or controlled_loopback, "positive-mode", "positive M77 feedback requires the owned Host loopback fixture")
    if controlled_loopback:
        require(args.host_chrome is not None and args.host_chrome.is_file()
                and os.access(args.host_chrome, os.X_OK),
                "host-browser-input", "an owned executable Host Chrome is required")
    freeze = json.loads(args.runtime_freeze.read_text())
    require(sha(args.runtime_freeze) == args.runtime_freeze_sha256,
        "runtime-freeze-hash", "runtime freeze differs from root's selected hash")
    require(isinstance(freeze.get("files"), dict) and freeze["files"],
        "runtime-file-pins", "exact runtime file pins are required")
    for name, expected in freeze["files"].items():
        path = Path(name) if name.startswith("/") else ROOT / name
        require(sha(path) == expected, "runtime-input", "a frozen runtime input changed")
    required = [Path(__file__).resolve(), PAGE, ROOT / "src/bin/unified_maintained_story.rs",
        ROOT / "scripts/host_native_runtime.py", ROOT / "scripts/android_pair_runtime.py",
        ROOT / "browser-probe/harness/rtc-capability-fixture.mjs",
        ROOT / "scripts/check_unified_browser_native.py", ROOT / "scripts/wan_environment_preflight.py",
        args.facility, args.provider_profile, args.native_profile, args.browser_profile,
        args.provider_build_receipt, args.maintained_build_receipt, args.wasm_build_receipt]
    if controlled_loopback:
        required.extend([
            ROOT / "scripts/host_browser_runtime.py",
            ROOT / "browser-probe/harness/mio75-host-browser.mjs",
            ROOT / "browser-probe/harness/mio75-browser-pair-page.js",
            args.host_chrome,
        ])
    for path in required:
        path = path.resolve()
        key = str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)
        require(freeze["files"].get(str(path), freeze["files"].get(key)) == sha(path),
            "required-runtime-pin", "a directly loaded input is not pinned")
    result = {}
    for role, receipt, key in (("provider", args.provider_build_receipt, "unified_admission_story"),
                              ("maintained", args.maintained_build_receipt, "unified_maintained_story")):
        build = json.loads(receipt.read_text())
        require(build.get("qualifiedForFrozenRun") is True and build.get("nativePlatform") == "host",
            "host-build", "root-qualified current Host build is required")
        bindings = compiled_inputs(receipt)
        require(not bindings.get("inactiveCurrentSourceDeltas"), "compiled-input", "actual Host SDK inputs changed")
        deployed = build["deployment"][key]
        require(sha(deployed["deployedPath"]) == deployed["deployedSha256"]
                and sha(deployed["compiledPath"]) == deployed["compiledSha256"]
                and type(deployed["transformation"].get("exit")) is int
                and deployed["transformation"]["exit"] == 0,
            "host-artifact", "original/deployment Host artifact chain differs")
        result[role] = {"binary": deployed["deployedPath"], "sha256": deployed["deployedSha256"],
            "buildReceiptSha256": sha(receipt), "bindings": bindings}
    wasm = json.loads(args.wasm_build_receipt.read_text())
    require(wasm.get("qualifiedForFrozenRun") is True, "wasm-build", "qualified WASM build required")
    result["browser"] = {"bindings": compiled_inputs(args.wasm_build_receipt),
        "buildReceiptSha256": sha(args.wasm_build_receipt), "artifacts": wasm["artifacts"]}
    for name, digest in wasm["artifacts"].items():
        require(sha(args.package / name) == digest, "package-artifact", "a Browser package file differs")
    facility = json.loads(args.facility.read_text())
    relay = urlsplit(args.relay)
    relay_is_controlled = (relay.scheme == "http" and relay.hostname in {"127.0.0.1", "localhost"}
                           and relay.port is not None)
    require(facility["native_url"] == "mqtts://broker.emqx.io:8883/"
            and facility["wss_url"] == "wss://broker.emqx.io:8084/mqtt"
            and ((controlled_loopback and relay_is_controlled)
                 or (not controlled_loopback and args.relay == "https://aps1-1.relay.n0.iroh.link./")),
        "explicit-facilities", "the approved signaling facility and selected relay mode are required")
    for name in ("provider_profile", "native_profile", "browser_profile"):
        value = json.loads(getattr(args, name).read_text())
        if positive:
            expected = {"stun_servers": [], "local_bind_addresses": [], "candidate_addresses": []}
            require(value == expected, "positive-empty-ice-profile",
                "positive M77 feedback requires the existing empty ICE profile")
        else:
            require(set(value) == {"stun_servers", "local_bind_addresses", "candidate_addresses"}
                    and isinstance(value["stun_servers"], list)
                    and (controlled_loopback
                         and value["stun_servers"] == ["stun:127.0.0.1:3478"]
                         and value["candidate_addresses"] == ["198.18.0.1"]
                         and value["local_bind_addresses"] == []
                         or not controlled_loopback
                         and all(item == "stun:162.159.207.0:3478" for item in value["stun_servers"])),
                "existing-profile", "a profile adds an unapproved observer or unknown policy")
    if not positive:
        require(json.loads(args.browser_profile.read_text())["stun_servers"]
                and json.loads(args.native_profile.read_text())["stun_servers"],
            "rtc-enabled-profile", "disabled RTC profiles cannot exercise M77 preparation")
    return result, facility


def rtc_prelude():
    path = ROOT / "browser-probe/harness/rtc-capability-fixture.mjs"
    script = "import {rtcCapabilityPrelude} from " + json.dumps(path.as_uri()) \
        + ";process.stdout.write(rtcCapabilityPrelude('observe'));"
    return subprocess.run(["rtk", "proxy", "node", "--input-type=module", "-e", script],
        check=True, capture_output=True, text=True, timeout=10).stdout


def run(args):
    controlled_loopback = getattr(args, "controlled_loopback", False)
    require(not args.output.exists(), "new-output", "preserve the existing output and first failure")
    args.output.mkdir(mode=0o700, parents=True)
    args.output.chmod(0o700)
    report = {"ticket": "MIOPUNCH-77",
        "scope": ("fixture-only controlled Host Native + Host Browser active-C feedback"
                  if controlled_loopback else "fixture-only actual maintained Node/relay continuity feedback"),
        "case": args.case, "passed": False, "m77Qualified": False,
        "networkFailureQualified": False, "physicalWanQualified": False,
        "command": ["rtk", "proxy", "python3", str(Path(__file__).resolve()), *sys.argv[1:]],
        "phases": [], "cleanup": {}, "sdkCalls": [], "firstRedCaptured": False,
        "firstRedQualified": False, "setupDeadlineSurvivalQualified": False,
        "positiveQualified": False, "causalWitnessQualified": False}
    evidence = PrivateEvidence(args.output)
    evidence.write("command", report["command"])
    actor = page = browser_device = None
    effective_relay = args.relay
    current = "preflight"
    inputs = None

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
        reply = phase(current, lambda: page.call(operation, values))
        report["sdkCalls"].append({"operation": operation, "ok": reply.get("ok") is True,
            "sdkCallerMs": reply.get("callerElapsedMs")})
        require(reply.get("ok") is True, "sdk-operation", "actual Browser SDK operation failed")
        return reply.get("result")

    def sdk_failure(operation, values=None):
        """Capture an expected SDK rejection without turning it into runner failure."""
        nonlocal current
        current = "sdk-failure:" + operation
        evidence.write(current + ":request", values)
        reply = phase(current, lambda: page.call(operation, values))
        reason = stable_failure_reason(reply)
        report["sdkCalls"].append({"operation": operation, "ok": reply.get("ok") is True,
            "expectedFailure": True, "failureClass": reply.get("failureClass"),
            "failureReason": reason, "sdkCallerMs": reply.get("callerElapsedMs")})
        require(reply.get("ok") is False, "candidate-failure", "active candidate unexpectedly committed")
        return reply

    def native(operation, expected, **values):
        request = {"op": operation, **values}
        evidence.write("native:" + operation + ":request", request)
        return phase("native:" + operation, lambda: actor.command(json.dumps(request), expected))

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

        class MaintainedPage(BrowserPage):
            def __init__(self, device, package, prelude):
                super().__init__(device, package, prelude=prelude, environment_rtc_probe=False)
                original = self.server.RequestHandlerClass

                class Handler(original):
                    def do_GET(self):
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
            provider_args[3:3] = ["--relay", args.relay]
        phase("start-real-provisioner", lambda: actor.start(provider_args,
            stop_command=json.dumps({"op": "stop"})))
        ready = phase("provisioner-ready", lambda: actor.wait_event("ready"))
        require(ready.get("normalPublicTrust") is True, "normal-trust", "provisioner normal trust was not observed")
        if controlled_loopback:
            from host_browser_runtime import HostBrowserPage

            class MaintainedHostPage(HostBrowserPage):
                def __init__(self, package, executable, prelude):
                    super().__init__(package, executable, prelude=prelude)
                    original = self.server.RequestHandlerClass

                    class Handler(original):
                        def do_GET(inner_self):
                            if inner_self.path == "/android-pair-page.js":
                                return inner_self.send_bytes(PAGE.read_bytes(), "text/javascript; charset=utf-8")
                            return super().do_GET()
                    self.server.RequestHandlerClass = Handler

            if not args.host_chrome:
                raise RuntimeError("controlled loopback requires an owned Host Chrome executable")
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
        maintained_args = [store, facility_path, native_profile, args.relay, browser_id]
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
            require(maintained.get("permittedRelay") == args.relay, "maintained-relay-owner",
                "actual saved maintained Node relay differs")
        reopened = sdk("start", {"configuration": configuration(maintained, True, effective_relay)})
        require(reopened["id"] == browser_id, "browser-reopen", "saved Browser identity changed")
        reopened_materials = sdk("material_receipt")
        require(all(reopened_materials.get(key) == materials.get(key)
            for key in ("id", "request", "authorization", "stage", "authorization_saved", "secret_saved")),
            "material-reopen", "durable membership/materials changed on reopen")
        sdk("open_member", {"target": maintained["id"]})
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
        for paths in (ordinary.get("paths", []), before.get("paths", [])):
            require(any(path.get("isRelay") is True and path.get("selected") is True
                        and path.get("route") == "relay:" + effective_relay for path in paths)
                    and not any(path.get("isRtc") is True for path in paths),
                "relay-before-rtc", "ordinary permitted relay must be ready before RTC preparation")
        first_stream = sdk("continuity_start")
        require(first_stream.get("sequence") == 0 and first_stream.get("stableUnchanged") is True
            and first_stream.get("bindingUnchanged") is True
            and first_stream.get("native", {}).get("bindingUnchanged") is True
            and first_stream.get("localStableId") == before["coordinator"]["stableId"]
            and first_stream.get("native", {}).get("stableId") == ordinary["stableId"]
            and first_stream["native"].get("payload") == "original stream PING 0",
            "relay-stream", "real same-M continuity stream did not acknowledge sequence0")
        browser_objects = sdk("rtc_fixture_snapshot")
        baseline_node = native("status", "status")["status"]
        require(all(integer(browser_objects.get(name)) and browser_objects[name] == 0 for name in (
                "openPeerConnections", "openDataChannels"))
            and all(integer(baseline_node.get(name)) and baseline_node[name] == 0 for name in (
                "activeRtcAssociations", "activeRtcPumps")),
            "no-initial-rtc", "relay stream began only after an unexpected RTC association")
        report["relayBeforePreparation"] = {"identitiesMatched": {"native": True, "browser": True},
            "currentMemberGates": True, "freshAckBoth": True, "path": "permitted-data-relay",
            "sequence0Acknowledged": True, "sameLocalBindings": True, "payloadMatched": True,
            "requestBytes": len("original stream PING 0".encode()),
            "responseBytes": len(first_stream["native"]["payload"].encode()),
            "originalFreshAckWatermarks": {"native": native_common.get("probeWatermark"),
                "browser": before["coordinator"].get("probeWatermark")},
            "advertisedRtcRoles": {"native": native_common.get("rtcRole"),
                "browser": before["coordinator"].get("rtcRole")},
            "browserObjectCounts": {name: browser_objects.get(name) for name in (
                "peerCreated", "channelCreated", "openPeerConnections", "openDataChannels")},
            "originalQuicRoles": {"native": ordinary.get("quicRole"), "browser": "client"}}
        if args.case == "causal-witness":
            policy = maintained.get("savedMaintenancePolicy", {})
            caller_ms = policy.get("contact_timeout_ms")
            path_policy = maintained.get("savedPathSelectionPolicy", {})
            require(integer(caller_ms, 100), "causal-witness-budget",
                "saved maintenance contact budget is unavailable for the causal active C")
            require(path_policy.get("preference") == "Quality"
                    and integer(path_policy.get("reevaluation_interval_ms"), 250),
                "causal-witness-quality-policy", "saved Quality path policy is unavailable")
            started = time.monotonic()
            failed = sdk_failure("improve_path", {"name": "member", "callerMs": caller_ms})
            active_elapsed_ms = round((time.monotonic() - started) * 1000, 3)
            after_failure = sdk("member_diagnostics")
            progress = (after_failure.get("bootstrap") or {}).get("activeRtcPrepareProgress")
            progress_valid = (isinstance(progress, dict)
                    and isinstance(progress.get("stage"), str)
                    and bool(progress.get("stage"))
                    and progress.get("completed") is False
                    and progress.get("role") in {"offerer", "answerer"}
                    and (progress.get("remainingMs") is None
                         or integer(progress.get("remainingMs"), 0))
                    and type(progress.get("originalOpen")) is bool
                    and type(progress.get("controlOpen")) is bool
                    and type(progress.get("controlPeerMatchesOriginal")) is bool)
            reason = stable_failure_reason(failed)
            require(reason == "DataPathUnavailable", "causal-witness-failure",
                "the current active C did not return the observed bounded path-unavailable failure")
            require(after_failure.get("ordinaryHandle") is True
                    and after_failure.get("memberAuthorized") is True
                    and any(path.get("isRelay") is True and path.get("selected") is True
                            for path in after_failure.get("paths", []))
                    and not any(path.get("isRtc") is True for path in after_failure.get("paths", [])),
                "causal-witness-relay-survival", "the failed active C changed the healthy original M")
            survived = sdk("continuity_exchange", {"name": "member", "sequence": 1})
            require(survived.get("sequence") == 1
                    and survived.get("streamId") == first_stream.get("streamId")
                    and survived.get("stableUnchanged") is True
                    and survived.get("bindingUnchanged") is True
                    and survived.get("native", {}).get("bindingUnchanged") is True,
                "causal-witness-same-stream", "the original M stream did not survive the failed active C")
            pong = sdk("exchange", {"name": "member", "message": "PING"})
            require(pong == "PONG", "causal-witness-pong", "the original M did not answer PONG after the failed active C")
            require(progress_valid, "causal-witness-progress",
                "active C did not leave a bounded last-stage witness on the original M")
            report["causalWitness"] = {
                "activeCAttempted": True,
                "failureReason": reason,
                "failureClass": failed.get("failureClass"),
                "callerBudgetMs": caller_ms,
                "callerElapsedMs": failed.get("callerElapsedMs"),
                "observedElapsedMs": active_elapsed_ms,
                "lastStage": progress.get("stage"),
                "lastStageCompleted": progress.get("completed"),
                "stageRole": progress.get("role"),
                "remainingMs": progress.get("remainingMs"),
                "sameStableId": survived.get("stableUnchanged") is True,
                "sameBinding": survived.get("bindingUnchanged") is True
                    and survived.get("native", {}).get("bindingUnchanged") is True,
                "sameStream": survived.get("streamId") == first_stream.get("streamId"),
                "pongAfterFailure": True,
                "relayHealthy": True,
                "positiveQualified": False,
                "pathPolicy": path_policy,
            }
            report["positiveQualified"] = False
            report["causalWitnessQualified"] = True
            report["passed"] = True
            raise CaseCompleted()
        if args.case == "positive":
            policy = maintained.get("savedMaintenancePolicy", {})
            caller_ms = policy.get("contact_timeout_ms")
            path_policy = maintained.get("savedPathSelectionPolicy", {})
            require(integer(caller_ms, 100), "positive-budget",
                "saved maintenance contact budget is unavailable for positive active C")
            require(path_policy.get("preference") == "Quality"
                    and integer(path_policy.get("reevaluation_interval_ms"), 250),
                "positive-quality-policy", "saved Quality path policy is unavailable")
            started = time.monotonic()
            committed = sdk("improve_path", {"name": "member", "callerMs": caller_ms})
            active_elapsed_ms = round((time.monotonic() - started) * 1000, 3)
            require(committed.get("outcome") == "committed"
                    and committed.get("stableId") == first_stream.get("localStableId"),
                "positive-active-commit", "active C did not commit the existing M")
            after_active = sdk("member_diagnostics")
            active_objects = sdk("rtc_fixture_snapshot")
            native_active = native("status", "status")["status"]
            bootstrap = after_active.get("bootstrap") or {}
            active_coordinator = after_active.get("coordinator") or {}
            require(after_active.get("ordinaryHandle") is True
                    and after_active.get("memberAuthorized") is True
                    and after_active.get("expectedPeerMatched") is True,
                "positive-member-current", "active C changed the original member gate")
            require_active_commit_proof(after_active, first_stream.get("localStableId"))
            browser_rtc_paths = [path for path in after_active.get("paths", []) if path.get("isRtc") is True]
            native_rtc_paths = [path for path in native_active.get("ordinary", {}).get("paths", [])
                                if path.get("isRtc") is True]
            require(any(path.get("selected") is True for path in browser_rtc_paths)
                    and any(path.get("selected") is True for path in native_rtc_paths),
                "positive-rtc-selected", "the committed RTC path is not selected on both M views")
            selected_native = [path for path in native_rtc_paths if path.get("selected") is True]
            require(all(integer(path.get("rttMicros")) for path in selected_native),
                "positive-quality-rtt", "selected RTC path did not report measured RTT")
            require(integer(active_objects.get("peerCreated"), 1)
                    and integer(active_objects.get("channelCreated"), 1)
                    and integer(active_objects.get("openPeerConnections"), 1)
                    and integer(active_objects.get("openDataChannels"), 1)
                    and integer(native_active.get("activeRtcAssociations"), 1)
                    and integer(native_active.get("activeRtcPumps"), 1),
                "positive-rtc-live", "positive active C did not retain live RTC resources")
            survived = sdk("continuity_exchange", {"name": "member", "sequence": 1})
            require(survived.get("sequence") == 1
                    and survived.get("streamId") == first_stream.get("streamId")
                    and survived.get("stableUnchanged") is True
                    and survived.get("bindingUnchanged") is True
                    and survived.get("native", {}).get("bindingUnchanged") is True,
                "positive-same-stream", "same original stream did not survive active C commit")
            pong = sdk("exchange", {"name": "member", "message": "PING"})
            require(pong == "PONG", "positive-pong", "original M did not answer PONG after active C commit")
            report["positive"] = {
                "activeCAttempted": True,
                "activeCCommitted": True,
                "callerBudgetMs": caller_ms,
                "callerElapsedMs": committed.get("callerElapsedMs"),
                "observedElapsedMs": active_elapsed_ms,
                "setupDeadlineSurvived": None,
                "sameStableId": survived.get("stableUnchanged") is True,
                "sameBinding": survived.get("bindingUnchanged") is True
                    and survived.get("native", {}).get("bindingUnchanged") is True,
                "sameStream": survived.get("streamId") == first_stream.get("streamId"),
                "pongAfterCommit": True,
                "freshAck": bootstrap.get("freshAck"),
                "bothConfirmed": bootstrap.get("bothConfirmed"),
                "finalReleaseFreshAck": bootstrap.get("finalReleaseFreshAck"),
                "pathPolicy": path_policy,
                "selectedRtcRttMicros": [path.get("rttMicros") for path in selected_native],
                "memberDataRelayOptIn": False,
                "preferDirectCaseDeferred": True,
            }
            report["setupDeadlineSurvivalQualified"] = False
            report["positiveQualified"] = True
            report["passed"] = True
            raise CaseCompleted()
        if args.case == "network-failure":
            failure_policy = maintained.get("savedMaintenancePolicy", {})
            failure_caller_ms = failure_policy.get("contact_timeout_ms")
            require(integer(failure_caller_ms, 100), "network-failure-budget",
                "saved maintenance contact budget is unavailable for the active C attempt")
            failed = sdk_failure("improve_path", {"name": "member", "callerMs": failure_caller_ms})
            candidate_after_failure = sdk("candidate_observation", {"name": "member"})
            candidate_failure_immediate = candidate_after_failure.get("immediate", {})
            candidate_failure_late = candidate_after_failure.get("late", {})
            after_failure_member = candidate_failure_late.get("member", {})
            immediate_failure_objects = candidate_failure_immediate.get("rtcObjects", {})
            browser_failure_objects = candidate_failure_late.get("rtcObjects", {})
            reason = stable_failure_reason(failed)
            peer_delta = browser_failure_objects.get("peerCreated", 0) - browser_objects.get("peerCreated", 0)
            peer_close_delta = browser_failure_objects.get("peerCloseCalls", 0) - browser_objects.get("peerCloseCalls", 0)
            channel_delta = browser_failure_objects.get("channelCreated", 0) - browser_objects.get("channelCreated", 0)
            channel_close_delta = browser_failure_objects.get("channelCloseCalls", 0) - browser_objects.get("channelCloseCalls", 0)
            require(reason in {"DataPathUnavailable", "DeadlineExceeded", "Cancelled"},
                "network-failure-reason", "actual active RTC failure reason is not a bounded path failure")
            require(integer(peer_delta, 1) and integer(peer_close_delta)
                and peer_close_delta >= peer_delta,
                "network-failure-peer-retirement", "actual active C did not create and close a new peer")
            require(integer(channel_delta) and integer(channel_close_delta)
                and channel_close_delta >= channel_delta,
                "network-failure-channel-retirement", "actual active C channel counters are inconsistent")
            for sample_name, sample in (("immediate", candidate_failure_immediate),
                                        ("late", candidate_failure_late)):
                objects = sample.get("rtcObjects", {})
                require(integer(objects.get("openPeerConnections"))
                    and objects["openPeerConnections"] == 0
                    and integer(objects.get("openDataChannels"))
                    and objects["openDataChannels"] == 0,
                    "network-failure-" + sample_name + "-retirement",
                    "new active C RTC objects remained open in " + sample_name + " observation")
            require(after_failure_member.get("ordinaryHandle") is True
                and after_failure_member.get("memberAuthorized") is True
                    and any(path.get("isRelay") is True and path.get("selected") is True
                        for path in after_failure_member.get("paths", []))
                and not any(path.get("isRtc") is True and path.get("selected") is True
                            for path in after_failure_member.get("paths", [])),
                "network-failure-relay-survival", "failed RTC attempt changed the healthy relay M")
            survived = sdk("continuity_exchange", {"name": "member", "sequence": 1})
            require(survived.get("sequence") == 1
                and survived.get("streamId") == first_stream.get("streamId")
                and survived.get("stableUnchanged") is True
                and survived.get("bindingUnchanged") is True
                and survived.get("native", {}).get("bindingUnchanged") is True,
                "network-failure-same-stream", "same original stream did not survive RTC failure")
            pong = sdk("exchange", {"name": "member", "message": "PING"})
            require(pong == "PONG", "network-failure-pong", "original M did not answer PONG after RTC failure")
            report["networkFailure"] = {
                "activeCAttempted": True, "failureReason": reason,
                "failureClass": failed.get("failureClass"), "callerBudgetMs": failure_caller_ms,
                "callerElapsedMs": failed.get("callerElapsedMs"),
                "baseline": {name: browser_objects.get(name) for name in (
                    "peerCreated", "peerCloseCalls", "channelCreated", "channelCloseCalls")},
                "afterImmediate": {name: immediate_failure_objects.get(name) for name in (
                    "peerCreated", "peerCloseCalls", "channelCreated", "channelCloseCalls",
                    "openPeerConnections", "openDataChannels")},
                "afterLate": {name: browser_failure_objects.get(name) for name in (
                    "peerCreated", "peerCloseCalls", "channelCreated", "channelCloseCalls",
                    "openPeerConnections", "openDataChannels")},
                "deltas": {"peerCreated": peer_delta, "peerCloseCalls": peer_close_delta,
                           "channelCreated": channel_delta, "channelCloseCalls": channel_close_delta},
                "sameStableId": survived.get("stableUnchanged") is True,
                "sameBinding": survived.get("bindingUnchanged") is True
                    and survived.get("native", {}).get("bindingUnchanged") is True,
                "sameStream": survived.get("streamId") == first_stream.get("streamId"),
                "pongAfterFailure": True, "relayHealthy": True,
            }
            report["networkFailureQualified"] = True
            report["passed"] = True
            raise CaseCompleted()
        native("arm-active-candidate-failure", "active-candidate-failure-armed")
        failed = sdk_failure("improve_path", {"name": "member", "callerMs": 3000})
        failure_state = native("active-candidate-failure-state", "active-candidate-failure-state")
        candidate_after_failure = sdk("candidate_observation", {"name": "member"})
        candidate_failure_immediate = candidate_after_failure.get("immediate", {})
        candidate_failure_late = candidate_after_failure.get("late", {})
        after_failure_member = candidate_failure_late.get("member", {})
        failure_state_value = failure_state.get("state", {})
        browser_failure_objects = candidate_failure_late.get("rtcObjects", {})
        require(failure_state_value.get("checkpointReached") is True
            and failure_state_value.get("held") is False
            and integer(failure_state_value.get("activeRtcAssociations"))
            and failure_state_value["activeRtcAssociations"] == 0
            and integer(failure_state_value.get("activeRtcPumps"))
            and failure_state_value["activeRtcPumps"] == 0,
            "candidate-driver-failure", "native active candidate driver checkpoint was not reached")
        require(all(integer(browser_failure_objects.get(name))
                    and browser_failure_objects[name] == 0
                    for name in ("openPeerConnections", "openDataChannels")),
            "candidate-browser-cleanup", "failed active candidate left browser RTC objects open")
        require(after_failure_member.get("ordinaryHandle") is True
            and after_failure_member.get("memberAuthorized") is True
            and any(path.get("isRelay") is True and path.get("selected") is True
                    for path in after_failure_member.get("paths", []))
            and not any(path.get("isRtc") is True for path in after_failure_member.get("paths", [])),
            "candidate-failure-relay-survival", "failed active candidate changed or closed the relay M")
        survived = sdk("continuity_exchange", {"name": "member", "sequence": 1})
        require(survived.get("sequence") == 1
            and survived.get("streamId") == first_stream.get("streamId")
            and survived.get("stableUnchanged") is True
            and survived.get("bindingUnchanged") is True
            and survived.get("native", {}).get("bindingUnchanged") is True,
            "candidate-failure-stream-survival", "original stream did not survive candidate failure")
        native("clear-active-candidate-failure", "active-candidate-failure-cleared")
        report["candidateFailure"] = {
            "attempted": True,
            "sdkReturnedFailure": failed.get("ok") is False,
            "failureClass": failed.get("failureClass"),
            "callerElapsedMs": failed.get("callerElapsedMs"),
            "nativeCheckpointReached": failure_state_value.get("checkpointReached"),
            "relayMHealthy": True,
            "sameStreamAfterFailure": True,
            "observerLateDelayMs": candidate_after_failure.get("lateDelayMs"),
            "browserRtcObjectsImmediate": {
                name: candidate_failure_immediate.get("rtcObjects", {}).get(name)
                for name in ("openPeerConnections", "openDataChannels")},
            "browserRtcObjectsAfterFailure": {
                name: browser_failure_objects.get(name)
                for name in ("openPeerConnections", "openDataChannels")},
            "nativeRtcObjectsAfterFailure": {
                name: failure_state_value.get(name)
                for name in ("activeRtcAssociations", "activeRtcPumps")},
        }
        released = native("release-business-priority", "priority-released")
        require(released.get("businessPriorityReleased") is True, "priority-drop", "actual priority lease was not dropped")
        wake = native("network-changed", "network-changed")
        policy = maintained["savedMaintenancePolicy"]
        require(integer(policy.get("exploration_interval_ms"), 1) and integer(policy.get("contact_timeout_ms"), 1),
            "saved-policy", "existing whole-operation maintenance policy unavailable")
        window_ms = policy["exploration_interval_ms"] + policy["contact_timeout_ms"]
        require(window_ms <= 12000, "observation-window",
            "saved policy requires a separately released longer fixture observation")
        report["maintenancePolicy"] = policy
        start = time.monotonic()
        sequence, observations = 1, []
        while (time.monotonic() - start) * 1000 < window_ms:
            sequence += 1
            value = sdk("continuity_exchange", {"sequence": sequence})
            require(value.get("sequence") == sequence and value.get("streamId") == first_stream["streamId"]
                and value.get("localStableId") == first_stream["localStableId"]
                and value.get("native", {}).get("stableId") == first_stream["native"]["stableId"]
                and value.get("stableUnchanged") is True and value.get("bindingUnchanged") is True
                and value.get("native", {}).get("bindingUnchanged") is True,
                "same-stream", "original relay M/stream changed during the maintenance opportunity")
            observations.append(native("status", "status")["status"])
            time.sleep(min(0.25, max(0, window_ms / 1000 - (time.monotonic() - start))))
        after = sdk("member_diagnostics")
        final_node = observations[-1]
        require(integer(final_node.get("watchNotifications")) and integer(wake["status"].get("watchNotifications"))
            and final_node["watchNotifications"] > wake["status"]["watchNotifications"],
            "owner-opportunity", "actual maintained-owner watch produced no later notification")
        require(final_node["ordinary"].get("open") is True
            and final_node["ordinary"].get("stableUnchanged") is True
            and final_node["ordinary"].get("bindingUnchanged") is True,
            "healthy-relay", "the actual original M did not remain healthy")
        rtc_present = any(path.get("isRtc") is True for path in after.get("paths", []))
        report["feedback"] = {"actualOwnerWake": True, "actualBusinessPriorityDropped": True,
            "watchNotificationsBefore": wake["status"]["watchNotifications"],
            "watchNotificationsAfter": final_node["watchNotifications"],
            "observationMs": round((time.monotonic() - start) * 1000, 3),
            "savedWholeOperationMs": policy["contact_timeout_ms"], "sameStreamAcknowledgedSequences": sequence,
            "originalMHealthy": True, "rtcCandidateObserved": rtc_present,
            "perMemberMaintenanceReservation": None}
        report["firstRedCaptured"] = not rtc_present
        if not rtc_present:
            raise EvidenceFailure("live-member-improvement-not-observed",
                "real relay M/stream stayed healthy after the actual maintenance wake, but no RTC candidate was prepared")
        # No full77 quality/failure/MID1 qualification is asserted by this first seam.
        report["passed"] = True
    except CaseCompleted:
        pass
    except BaseException as error:
        evidence.exception(current, error)
        remember_failure(report, error, current)
    finally:
        if page is not None:
            try:
                value = phase("final-browser-cleanup", page.close)
                report["cleanup"]["browser"] = public_cleanup(value)
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
    parser.add_argument("--case", choices=("maintained", "positive", "causal-witness", "network-failure"), default="maintained")
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
