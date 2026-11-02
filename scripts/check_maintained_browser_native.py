#!/usr/bin/env python3
"""Prepared MIO78 asymmetrical maintained-Node public MQTT→RTC story.

This runner requires a future explicitly reviewed `maintained-unified-member`
fixture mode. The current independent Endpoint mode is deliberately rejected.
It creates no device/public traffic on import; root owns its serial run window.
Reuse the existing frozen WASM package; only the new maintained native fixture
needs an incremental Android build after its source boundary is released.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import time

from android_pair_runtime import BrowserPage, Device, NativeActor, browser_cleanup_succeeded
from build_android_pair import sources
from check_unified_browser_native import (
    ROOT, MEMBER_ALPN, checked, close_native, native_errors, redacted, sha256,
    validate_build_sources, native_cleanup_succeeded as endpoint_cleanup_succeeded,
)


def snapshot():
    return sources([*(ROOT / "browser-probe/src").rglob("*"),
                    ROOT / "browser-probe/Cargo.toml", Path(__file__),
                    ROOT / "scripts/check_unified_browser_native.py",
                    ROOT / "scripts/android_pair_runtime.py",
                    ROOT / "browser-probe/harness/android-pair-page.js"])


def validate_maintained_build_sources(receipt):
    binding = validate_build_sources(receipt)
    active = {"src/network_maintenance.rs", "src/contact_listener.rs",
              "src/mqtt_transport.rs", "src/resources.rs", "src/native_network.rs"}
    if active & binding["inactiveCurrentSourceDeltas"].keys():
        raise RuntimeError("maintained-node source differs from the native build")
    return binding


def count(value):
    return type(value) is int and value >= 0


def require_maintained_ready(value, public_trust_required=True):
    if value.get("maintenanceOwnedByNode") is not True:
        raise RuntimeError("fixture did not report a real NodeMaintenance owner")
    listener = value.get("maintenanceStatus", {}).get("contact_listener", {})
    traffic = value.get("listenerTraffic", {})
    protocol = traffic.get("protocol", {})
    if (listener.get("phase") != "Ready" or not count(listener.get("generation"))
            or listener["generation"] == 0 or not count(listener.get("ready_routes"))
            or listener["ready_routes"] == 0 or traffic.get("driver_active") != 1
            or not all(count(listener.get(name)) for name in ("starts", "withdrawals"))
            or not count(traffic.get("driver_active"))
            or protocol.get("connack") != 0 or type(protocol.get("connack")) is not int
            or protocol.get("suback_codes") != [0]
            or any(type(code) is not int for code in protocol.get("suback_codes", []))
            or value.get("relayOnline") is not True):
        raise RuntimeError("actual maintained listener/relay readiness is missing")
    if public_trust_required and value.get("normalPublicTrust") is not True:
        raise RuntimeError("native normal public TLS trust was not observed")
    return listener


def require_selected_rtc(value):
    if not any(path.get("selected") is True and path.get("isRtc") is True
               and path.get("isSignal") is not True and path.get("isRelay") is not True
               for path in value.get("paths", [])):
        raise RuntimeError("ordinary M did not select a qualified RTC data path")


def native_cleanup_succeeded(value):
    if not endpoint_cleanup_succeeded(value):
        return False
    sdk = value.get("sdkRetirement", {})
    listener = sdk.get("maintenanceStatus", {}).get("contact_listener", {})
    traffic = sdk.get("listenerTraffic", {})
    return (sdk.get("maintenanceOwnedByNode") is True
            and sdk.get("maintenanceStatus", {}).get("state") == "Stopped"
            and listener.get("phase") == "Stopped" and count(listener.get("ready_routes"))
            and listener["ready_routes"] == 0
            and count(traffic.get("driver_active")) and traffic["driver_active"] == 0
            and count(traffic.get("queued_packets")) and traffic["queued_packets"] == 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--wasm-build-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--native-profile", type=Path, required=True)
    parser.add_argument("--browser-profile", type=Path, required=True)
    parser.add_argument("--native-relay", required=True, help="explicit native-only permitted healthy data relay")
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
    facility = {"logical_id": "mio78-explicit-public-emqx",
                "native_url": "mqtts://broker.emqx.io:8883/",
                "wss_url": "wss://broker.emqx.io:8084/mqtt"}
    report = {"scope": "formal Pixel6a browser without relay → Y700 maintained Node with healthy relay; public MQTT restricted contact → RTC data",
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
              "nativeSourceBinding": validate_maintained_build_sources(args.build_receipt),
              "browserSourceBinding": validate_build_sources(args.wasm_build_receipt),
              "nativeDeploymentTransformation": deployment["transformation"],
              "browserPackageArtifacts": wasm_artifacts,
              "compiledNativeSha256": deployment["compiledSha256"],
              "deployedNativeSha256": deployment["deployedSha256"],
              "wasmSha256": sha256(args.package / "miopunch_browser_probe_bg.wasm"),
              "runnerSha256": sha256(__file__), "sourceContract": "maintained-unified-member plus actual Node status/traffic getters", "nativeOnlyRelay": args.native_relay, "sourcesBefore": snapshot()}
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
            return {"relays": [], "disabled": [], "irc": [],
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
        assert endpoint_cleanup_succeeded(preparation_close, False)
        mode = ["maintained-unified-member", args.native_relay]
        actor.start([data_dir, facility_remote, "-", *mode, "--ice-profile", profile_remote])
        ready = stage("native-saved-member-reopen", lambda: actor.wait_event("ready"))
        assert ready["id"] == first_ready["id"] and browser_id in ready["knownMembers"]
        require_maintained_ready(ready)
        report["maintainedNodeBeforeContact"] = redacted(ready)
        reopened = stage("browser-saved-member-reopen", lambda: checked(page.call(
            "start", {"configuration": configuration(ready)})))
        assert reopened["id"] == browser_id and reopened["status"]["stage"] == "Complete"
        report["persistedIdentitiesReopened"] = [True, True]
        caller_started = time.monotonic()
        response = stage("common-member-connect", lambda: page.call("open_member", {"target": ready["id"]}))
        report["callerMs"] = (time.monotonic() - caller_started) * 1000
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
        require_selected_rtc(before)
        native_proof = actor.wait_event("unified-path-ready")
        assert (native_proof["freshAck"] is True and native_proof["role"] == "responder"
                and native_proof["rtcRole"] == "Answerer" and native_proof["version"] == 1)
        report["nativeCurrentMProof"] = redacted(native_proof)
        reply = stage("business-stream", lambda: checked(page.call("exchange", {"message": "PING"})))
        assert reply == "PONG"
        report["bidirectionalStreamMatched"] = True
        report["browserAfterBusiness"] = checked(page.call("member_diagnostics"))
        report["nativeBusiness"] = redacted(actor.wait_event("business"))
        report["publicRestrictedSdkQualified"] = True
        report["nativeStatusBeforeClose"] = actor.command("status", "status")
        require_maintained_ready(report["nativeStatusBeforeClose"], public_trust_required=False)
        report["nodeStayedRunningDuringContactAndBusiness"] = True
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
                cleanup_started = time.monotonic()
                report["cleanup"]["native"] = close_native(actor)
                report["nativeCleanupMs"] = (time.monotonic() - cleanup_started) * 1000
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
