#!/usr/bin/env python3
"""One formal Android Browser→Native member story over explicit public MQTT.

ADB and the host HTTP page carry local control only. Identity/admission and all
QUIC traffic use the real SDK transports on the selected phones.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import tempfile
import time

from android_pair_runtime import BrowserPage, Device, NativeActor, browser_cleanup_succeeded
from build_android_pair import sources

ROOT = Path(__file__).resolve().parents[1]
MEMBER_ALPN = "miopunch/session/member/1"
HIDDEN = {"sdp", "description", "exporter", "binding", "bindinghash", "protectedbinding",
          "privatekey", "nodekey", "secret", "password", "credential", "invitation", "code",
          "usernamefragment", "icepwd", "icepassword", "ufrag", "id", "peer", "remoteid",
          "localid", "nodeid", "applicant", "knownmembers", "members"}
INACTIVE_SOURCE_DELTAS = {
    "src/bin/unified_native_member.rs": "separate native traversal actor is not the selected Android binary",
    "src/native_path_adapter.rs": "later native TCP preparation/unavailable status and rollback branches; this story disables native UDP/TCP and selects RTC",
    "src/tcp_punch.rs": "later native TCP observation/unavailable status branch; this story installs no TCP transport",
    "src/network_maintenance.rs": "later maintained-node builder customization; this story directly binds the configured endpoint and does not call maintain_network_*",
}


def redacted(value):
    if isinstance(value, list):
        return [redacted(entry) for entry in value]
    if isinstance(value, dict):
        return {key: redacted(entry) for key, entry in value.items()
                if key.replace("_", "").lower() not in HIDDEN
                and not any(hidden in key.lower() for hidden in (
                    "sdp", "ufrag", "certificate", "fingerprint", "username", "password"))}
    return value


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def snapshot():
    return sources([*(ROOT / "browser-probe/src").rglob("*"),
                    ROOT / "browser-probe/Cargo.toml", Path(__file__),
                    ROOT / "scripts/android_pair_runtime.py",
                    ROOT / "browser-probe/harness/android-pair-page.js"])


def checked(response):
    if response.get("ok") is not True:
        raise RuntimeError(response.get("failure", "browser SDK request failed"))
    return response["result"]


def validate_build_sources(receipt):
    before = json.loads((receipt.parent / "sources-before.json").read_text())
    after = json.loads((receipt.parent / "sources-after.json").read_text())
    if before != after:
        raise RuntimeError("build source snapshots differ")
    source_digest = hashlib.sha256(json.dumps(after, sort_keys=True).encode()).hexdigest()
    build = json.loads(receipt.read_text())
    if build["sourceDigestBefore"] != source_digest or build["sourceDigestAfter"] != source_digest:
        raise RuntimeError("build source digest differs from recorded snapshots")
    inactive = {}
    verified = {}
    allowed_inactive = dict(INACTIVE_SOURCE_DELTAS)
    if any("wasm32-unknown-unknown" in entry.get("command", [])
           for entry in build.get("commands", [])):
        # The historical broad snapshot also includes native modules that are
        # absent from browser-probe/src/lib.rs and its shared-module inventory.
        allowed_inactive.update({
            "src/resources.rs": "native ResourceEndpoint memberReady getter is not compiled into browser WASM",
            "src/mqtt_transport.rs": "native Android MQTT TLS factory is not compiled into browser WASM; its WebSocket carrier uses browser-probe/src/mqtt.rs",
        })
    for name, expected in after.items():
        path = Path(name.removeprefix("external:")) if name.startswith("external:") else ROOT / name
        actual = sha256(path) if path.is_file() else None
        if actual != expected:
            if name not in allowed_inactive:
                raise RuntimeError("used source differs from frozen build: " + name)
            inactive[name] = {"compiledSourceSha256": expected, "currentSourceSha256": actual,
                              "scope": allowed_inactive[name]}
        else:
            verified[name] = expected
    return {"compiledSourceSnapshotSha256": sha256(receipt.parent / "sources-after.json"),
            "verifiedCurrentSourceSubset": verified, "inactiveCurrentSourceDeltas": inactive,
            "allCurrentSourceMatch": not inactive,
            "qualification": "exact frozen artifact; current native traversal branches are not qualified by this RTC story"}


def native_cleanup_succeeded(value, directory_required=True):
    sdk = value.get("sdkRetirement", {})
    return (value.get("forced") is False and value.get("commandExit") == 0
            and value.get("ownedRemoteProcessAbsent") is True
            and value.get("ownedRemoteProcessAbsentLate") is True
            and value.get("localReadersEnded") is True
            and (not directory_required or value.get("ownedDirectoryRemoved") is True)
            and sdk.get("activeRtcAssociations") == 0 and sdk.get("activeRtcPumps") == 0
            and isinstance(sdk.get("socketsBefore"), int) and isinstance(sdk.get("socketsLate"), int)
            and sdk["socketsLate"] <= sdk["socketsBefore"])


def close_native(actor, remove_directory=True):
    cleanup = actor.close(remove_directory=remove_directory)
    try:
        cleanup["sdkRetirement"] = actor.wait_event("closed", timeout=1)
    except (RuntimeError, TimeoutError) as error:
        cleanup["sdkRetirementMissing"] = type(error).__name__
    return cleanup


def native_errors(actor):
    # Preserve actual startup/driver errors without private values or raw SDP.
    result = []
    for line in actor.stderr:
        if any(marker in line.lower() for marker in (
                "a=candidate:", "a=ice-", "v=0", "private key", "exporter=", "secret=", "password=")):
            result.append("<private diagnostic omitted>")
        else:
            result.append(re.sub(r"(?i)\b[0-9a-f]{32,}\b", "<identifier>", line.rstrip()))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--wasm-build-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--native-profile", type=Path, required=True)
    parser.add_argument("--browser-profile", type=Path, required=True)
    parser.add_argument("--relay", help="optional explicitly allowed data relay")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output exists; preserve original failures")
    args.output.mkdir(parents=True, mode=0o700)
    build = json.loads(args.build_receipt.read_text())
    if build.get("qualifiedForFrozenRun") is not True:
        parser.error("native build must have a successful frozen-source receipt")
    deployment = build["deployment"]["browser_wss_member_native"]
    binary = Path(deployment["deployedPath"])
    if sha256(binary) != deployment["deployedSha256"]:
        parser.error("native artifact differs from frozen deployment copy")
    compiled = Path(deployment["compiledPath"])
    if (not compiled.is_file() or sha256(compiled) != deployment["compiledSha256"]
            or build["artifacts"].get(str(compiled)) != deployment["compiledSha256"]
            or deployment["transformation"].get("exit") != 0
            or deployment["transformation"].get("operation") != "strip debug sections on owned copy only"):
        parser.error("compiled→strip→deployment chain differs from native receipt")
    wasm_build = json.loads(args.wasm_build_receipt.read_text())
    if wasm_build.get("qualifiedForFrozenRun") is not True:
        parser.error("browser build must have a successful frozen-source receipt")
    wasm_artifacts = wasm_build["artifacts"]
    if any(not (args.package / name).is_file() or sha256(args.package / name) != digest
           for name, digest in wasm_artifacts.items()):
        parser.error("browser package differs from frozen WASM build")
    # Both endpoints explicitly choose this already qualified facility. No new
    # defaults or service scan is implied by running the member story.
    facility = {"logical_id": "mio73-explicit-public-emqx",
                "native_url": "mqtts://broker.emqx.io:8883/",
                "wss_url": "wss://broker.emqx.io:8084/mqtt"}
    report = {"scope": "formal Pixel6a Browser→Y700 Native SDK member with public MQTT contact",
              "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "command": ["rtk", "proxy", "python3", *sys.argv],
              "hostRole": "build, ADB and local page/control/evidence only",
              "publicMqtt": facility, "normalTlsVerification": True,
              "nativeOuterTlsQualified": False, "browserOuterWssTlsQualified": False,
              "contactSecurity": {"nativeOuterTls": True, "browserOuterWssTls": True,
                                  "protectedControl": "original end-to-end QUIC/TLS on both endpoints"},
              "nativeRoles": {"quic": "responder", "rtc": "answerer"},
              "browserRoles": {"quic": "initiator", "rtc": "offerer"},
              "independentPublicStunProbe": False, "turnConfigured": False,
              "physicalWanQualified": False, "passed": False, "stages": [],
              "buildReceiptSha256": sha256(args.build_receipt),
              "wasmBuildReceiptSha256": sha256(args.wasm_build_receipt),
              "nativeSourceBinding": validate_build_sources(args.build_receipt),
              "browserSourceBinding": validate_build_sources(args.wasm_build_receipt),
              "nativeDeploymentTransformation": deployment["transformation"],
              "browserPackageArtifacts": wasm_artifacts,
              "compiledNativeSha256": deployment["compiledSha256"],
              "deployedNativeSha256": deployment["deployedSha256"],
              "wasmSha256": sha256(args.package / "miopunch_browser_probe_bg.wasm"),
              "runnerSha256": sha256(__file__), "sourcesBefore": snapshot()}
    native_device = browser_device = actor = page = None
    browser_id = None

    def save():
        (args.output / "receipt.json").write_text(json.dumps(redacted(report), indent=2) + "\n")

    def stage(name, action):
        started = time.monotonic()
        try:
            result = action()
            report["stages"].append({"name": name, "ok": True,
                                      "elapsedMs": round((time.monotonic() - started) * 1000, 3)})
            save()
            print(json.dumps({"event": "stage", "name": name, "ok": True}), flush=True)
            return result
        except Exception as error:
            report["stages"].append({"name": name, "ok": False,
                                      "elapsedMs": round((time.monotonic() - started) * 1000, 3),
                                      "failureClass": type(error).__name__, "failure": str(error)})
            save()
            raise

    try:
        native_device = Device("TB322FC", "network")
        browser_device = Device("Pixel_6a", "usb")
        actor = NativeActor(native_device)
        actor.deploy(binary, "member", executable=True)
        with tempfile.TemporaryDirectory(prefix="mio73-public-input-") as directory:
            local = Path(directory)
            (local / "facility.json").write_text(json.dumps(facility))
            facility_remote = actor.deploy(local / "facility.json", "facility.json")
            profile_remote = actor.deploy(args.native_profile, "ice-profile.json")
        data_dir = actor.directory + "/saved-member"
        actor.start([data_dir, facility_remote, "-", "admission", "--ice-profile", profile_remote])
        first_ready = stage("native-admission-start", lambda: actor.wait_event("ready"))
        assert first_ready["normalPublicTrust"] is True
        report["nativeOuterTlsQualified"] = True
        page = BrowserPage(browser_device, args.package, environment_rtc_probe=False)
        stage("browser-owned-page-start", page.start)

        def configuration(ready):
            return {"relays": [args.relay] if args.relay else [], "disabled": [], "irc": [],
                    "mqtt": [facility], "rtc_ice": json.loads(args.browser_profile.read_text()),
                    "contact_routes": [{"contact": ready["id"], "facility": facility["logical_id"],
                                        "address": ready["route"]}]}

        first_browser = stage("browser-admission-start", lambda: checked(page.call(
            "start", {"configuration": configuration(first_ready)})))
        report["browserOuterWssTlsQualified"] = True
        browser_id = first_browser["id"]
        request = stage("request-admission", lambda: checked(page.call(
            "request_admission", {"invitation": first_ready["invitation"]})))
        assert request == "AwaitingApproval", request
        pending = actor.command("admission-status", "admission-status")
        requests = [entry for entry in pending["requests"] if entry["applicant"] == browser_id]
        assert len(requests) == 1
        stage("native-existing-approval", lambda: actor.command(
            "approve-request " + browser_id + " " + requests[0]["request"], "approved"))
        authorization = stage("authorization-save", lambda: checked(page.call(
            "request_admission", {"invitation": first_ready["invitation"]})))
        assert authorization == "PendingMaterials", authorization
        complete = stage("material-save", lambda: checked(page.call("complete_admission")))
        assert complete == "Complete", complete
        material = checked(page.call("material_receipt"))
        report["savedMaterials"] = {"stage": material["stage"],
                                   "authorizationSaved": material["authorization_saved"],
                                   "secretSaved": material["secret_saved"]}
        def stop_preparation():
            value = checked(page.call("stop"))
            assert (value["cleanupSucceeded"] and value["networkClosed"]
                    and value["networkFreed"] and value["ownedMemberHandles"] == 0)
            return value
        report["browserPreparationCleanup"] = stage("browser-preparation-stop", stop_preparation)
        preparation_close = close_native(actor, remove_directory=False)
        report["nativePreparationCleanup"] = preparation_close
        assert native_cleanup_succeeded(preparation_close, False)
        mode = ["unified-member", args.relay] if args.relay else ["unified-member"]
        actor.start([data_dir, facility_remote, "-", *mode, "--ice-profile", profile_remote])
        ready = stage("native-saved-member-reopen", lambda: actor.wait_event("ready"))
        assert ready["id"] == first_ready["id"] and browser_id in ready["knownMembers"]
        reopened = stage("browser-saved-member-reopen", lambda: checked(page.call(
            "start", {"configuration": configuration(ready)})))
        assert reopened["id"] == browser_id and reopened["status"]["stage"] == "Complete"
        report["persistedIdentitiesReopened"] = [True, True]
        response = stage("common-member-connect", lambda: page.call("open_member", {"target": ready["id"]}))
        report["connectResponse"] = response
        checked(response)
        released = actor.wait_event("member-released")
        assert released["alpn"] == MEMBER_ALPN and released["memberAuthorized"] is True
        assert released["remoteId"] == browser_id
        report["nativeReleased"] = redacted(released)
        before = checked(page.call("member_diagnostics"))
        report["browserBeforeBusiness"] = before
        assert before["newProtocol"] and before["expectedPeerMatched"] and before["memberAuthorized"]
        assert before["coordinator"]["freshAck"] is True
        reply = stage("business-stream", lambda: checked(page.call("exchange", {"message": "PING"})))
        assert reply == "PONG"
        report["bidirectionalStreamMatched"] = True
        report["browserAfterBusiness"] = checked(page.call("member_diagnostics"))
        report["nativeBusiness"] = redacted(actor.wait_event("business"))
        report["publicRestrictedSdkQualified"] = True
        report["nativeStatusBeforeClose"] = actor.command("status", "status")
        report["passed"] = True
    except Exception as error:
        report["failureClass"] = type(error).__name__
        report["failure"] = str(error)
        if page and page.ready:
            try:
                status = page.call("status")
                report["browserFailureStatus"] = redacted(status)
                if status.get("ok") is True and browser_id is not None:
                    report["browserFailureIdentityUnchanged"] = status["result"]["id"] == browser_id
                report["browserFailurePathDiagnostics"] = redacted(page.call("path_diagnostics"))
                report["browserFailureRtcDiagnostics"] = redacted(page.call("rtc_diagnostics"))
            except Exception as diagnostic_error:
                report["failureDiagnosticClass"] = type(diagnostic_error).__name__
    finally:
        report["cleanup"] = {}
        if page:
            try:
                report["cleanup"]["browser"] = page.close()
            except Exception as error:
                report["cleanup"]["browserFailureClass"] = type(error).__name__
            report["passed"] &= browser_cleanup_succeeded(report["cleanup"].get("browser", {}))
        if actor:
            try:
                report["cleanup"]["native"] = close_native(actor)
            except Exception as error:
                report["cleanup"]["nativeFailureClass"] = type(error).__name__
            report["passed"] &= native_cleanup_succeeded(report["cleanup"].get("native", {}))
            report["nativePendingEvents"] = redacted(actor.events)
            report["nativeErrors"] = native_errors(actor)
            report["nativeCommands"] = actor.runs
        report["deviceCommands"] = {
            "native": native_device.commands if native_device else [],
            "browser": browser_device.commands if browser_device else []}
        report["sourcesAfter"] = snapshot()
        report["sourceFrozenDuringRun"] = report["sourcesBefore"] == report["sourcesAfter"]
        # Execution uses copied native and WASM artifacts from the two frozen
        # builds. Concurrent edits to an unrelated source are recorded but do
        # not mutate those exact loaded programs or justify repeating the run.
        report["artifactsFrozenDuringRun"] = (sha256(binary) == deployment["deployedSha256"]
            and all((args.package / name).is_file() and sha256(args.package / name) == digest
                    for name, digest in wasm_artifacts.items()))
        report["passed"] &= report["artifactsFrozenDuringRun"]
        report["completedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["commandExit"] = 0 if report["passed"] else 1
        save()
        print(json.dumps({"event": "story-ended", "passed": report["passed"],
                          "exit": report["commandExit"], "receipt": str(args.output / "receipt.json")}), flush=True)
    return report["commandExit"]


if __name__ == "__main__":
    raise SystemExit(main())
