#!/usr/bin/env python3
"""One physical-device relay-only SDK case; host controls and records only."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
import urllib.request

from android_pair_runtime import ADB, BrowserPage, Device, NativeActor, ROOT, browser_cleanup_succeeded

MEMBER_ALPN = "miopunch/session/member/1"
OMIT = object()
CASES = {
    "disabled": ("observe", False, "disabled-by-policy"),
    "missing-api": ("api-missing", True, "api-missing"),
    "pc-denied": ("peer-connection-denied", True, "peer-connection-rejected"),
    "dc-denied": ("data-channel-denied", True, "data-channel-rejected"),
    "no-data-path": ("observe", False, "disabled-by-policy"),
}
DISABLED_OBSERVER = "stun://162.159.207.0:3478/"
RELAY_TRACE_PREFIX = "[DEBUG-mio76-relay] "
RELAY_PHASES = {"entry", "begin", "exchange", "probe-start", "probe-progress", "probe-ack",
                "validation-end", "finish"}
RELAY_COUNT_FIELDS = {"attemptSequence", "configuredPermittedCount", "localPublishedCount",
    "localConnectedCount", "localRelayErrorCount", "relayPathCount", "permittedRelayPathCount",
    "localSupportedMask", "localPermittedMask", "rtcRole", "localCandidateCount",
    "remoteCandidateCount", "mutuallyAllowedCount", "probeWatermark", "elapsedMs",
    "callerRemainingMs", "stageRemainingMs", "probePathRttMicros", "probePingTx", "probePingRx",
    "probeAckTx", "probeAckRx", "probePathAckTx", "probePathAckRx", "probeDatagramsTx",
    "probeDatagramsRx", "probeBytesTx", "probeBytesRx", "probeLostPackets", "startedUtcMs"}
RELAY_BOOL_FIELDS = {"sideClient", "memberOpen", "originalOpen", "cachedGateValid",
    "storageGateCheckPassed", "gateValid", "freshProbeIssued", "freshAck", "validationOk", "finishOk",
    "exactProbePathPresent", "probePathSelected"}
RELAY_HOME_COUNT_FIELDS = {"publishedCount", "connectedCount", "errorCount"}
INACTIVE_RELAY_SOURCE_DELTAS = {
    "src/bin/unified_native_member.rs": "separate native traversal actor is not this Android binary",
    "src/native_path_adapter.rs": "native traversal branches are disabled by this relay-only fixture",
    "src/tcp_punch.rs": "native TCP branches are disabled; this fixture installs no TCP transport",
    "src/network_maintenance.rs": "root-approved endpoint-only fixture calls bind_configured_endpoint_with_rtc_and_contact; it never calls the new maintain_network_with_endpoint builder seam",
}
INACTIVE_BROWSER_NATIVE_SOURCE_DELTAS = {
    "src/mqtt_transport.rs": "root-approved Android-only MQTT trust fix; browser lib.rs and actual WASM dep-info do not include this native module",
    "src/resources.rs": "root-approved native contact-route readiness fix; browser lib.rs and actual WASM dep-info do not include this native module",
    "src/native_network.rs": "native full-facility override permission fix; actual WASM dep-info does not include this native module",
}


def source_path(name):
    return Path(name.removeprefix("external:")) if name.startswith("external:") else ROOT / name


def sources_match(snapshot):
    return all(source_path(name).is_file() and digest(source_path(name)) == value
               for name, value in snapshot.items())


def qualified_source_binding(receipt, snapshot, *, browser=False):
    build = json.loads(receipt.read_text())
    before = json.loads((receipt.parent / "sources-before.json").read_text())
    source_digest = hashlib.sha256(json.dumps(snapshot, sort_keys=True).encode()).hexdigest()
    require(before == snapshot and build.get("sourceDigestBefore") == source_digest
            and build.get("sourceDigestAfter") == source_digest,
            "compiled source manifest is not the exact frozen build input")
    verified, inactive, current = {}, {}, {}
    inactive_deltas = dict(INACTIVE_RELAY_SOURCE_DELTAS)
    if browser:
        inactive_deltas.update(INACTIVE_BROWSER_NATIVE_SOURCE_DELTAS)
    for name, expected in snapshot.items():
        path = source_path(name)
        actual = digest(path) if path.is_file() else None
        require(actual is not None, "frozen build source is missing: " + name)
        current[name] = actual
        if actual == expected:
            verified[name] = expected
        else:
            require(name in inactive_deltas, "used source differs from frozen build: " + name)
            inactive[name] = {"compiledSourceSha256": expected, "currentSourceSha256": actual,
                              "scope": inactive_deltas[name]}
    binding = {"verifiedCurrentSourceSubset": verified, "inactiveCurrentSourceDeltas": inactive,
            "allCompiledSourceMatchesCurrent": not inactive, "allCurrentSourceMatch": not inactive,
            "allCurrentNativeSourceMatch": not inactive,
            "artifactPlatform": "browser" if browser else "native",
            "activeSubsetQualified": True, "maintainedNodeQualified": False,
            "qualification": "exact frozen artifact and endpoint-only relay case; subsequent native traversal and Node maintenance changes are not qualified"}
    if browser:
        dep_info = ROOT / "target/wasm32-unknown-unknown/debug/miopunch_browser_probe.d"
        dependencies = {Path(name).resolve() for name in dep_info.read_text().split(": ", 1)[1].split()}
        require(all(source_path(name).resolve() not in dependencies for name in inactive),
                "an inactive-source exception is actually compiled by this WASM artifact")
        require(all(digest(path) == snapshot.get(str(path.relative_to(ROOT)))
                    for path in dependencies if path.is_relative_to(ROOT)),
                "an actual WASM dependency differs from the compiled build snapshot")
        binding["dependencyInventory"] = {"path": str(dep_info), "sha256": digest(dep_info),
            "allCurrentDeltasAbsentFromActualDependencies": True,
            "allActualCompiledSourcesStillMatch": True,
            "compiledDependencyCount": len(dependencies)}
    return binding, current


def native_artifact_receipt(build, binary):
    actual = digest(binary)
    direct = build.get("artifacts", {}).get(str(binary.resolve()))
    if direct == actual:
        return {"compiledSha256": actual, "deployedSha256": actual, "transformation": "none"}
    for deployment in build.get("deployment", {}).values():
        if (Path(deployment["deployedPath"]).resolve() == binary.resolve()
                and deployment["deployedSha256"] == actual
                and build.get("artifacts", {}).get(deployment["compiledPath"])
                    == deployment["compiledSha256"]
                and deployment.get("transformation", {}).get("exit") == 0):
            return deployment
    raise RuntimeError("native binary differs from its frozen build/deployment receipt")


def native_cleanup_succeeded(cleanup, directory_required=True):
    retired = cleanup.get("sdkRetirement", {})
    count_names = ("activeRtcAssociations", "activeRtcPumps", "socketsBefore", "socketsLate")
    if any(type(retired.get(name)) is not int or retired[name] < 0 for name in count_names):
        return False
    return (cleanup.get("forced") is False
            and type(cleanup.get("commandExit")) is int and cleanup["commandExit"] == 0
            and cleanup.get("ownedRemoteProcessAbsent") is True
            and cleanup.get("ownedRemoteProcessAbsentLate") is True
            and cleanup.get("localReadersEnded") is True
            and (not directory_required or cleanup.get("ownedDirectoryRemoved") is True)
            and retired.get("activeRtcAssociations") == 0 and retired.get("activeRtcPumps") == 0
            and retired["socketsLate"] <= retired["socketsBefore"])


def close_native(actor, remove_directory=True):
    cleanup = actor.close(remove_directory=remove_directory)
    try:
        cleanup["sdkRetirement"] = actor.wait_event("closed", timeout=1)
    except (RuntimeError, TimeoutError) as error:
        cleanup["sdkRetirementMissing"] = type(error).__name__
    return cleanup


def native_failure_diagnostics(lines):
    """Bounded fatal errors only; arbitrary transport logs remain private."""
    result = []
    for line in lines[:64]:
        line = line.strip()
        if not line.startswith(("Error:", "error:", "UNIFIED_CANDIDATE_FAILED")):
            continue
        if any(marker in line.lower() for marker in (
                "a=", "v=0", "sdp", "private", "exporter", "binding", "secret", "password", "credential")):
            result.append("<private diagnostic omitted>")
        else:
            result.append(re.sub(r"(?i)\b[0-9a-f]{32,}\b", "<identifier>", line[:4096]))
        if len(result) == 8:
            break
    return result


def relay_observation(value):
    """Consume only the agreed actual DTO. Missing observations remain absent."""
    if not isinstance(value, dict) or value.get("tag") != "DEBUG-mio76-relay":
        raise ValueError("invalid relay diagnostic tag")
    if value.get("phase") not in RELAY_PHASES:
        raise ValueError("invalid relay diagnostic phase")
    observed = {"tag": value["tag"], "phase": value["phase"]}
    for name in RELAY_COUNT_FIELDS:
        if name in value:
            require(type(value[name]) is int and value[name] >= 0, "invalid actual relay count: " + name)
            observed[name] = value[name]
    for name in RELAY_BOOL_FIELDS:
        if name in value:
            require(type(value[name]) is bool, "invalid actual relay boolean: " + name)
            observed[name] = value[name]
    if "selectedRouteKind" in value:
        require(value["selectedRouteKind"] in {"signal", "relay", "ip", "other", "none"},
                "invalid actual relay route kind")
        observed["selectedRouteKind"] = value["selectedRouteKind"]
    if "errorKind" in value:
        require(value["errorKind"] is None or (isinstance(value["errorKind"], str)
                and re.fullmatch(r"[A-Za-z]{1,64}", value["errorKind"])), "invalid relay IO error category")
        observed["errorKind"] = value["errorKind"]
    return observed


def native_relay_observations(lines):
    observations, malformed = [], 0
    for line in lines:
        if line.startswith(RELAY_TRACE_PREFIX):
            try:
                observations.append(relay_observation(json.loads(line[len(RELAY_TRACE_PREFIX):])))
            except (ValueError, RuntimeError):
                malformed += 1
    return {"observations": observations, "malformedReceipts": malformed,
            "scope": "actual bounded native relay diagnostics; absent phases and counts remain unknown"}


def browser_relay_observations(diagnostics):
    home = {}
    if "relayHome" in diagnostics:
        require(isinstance(diagnostics["relayHome"], dict), "invalid actual relay home watcher")
        observed = {}
        for name in RELAY_HOME_COUNT_FIELDS:
            if name in diagnostics["relayHome"]:
                value = diagnostics["relayHome"][name]
                require(type(value) is int and value >= 0, "invalid actual relay home count: " + name)
                observed[name] = value
        home["relayHome"] = observed
    if "relayChecks" not in diagnostics:
        return {"getterAvailable": False, "observations": None, **home}
    require(isinstance(diagnostics["relayChecks"], list), "invalid browser relay diagnostics list")
    observations = [relay_observation(value) for value in diagnostics["relayChecks"]]
    require(len(observations) <= 64, "browser relay diagnostics exceeded the agreed bound")
    return {"getterAvailable": True, "observations": observations, **home,
            "scope": "actual bounded browser relay diagnostics; absent phases and counts remain unknown"}


class OwnedCdpObserver:
    """Metadata-only CDP session on the already owned tab and ADB forward."""
    def __init__(self, page):
        self.page, self.process, self.reader = page, None, None
        self.condition = threading.Condition()
        self.events = []
        self.available = False
        self.failure_class = None

    def start(self):
        try:
            if not getattr(self.page, "cdp_base", None):
                raise RuntimeError("existing owned CDP forward is unavailable")
            with urllib.request.urlopen(self.page.cdp_base + "/json/list", timeout=2) as response:
                targets = [target for target in json.load(response)
                           if target.get("type") == "page" and target.get("url") == self.page.url]
            require(len(targets) == 1, "exact owned CDP target is unavailable")
            command = ["rtk", "proxy", "node", str(ROOT / "browser-probe/harness/relay-cdp-observer.mjs"),
                       targets[0]["webSocketDebuggerUrl"]]
            self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, text=True, start_new_session=True)

            def read():
                try:
                    for line in self.process.stdout:
                        if len(line) > 8192:
                            continue
                        try:
                            event = json.loads(line)
                        except ValueError:
                            continue
                        with self.condition:
                            if len(self.events) < 512:
                                self.events.append(event)
                            self.condition.notify_all()
                finally:
                    with self.condition:
                        self.condition.notify_all()

            self.reader = threading.Thread(target=read, daemon=True)
            self.reader.start()
            deadline = time.monotonic() + 3
            with self.condition:
                while not any(event.get("event") == "observer-ready" for event in self.events):
                    if self.process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("owned CDP Network enable did not complete")
                    self.condition.wait(0.1)
                self.available = any(event.get("event") == "observer-ready"
                                     and event.get("networkEnabled") is True for event in self.events)
        except Exception as error:
            self.failure_class = type(error).__name__
        return self.receipt()

    def receipt(self):
        with self.condition:
            return {"available": self.available, "scope": "existing owned page CDP Network metadata only",
                    "failureClass": self.failure_class, "events": list(self.events),
                    "framesOrHeadersRecorded": False}

    def close(self):
        started = time.monotonic()
        cleanup = {"forced": False}
        if self.process:
            if self.process.poll() is None:
                try:
                    self.process.stdin.write("stop\n")
                    self.process.stdin.flush()
                    self.process.stdin.close()
                except (OSError, ValueError):
                    cleanup["stopWriteFailed"] = True
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    cleanup["forced"] = True
                    os.killpg(self.process.pid, signal.SIGTERM)
                    try:
                        self.process.wait(timeout=1)
                    except subprocess.TimeoutExpired:
                        os.killpg(self.process.pid, signal.SIGKILL)
                        self.process.wait()
            cleanup["commandExit"] = self.process.poll()
        if self.reader:
            self.reader.join(timeout=1)
        cleanup["readerEnded"] = not self.reader or not self.reader.is_alive()
        cleanup["elapsedMs"] = round((time.monotonic() - started) * 1000, 3)
        return cleanup


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def source_hashes():
    paths = {ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__).resolve(),
             ROOT / "browser-probe/Cargo.toml",
             ROOT / "scripts/android_pair_runtime.py"}
    for directory in ("src", "browser-probe/src"):
        paths.update((ROOT / directory).rglob("*.rs"))
        paths.update((ROOT / directory).rglob("*.js"))
    paths.update((ROOT / "browser-probe/harness").glob("*capability*"))
    paths.add(ROOT / "browser-probe/harness/android-pair-page.js")
    paths.add(ROOT / "browser-probe/harness/relay-cdp-observer.mjs")
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths)}


def artifact_hashes(binary, package):
    return {"androidNative": digest(binary),
            "browserPackage": {str(path.relative_to(package)): digest(path)
                for path in sorted(package.rglob("*"))
                if path.is_file() and path.suffix in {".js", ".wasm"}}}


def public(value, key=""):
    """Publish observations and comparisons, never private fixture material."""
    forbidden = {"invitation", "key", "secret", "password", "exporter", "binding", "context",
                 "authorization", "contact", "applicant", "remoteId", "localId", "remote_id",
                 "local_id", "request_id", "node_id", "root", "genesis", "route", "mqtt_route",
                 "stableId", "stable_id", "memberStableId", "request", "code"}
    sensitive = {"sdp", "description", "privatekey", "nodekey", "credential", "icepwd", "icepassword",
                 "usernamefragment", "ufrag", "bindinghash", "protectedbinding", "claimed", "publickey",
                 "peerid", "expectedpeerid", "recipient"}
    if key.replace("_", "").lower() in sensitive:
        return value if isinstance(value, bool) else OMIT
    if key == "route" and isinstance(value, str) and value.startswith("relay:"):
        return value
    if key in forbidden and not (key == "request" and isinstance(value, dict)):
        return value if isinstance(value, bool) else OMIT
    if key in {"members", "knownMembers", "records"} and isinstance(value, list) and all(isinstance(child, str) for child in value):
        return len(value)
    if key == "paths" and isinstance(value, list) and any(isinstance(child, str) for child in value):
        return OMIT  # Historical Debug contact addresses are opaque routing material.
    if key == "id" and isinstance(value, str):
        return OMIT
    if isinstance(value, dict):
        return {name: clean for name, child in value.items()
                if (clean := public(child, name)) is not OMIT}
    if isinstance(value, list):
        return [public(child) for child in value]
    if isinstance(value, str) and key in {"failure", "error", "observation"}:
        return re.sub(r"\b[0-9a-fA-F]{32,128}\b", "<private-fixture-value>", value)
    return value


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES, required=True)
    parser.add_argument("--browser-model", choices=("Pixel_6a", "TB322FC"), default="Pixel_6a")
    parser.add_argument("--native-model", choices=("Pixel_6a", "TB322FC"), default="TB322FC")
    parser.add_argument("--adb", type=Path, default=ADB)
    parser.add_argument("--native-binary", type=Path, required=True)
    parser.add_argument("--native-build-receipt", type=Path, required=True)
    parser.add_argument("--browser-build-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, default=ROOT / "browser-probe/pkg")
    parser.add_argument("--relay", required=True, help="one explicitly permitted HTTPS data relay")
    parser.add_argument("--facility", type=Path, required=True, help="explicit public MQTT contact facility")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(args.browser_model != args.native_model, "the two business peers must be different devices")
    require(args.relay.startswith("https://"), "normal public HTTPS relay is required")
    require(not args.output.exists(), "preserve prior receipts; output must be new")
    args.output.mkdir(parents=True)
    report = {"ticket": "MIOPUNCH-76", "case": args.case, "passed": False,
              "available": False, "hostRole": "build/ADB/control/evidence only",
              "scope": "physical Browser-to-Native strict common SDK relay-only member stream",
              "physicalPublicNatQualified": False, "turnConfigured": False,
              "callerBudgetMs": 12000, "stageBudgetMs": 5000,
              "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "roles": {"browser": "member initiator", "native": "member responder"},
              "devices": {"browser": args.browser_model, "native": args.native_model},
              "relay": args.relay, "operations": [], "cleanup": {}}
    report["command"] = ["rtk", "proxy", sys.executable, *sys.argv]
    page = actor = browser_device = native_device = observer = None
    browser_id = None
    before = source_hashes()
    member_sessions = set()

    def save():
        (args.output / "receipt.json").write_text(json.dumps(public(report), indent=2) + "\n")

    def call(op, values=None, *, expected_ok=True):
        result = page.call(op, values)
        report["operations"].append({"operation": op, **public(result)})
        save()
        require(result.get("ok") is expected_ok, "formal browser SDK operation failed: " + op)
        return result.get("result") if expected_ok else result

    def configuration(ready, relays, disabled=None):
        configuration = {"relays": relays, "disabled": list(disabled or []), "irc": [],
                "mqtt": [facility], "rtc_enabled": enabled,
                "contact_routes": [{"contact": ready["id"], "facility": facility["logical_id"],
                                    "address": ready["route"]}]}
        if args.case == "disabled":
            configuration["rtc_ice"] = {"stun_servers": ["stun:162.159.207.0:3478"],
                                        "local_bind_addresses": [], "candidate_addresses": []}
            configuration["disabled"].append(DISABLED_OBSERVER)
        return configuration

    def check_capability():
        diagnostics = call("path_diagnostics")
        report["browserRelayDiagnostics"] = browser_relay_observations(diagnostics)
        capability = diagnostics.get("rtcCapability")
        require(isinstance(capability, dict), "actual SDK startup capability receipt is required")
        require(capability["reason"] == reason and capability["effective"] is False,
                "SDK did not classify this runtime/policy failure and withhold RTC")
        require(capability["permitted"] is enabled, "runtime support and resource permission were merged")
        require(capability["supported"] is (None if not enabled else False),
                "a failed or skipped runtime check was claimed supported")
        snapshot = call("rtc_fixture_snapshot")
        require(snapshot["openPeerConnections"] == 0 and snapshot["openDataChannels"] == 0,
                "startup runtime probe left a live RTC object")
        if args.case in {"disabled", "no-data-path", "missing-api", "pc-denied"}:
            require(snapshot["peerCreated"] == 0, "unsupported/disabled runtime constructed a PeerConnection")
        else:
            require(snapshot["peerCreated"] == 1 and snapshot["peerCloseCalls"] == 1,
                    "DataChannel denial did not close its real PeerConnection")
        report["startupCapability"] = capability
        report["startupObjects"] = snapshot
        return diagnostics

    def open_and_exchange(ready, message):
        call("open_member", {"target": ready["id"]})
        release = actor.wait_event("member-released")
        require(release.get("alpn") == MEMBER_ALPN, "member M did not use the strict new protocol")
        require(release.get("remoteId") == browser_id, "native member TLS peer differs from saved browser")
        session = release.get("stableId")
        require(session is not None and session not in member_sessions,
                "a new member attempt reused the previous ordinary session")
        prior_session_different = True if member_sessions else None
        member_sessions.add(session)
        native_path = actor.wait_event("unified-path-ready")
        require(native_path.get("freshAck") is True and native_path.get("role") == "responder",
                "native did not record a fresh authenticated ACK on this member M")
        diagnostics = call("member_diagnostics")
        require(diagnostics.get("newProtocol") is True and diagnostics.get("alpn") == MEMBER_ALPN
                and diagnostics.get("ordinaryHandle") is True and diagnostics.get("memberAuthorized") is True,
                "this actual browser ordinary M did not pass its original member gate and new protocol")
        require(release.get("memberAuthorized") is True, "the native did not verify this actual member M")
        require(diagnostics.get("expectedPeerMatched") is True,
                "browser did not verify this M's actual peer against its target")
        paths = diagnostics.get("paths", [])
        require(any(path.get("isRelay") and path.get("selected") for path in paths),
                "actual ordinary data path is not the permitted relay")
        require(not any(path.get("isRtc") for path in paths), "relay-only runtime published an RTC candidate")
        for count in ("activeAssociations", "activePumpTasks"):
            if count in diagnostics:
                require(diagnostics[count] == 0, "relay-only M retained an RTC association or pump")
        browser_path = diagnostics.get("coordinator") or {}
        require(browser_path.get("freshAck") is True and browser_path.get("role") == "initiator",
                "browser did not record this M's fresh authenticated path ACK")
        require(browser_path.get("version") == 1 and browser_path.get("rtcRole") == "None",
                "strict capability exchange advertised an RTC role for the unavailable browser runtime")
        require(call("exchange", {"message": message}) == message,
                "actual SDK stream request and response contents differ")
        business = actor.wait_event("business")
        require(business.get("alpn") == MEMBER_ALPN and business.get("bytes") == len(message.encode())
                and business.get("stableId") == release.get("stableId"),
                "native did not receive business bytes on this original member M")
        report.setdefault("members", []).append({"newProtocol": True, "alpn": release["alpn"],
            "identityMatched": {"native": True, "browser": diagnostics["expectedPeerMatched"]},
            "requestBytes": len(message.encode()), "responseBytes": len(message.encode()),
            "streamContentsMatched": True, "path": "permitted-data-relay",
            "newOrdinarySession": True, "priorSessionDifferent": prior_session_different,
            "freshAck": {"native": native_path["freshAck"], "browser": True},
            "browserRtcAdvertised": False, "capabilitiesVersion": 1,
            "nativePath": public(native_path), "browserPath": public(browser_path),
            "native": public(release), "browser": public(diagnostics), "nativeBusiness": public(business)})
        state = call("status")
        actual_paths = call("path_diagnostics")
        report["browserRelayDiagnostics"] = browser_relay_observations(actual_paths)
        report.setdefault("postMemberObservations", []).append({"status": public(state),
            "identityUnchanged": state["id"] == browser_id,
            "pathDiagnostics": public(actual_paths),
            "rtcAndWebSocketObjects": call("rtc_fixture_snapshot")})
        call("close_member")
        save()

    try:
        build = json.loads(args.native_build_receipt.read_text())
        require(build.get("qualifiedForFrozenRun") is True,
                "native artifact lacks a successful frozen-source build receipt")
        report["nativeDeployment"] = native_artifact_receipt(build, args.native_binary)
        require(Path(report["nativeDeployment"].get("compiledPath", args.native_binary)).name
                    == "browser_wss_member_native", "this qualification requires the endpoint-only native member fixture")
        native_sources_path = args.native_build_receipt.parent / "sources-after.json"
        native_sources = json.loads(native_sources_path.read_text())
        require(hashlib.sha256(json.dumps(native_sources, sort_keys=True).encode()).hexdigest()
                == build.get("sourceDigestAfter"), "native build source snapshot hash does not match")
        report["nativeSourceBinding"], native_bound_sources = qualified_source_binding(
            args.native_build_receipt, native_sources)
        report["nativeBuildReceipt"] = {"sha256": digest(args.native_build_receipt), "exit": build["exit"],
            "sourceSnapshotSha256": digest(native_sources_path), "sourceDigest": build["sourceDigestAfter"]}
        (args.output / "native-build.json").write_text(json.dumps(build, indent=2) + "\n")
        (args.output / "native-sources.json").write_text(json.dumps(native_sources, indent=2) + "\n")
        browser_build = json.loads(args.browser_build_receipt.read_text())
        browser_sources_path = args.browser_build_receipt.parent / "sources-after.json"
        browser_sources = json.loads(browser_sources_path.read_text())
        require(browser_build.get("qualifiedForFrozenRun") is True
                and browser_build.get("sourceFrozenDuringBuild") is True
                and browser_build.get("commandExit") == 0
                and all(command.get("exit") == 0 for command in browser_build.get("commands", [])),
                "browser artifact lacks a successful frozen-source build receipt")
        report["browserSourceBinding"], browser_bound_sources = qualified_source_binding(
            args.browser_build_receipt, browser_sources, browser=True)
        actual_package = artifact_hashes(args.native_binary, args.package)["browserPackage"]
        built_package = {name: value for name, value in browser_build.get("artifacts", {}).items()
                         if Path(name).suffix in {".js", ".wasm"}}
        require(actual_package == built_package and any(name.endswith(".wasm") for name in actual_package),
                "WASM/package snippets differ from their exact frozen build receipt")
        report["browserBuildReceipt"] = {"sha256": digest(args.browser_build_receipt),
            "exit": browser_build["commandExit"], "sourceSnapshotSha256": digest(browser_sources_path),
            "sourceDigest": browser_build["sourceDigestAfter"]}
        (args.output / "browser-build.json").write_text(json.dumps(browser_build, indent=2) + "\n")
        (args.output / "browser-sources.json").write_text(json.dumps(browser_sources, indent=2) + "\n")
        report["artifactSha256"] = artifact_hashes(args.native_binary, args.package)
        facility = json.loads(args.facility.read_text())
        require(facility.get("native_url", "").startswith("mqtts://")
                and facility.get("wss_url", "").startswith("wss://"),
                "this public case requires normal native MQTT TLS and browser WSS")
        report["facility"] = facility
        report["policy"] = {"normalCertificateValidation": True, "originHeaderOverride": False,
                            "serviceIsExplicitFixtureOnly": True}
        mode, enabled, reason = CASES[args.case]
        prelude = subprocess.check_output(["rtk", "proxy", "node", "--input-type=module", "-e",
            "import {rtcCapabilityPrelude} from './browser-probe/harness/rtc-capability-fixture.mjs';"
            "process.stdout.write(rtcCapabilityPrelude(process.argv[1]));", mode], cwd=ROOT, text=True)
        browser_device = Device(args.browser_model, "usb" if args.browser_model == "Pixel_6a" else "network", args.adb)
        native_device = Device(args.native_model, "usb" if args.native_model == "Pixel_6a" else "network", args.adb)
        actor = NativeActor(native_device)
        actor.deploy(args.native_binary, "native", executable=True)
        facility_path = actor.deploy(args.facility, "facility.json")
        store = actor.directory + "/node-store"
        actor.start([store, facility_path, "-", "admission"])
        ready = actor.wait_event("ready")
        require(ready.get("normalPublicTrust") is True, "native admission used a custom trust override")
        report["nativeAdmissionReady"] = public(ready)
        native_id = ready["id"]
        page = BrowserPage(browser_device, args.package, prelude=prelude, environment_rtc_probe=False)
        report["browserPage"] = public(page.start())
        observer = OwnedCdpObserver(page)
        report["cdpNetwork"] = observer.start()
        started = call("start", {"configuration": configuration(ready, [])})
        browser_id = started["id"]
        check_capability()
        require(call("request_admission", {"invitation": ready["invitation"]}) == "AwaitingApproval",
                "admission did not retain its original manual approval semantics")
        pending = actor.command("admission-status", "admission-status")
        requests = [request for request in pending["requests"] if request["applicant"] == browser_id]
        require(len(requests) == 1, "the independently identified browser has no exact pending admission request")
        request = requests[0]
        actor.command("approve-request " + request["applicant"] + " " + request["request"], "approved")
        require(call("request_admission", {"invitation": ready["invitation"]}) == "PendingMaterials",
                "approved admission material was not independently delivered")
        require(call("complete_admission") == "Complete", "approved materials were not durably completed")
        materials = call("material_receipt")
        require(materials["authorization_saved"] and materials["secret_saved"],
                "both original member materials must be persisted before reopening")
        call("stop")
        report["admissionNativeCleanup"] = close_native(actor, remove_directory=False)
        require(native_cleanup_succeeded(report["admissionNativeCleanup"], False),
                "admission process/readers did not close normally")
        actor.start([store, facility_path, "-", "unified-member", args.relay])
        ready = actor.wait_event("ready")
        report["nativeMemberReady"] = public(ready)
        require(ready["id"] == native_id, "native reopen changed the original persistent identity")
        require(ready.get("normalPublicTrust") is True and browser_id in ready.get("knownMembers", []),
                "native reopen did not retain its authorized browser with normal public trust")
        call("rtc_fixture_reset")
        reopened = call("start", {"configuration": configuration(ready, [args.relay])})
        require(reopened["id"] == browser_id and reopened["status"]["stage"] == "Complete",
                "browser reopen changed its identity or requires another membership decision")
        check_capability()
        report["persistedIdentitiesReopened"] = {"native": True, "browser": True}
        open_and_exchange(ready, "MIO76 " + args.case + " actual browser/native member stream")
        if args.case == "no-data-path":
            call("stop")
            call("rtc_fixture_reset")
            blocked = call("start", {"configuration": configuration(ready, [args.relay], [args.relay])})
            require(blocked["id"] == browser_id and blocked["status"]["stage"] == "Complete",
                    "a missing route changed member identity or authorization")
            require(blocked["status"]["running"] is True and blocked["status"]["authorization_saved"]
                    and blocked["status"]["secret_saved"], "no data route changed the running node or member materials")
            diagnostics_before = check_capability()
            carriers = diagnostics_before.get("carriers", [])
            require(len(carriers) == 1 and carriers[0].get("ready") is True
                    and carriers[0].get("closed") is False and carriers[0].get("protocol") == "mqtt",
                    "the real MQTT contact must be ready before the no-data-path attempt")
            failure = call("open_member", {"target": ready["id"]}, expected_ok=False)
            require(re.search(r"DataPathUnavailable|no currently permitted data path", failure.get("failure", "")),
                    "no data route did not report the stable DataPathUnavailable class")
            absent_handle = call("member_diagnostics", expected_ok=False)
            require("owned member handle does not exist" in absent_handle.get("failure", ""),
                    "no-data-path operation left an ordinary member handle")
            failure_status = call("status")
            require(failure_status.get("id") == browser_id and failure_status.get("stage") == "Complete"
                    and failure_status.get("running") is True
                    and failure_status.get("authorization_saved") is True
                    and failure_status.get("secret_saved") is True,
                    "DataPathUnavailable changed the running node's identity or membership")
            diagnostics_after = call("path_diagnostics")
            after_carriers = diagnostics_after.get("carriers", [])
            require(len(after_carriers) == 1 and after_carriers[0].get("ready") is True
                    and after_carriers[0].get("closed") is False,
                    "DataPathUnavailable withdrew an otherwise healthy MQTT contact")
            before_bytes = [carrier.get("traffic", {}).get("sent_bytes")
                            for carrier in diagnostics_before.get("carriers", [])]
            after_bytes = [carrier.get("traffic", {}).get("sent_bytes")
                           for carrier in diagnostics_after.get("carriers", [])]
            require(before_bytes == after_bytes == [0], "ordinary attempt sent QUIC/business bytes through MQTT")
            received_bytes = [carrier.get("traffic", {}).get("received_bytes")
                              for carrier in diagnostics_after.get("carriers", [])]
            require(received_bytes == [0], "ordinary attempt received QUIC/business bytes through MQTT")
            report["noDataPath"] = {"errorClass": "DataPathUnavailable", "ordinaryHandleReturned": False,
                "mqttContactReady": carriers[0]["ready"], "mqttQuicBytesSent": after_bytes,
                "mqttQuicBytesReceived": received_bytes,
                "ircConfigured": False, "identityAndMembershipUnchanged": True,
                "runningNodeUnchanged": True, "statusAfterFailure": public(failure_status)}
            call("stop")
            call("rtc_fixture_reset")
            recovered = call("start", {"configuration": configuration(ready, [args.relay])})
            require(recovered["id"] == browser_id and recovered["status"]["stage"] == "Complete",
                    "restoring a resource changed member material")
            check_capability()
            open_and_exchange(ready, "MIO76 relay permission restored on a new member attempt")
            report["newAttemptAfterRestore"] = True
        call("stop")
        report["cleanup"]["browserImmediate"] = call("rtc_fixture_snapshot")
        time.sleep(0.3)
        report["cleanup"]["browserLate"] = call("rtc_fixture_snapshot")
        late = report["cleanup"]["browserLate"]
        require(late["openPeerConnections"] == late["openDataChannels"] == late["openWebSockets"] == 0,
                "owned browser RTC objects or WSS sockets did not retire")
        report["available"] = True
        report["passed"] = True
    except Exception as error:
        report["firstFailure"] = {"class": type(error).__name__, "observation": str(error)}
        if page and page.ready:
            report["failureDiagnostics"] = []
            for operation in ("status", "path_diagnostics", "rtc_fixture_snapshot"):
                try:
                    response = page.call(operation, timeout=3)
                    report["failureDiagnostics"].append({"operation": operation, **public(response)})
                    if operation == "status" and response.get("ok") is True and browser_id is not None:
                        report["identityUnchangedAfterFailure"] = response["result"]["id"] == browser_id
                    if operation == "path_diagnostics" and response.get("ok") is True:
                        report["browserRelayDiagnostics"] = browser_relay_observations(response["result"])
                except Exception as diagnostic_error:
                    report["failureDiagnostics"].append({"operation": operation,
                        "failureClass": type(diagnostic_error).__name__})
    finally:
        if observer:
            report["cleanup"]["cdpObserver"] = observer.close()
            report["cdpNetwork"] = observer.receipt()
            if (report["cleanup"]["cdpObserver"].get("forced")
                    or report["cleanup"]["cdpObserver"].get("commandExit", 0) != 0
                    or not report["cleanup"]["cdpObserver"].get("readerEnded")):
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "ObserverCleanupIncomplete",
                    "observation": "owned local CDP observer did not close normally"})
        if page:
            report["cleanup"]["browserPage"] = page.close()
            report["browserVersion"] = page.browser_version
            if not browser_cleanup_succeeded(report["cleanup"]["browserPage"]):
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "BrowserCleanupIncomplete",
                    "observation": "owned page, local listener or ADB forwarding was not fully retired"})
        if actor:
            report["cleanup"]["native"] = close_native(actor)
            report["nativeCommands"] = actor.runs
            report["nativeFailureDiagnostics"] = native_failure_diagnostics(actor.stderr)
            report["nativeRelayDiagnostics"] = native_relay_observations(actor.stderr)
            report["nativePendingEvents"] = public([event for event in actor.events
                if event.get("event") in {"handle-withheld", "unified-path-ready", "member-released", "business"}])
            cleanup = report["cleanup"]["native"]
            if not native_cleanup_succeeded(cleanup):
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "NativeCleanupIncomplete",
                    "observation": "owned native process/readers/material directory did not close normally"})
        report["sourceSha256"] = before
        after = source_hashes()
        report["sourceStableDuringRun"] = before == after
        report["changedSourceFiles"] = [name for name, value in before.items() if after.get(name) != value]
        if before != after:
            report["passed"] = False
            report.setdefault("firstFailure", {"class": "SourceChangedDuringRun",
                "observation": "source changed during the SDK run; exact current source is not qualified"})
        if "native_bound_sources" in locals():
            report["nativeBuildSourceBindingStillMatches"] = sources_match(native_bound_sources)
            if not report["nativeBuildSourceBindingStillMatches"]:
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "NativeSourceChangedDuringRun",
                    "observation": "native/vendor source no longer matches the frozen native artifact"})
        if "browser_bound_sources" in locals():
            report["browserBuildSourceBindingStillMatches"] = sources_match(browser_bound_sources)
            if not report["browserBuildSourceBindingStillMatches"]:
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "BrowserSourceChangedDuringRun",
                    "observation": "browser/vendor source no longer matches the frozen WASM artifact"})
        if "artifactSha256" in report:
            final_artifacts = artifact_hashes(args.native_binary, args.package)
            report["artifactStableDuringRun"] = final_artifacts == report["artifactSha256"]
            if not report["artifactStableDuringRun"]:
                report["passed"] = False
                report.setdefault("firstFailure", {"class": "ArtifactChangedDuringRun",
                    "observation": "native or browser package changed during the SDK run"})
        if browser_device:
            report["browserControlCommands"] = browser_device.commands
        if native_device:
            report["nativeControlCommands"] = native_device.commands
        report["completedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["runnerExit"] = 0 if report["passed"] else 1
        save()
    print(json.dumps({"passed": report["passed"], "case": args.case, "receipt": str(args.output / "receipt.json")}))
    return report["runnerExit"]


if __name__ == "__main__":
    raise SystemExit(main())
