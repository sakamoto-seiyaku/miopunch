#!/usr/bin/env python3
"""STEP11 controlled separated networks; each fixture/work/cleanup clock<=120s."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

from browser_mqtt_trust import certificates, private_nss
from check_browser_rtc import ROOT, sha256, source_snapshot, reject_retired_fixture
from rtc_cross_network_process import OwnedProcess, public_event
from rtc_cross_network_evidence import validate as validate_network_evidence
from rtc_cross_network_topology import OwnedLab


def build(target, output):
    output.mkdir(parents=True, exist_ok=False)
    checks = []
    commands = [
        ("native", ["cargo", "build", "--locked", "--offline", "--features", "rtc-probe", "--jobs", "2", "--target-dir", str(target),
            "--bin", "browser_network_native", "--bin", "browser-probe-native", "--bin", "browser_wss_member_native"]),
        ("wasm", ["cargo", "build", "--locked", "--offline", "-p", "miopunch-browser-probe", "--features", "rtc-probe", "--jobs", "2",
            "--target", "wasm32-unknown-unknown", "--target-dir", str(target)]),
        ("bindgen", ["/tmp/miopunch-wasm-tools/bin/wasm-bindgen", "--target", "web", "--out-dir", str(ROOT/"browser-probe/pkg"),
            str(target/"wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm")]),
    ]
    for name, argv in commands:
        started = time.monotonic()
        with (output/(name+".log")).open("w") as log:
            completed = subprocess.run(["rtk", "proxy", *argv], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=600)
        checks.append({"name": name, "argv": argv, "exit": completed.returncode, "seconds": time.monotonic()-started})
        (output/"results.json").write_text(json.dumps({"scope": "builds outside each120s protocol clock", "commands": checks}, indent=2)+"\n")
        if completed.returncode:
            raise RuntimeError(f"{name} build failed; see {output}")


def artifacts(target):
    paths = [target/"debug/browser_network_native", target/"debug/browser-probe-native", target/"debug/browser_wss_member_native",
             ROOT/"browser-probe/pkg/miopunch_browser_probe.js", ROOT/"browser-probe/pkg/miopunch_browser_probe_bg.wasm"]
    return {str(path): sha256(path) for path in paths}


def trust(fixture):
    ca, leaf, key = certificates(fixture, dns_names=("localhost", "mio-mqtt.test", "mio-relay.test"))
    private_nss(fixture/"nssdb", ca)
    for source, name in ((ca, "ca.der"), (leaf, "leaf.der")):
        subprocess.run(["rtk", "proxy", "openssl", "x509", "-in", str(source), "-outform", "DER", "-out", str(fixture/name)], check=True, capture_output=True, timeout=3)
    subprocess.run(["rtk", "proxy", "openssl", "pkcs8", "-topk8", "-nocrypt", "-in", str(key), "-outform", "DER", "-out", str(fixture/"leaf-key.der")], check=True, capture_output=True, timeout=3)
    (fixture/"runtime").mkdir()


def stop_processes(processes, lab, receipt):
    """Share the bounded graceful head; preserve time for physical removal."""
    lab.stop_slots = max(1, sum(process.stopped is None for process in processes))
    for process in reversed(processes):
        pending = process.stopped is None
        try:
            result = process.stop()
            receipt["cleanup"].append(result)
            if result["exit"] != 0:
                receipt["passed"] = False
        except BaseException as error:
            receipt["cleanup"].append({"name": process.name, "error": str(error)})
            receipt["passed"] = False
        finally:
            if pending:
                lab.stop_slots = max(1, lab.stop_slots-1)


def member_case(output, target, mode, seed, source_record=None, expected_browser=None):
    output.mkdir(parents=True, exist_ok=False)
    temporary = None
    fixture = seed
    if mode != "prepare":
        temporary = tempfile.TemporaryDirectory(prefix="mio-rtc-s11-case-")
        fixture = Path(temporary.name)
    lab = OwnedLab(output, fixture, target, mode=mode)
    receipt = {"mode": mode, "scope": "controlled separated kernel networks, not physical WAN/cellular",
               "passed": False, "sourceBefore": source_snapshot(), "builtArtifactSet": artifacts(target), "cleanup": [],
               "executedArtifactPaths": [str(ROOT/"browser-probe/pkg/miopunch_browser_probe.js"), str(ROOT/"browser-probe/pkg/miopunch_browser_probe_bg.wasm"), str(target/"debug/browser_wss_member_native")] +
                   ([str(target/"debug/browser-probe-native")] if mode in ("prepare", "relay") else []) +
                   ([str(target/"debug/browser_network_native")] if mode == "prepare" else [])}
    processes = []
    native = None
    source = source_record
    story = None
    native_number = 0
    try:
        if mode == "prepare":
            trust(fixture)
        else:
            # Every measured case starts from the same closed, private seed.
            # No body/header/identity/material rewrite is permitted here.
            for item in seed.iterdir():
                destination = fixture/item.name
                if item.is_dir():
                    shutil.copytree(item, destination)
                else:
                    shutil.copy2(item, destination)
        lab.start()
        service = lab.topology["service"]
        broker = OwnedProcess(lab, "router", ["mosquitto", "-c", "/repo/scripts/mosquitto-test.conf"], "broker")
        processes.append(broker)
        broker.wait("owned-broker-started")
        services = OwnedProcess(lab, "router", ["/opt/node/bin/node", "browser-probe/harness/owned-cross-network-services.mjs"],
            "mqtt-services", environment=[("MIO_SERVICE_IP", service)])
        processes.append(services)
        facility = services.wait("services-ready")["facility"]
        (fixture/"facility.json").write_text(json.dumps(facility)+"\n")
        stun = OwnedProcess(lab, "router", ["python3", "/repo/scripts/stun_udp_observer.py", "--bind", service+":3478"], "stun")
        processes.append(stun)
        stun.wait("STUN_UDP_READY")
        relay_url = "https://mio-relay.test:8443/"
        if mode in ("prepare", "relay"):
            relay = OwnedProcess(lab, "router", ["/build/debug/browser-probe-native", "relay-tls-files", "/fixture/leaf.der", "/fixture/leaf-key.der", service+":8443", "mio-relay.test"], "relay")
            processes.append(relay)
            assert relay.wait("relay-ready")["url"] == relay_url
        outer_a = lab.topology["roles"]["nat-a"]["links"]["wan-a"]
        outer_b = lab.topology["roles"]["nat-b"]["links"]["wan-b"]
        profile = {"stun_servers": ["stun:"+service+":3478"], "candidate_addresses": [outer_a, outer_b]}
        (fixture/"ice-profile.json").write_text(json.dumps(profile)+"\n")
        browser_configuration = {"relays": [relay_url] if mode == "relay" else [], "disabled": [], "mqtt": [] if mode == "relay" else [facility],
            "irc": [], "contact_routes": [], "rtc_ice": {} if mode == "relay" else profile}
        browser = OwnedProcess(lab, "browser", ["/opt/node/bin/node", "browser-probe/harness/run-rtc-cross-network.mjs"], "browser-driver",
            environment=[("LD_LIBRARY_PATH", "/opt/hostlib")])
        processes.append(browser)
        browser.send(json.dumps({"configure": {"mode": mode, "browser": browser_configuration, "relay": relay_url,
            "sourceRecord": source_record, "workMs": max(1, int((lab.remaining()-1)*1000))}}))
        def dispatch(method, payload):
            nonlocal native, native_number
            if method == "native-start":
                if native and native.stopped is None:
                    raise RuntimeError("owned native already active")
                kind = payload["kind"]
                native_number += 1
                if kind == "prepare":
                    argv = ["/build/debug/browser_network_native", "/fixture/runtime/admin", relay_url, "/fixture/ca.der"]
                else:
                    argv = ["/build/debug/browser_wss_member_native", "/fixture/runtime/admin", "/fixture/facility.json", "/fixture/ca.pem"]
                    if kind == "direct":
                        argv += ["--ice-profile", "/fixture/ice-profile.json"]
                    elif kind == "relay":
                        argv += ["legacy-relay", relay_url]
                    elif kind != "record":
                        raise RuntimeError("unknown native fixture kind")
                wrapped = ["python3", "/repo/scripts/nat_connection_process.py", "--limit", str(max(1, lab.remaining()-2)),
                           f"/evidence/native-accounting-{native_number}.json", "--", *argv]
                native = OwnedProcess(lab, "native", wrapped, "native-"+str(native_number))
                processes.append(native)
                ready = native.wait("ready")
                if source_record is not None and ready["id"] != source_record["author"]:
                    raise RuntimeError("native clone does not retain exact source author")
                return ready
            if method == "native-stop":
                if native is None:
                    raise RuntimeError("native stop without owned process")
                return native.stop()
            if native is None or native.stopped is not None:
                raise RuntimeError("native RPC without active owned process")
            if method == "native-command":
                if (payload["command"], payload["event"]) not in (("approve", "approved"), ("record", "record"), ("status", "status")):
                    raise RuntimeError("native RPC command outside bounded fixture contract")
                return native.command(payload["command"], payload["event"])
            if method == "native-wait":
                if payload["event"] not in ("member-released", "business", "replica-finished", "rtc-path-cleanup"):
                    raise RuntimeError("native RPC event outside fixture contract")
                return native.wait(payload["event"])
            if method == "blocked-cleanup" and mode == "blocked":
                return native.wait("rtc-path-cleanup", budget=2)
            if method == "rtc-report":
                native.read()
                matches = [entry["report"] for entry in native.entries if entry.get("event") == "rtc-path" and entry["report"]["memberStableId"] == payload["stableId"]]
                if len(matches) != 1:
                    raise RuntimeError("exact original-M native final RTC actor report absent/ambiguous")
                return matches[0]
            if method == "no-ordinary-release":
                native.read()
                return not any(entry.get("event") in ("member-released", "business", "replica-finished") for entry in native.entries)
            raise RuntimeError("unknown owned controller method")
        completed = None
        while completed is None:
            lab.remaining()
            browser.read(0.05)
            while browser.pending:
                entry = browser.pending.pop(0)
                if entry["event"] == "browser-complete":
                    completed = entry
                    break
                if entry["event"] != "rpc":
                    raise RuntimeError("unknown browser driver event")
                try:
                    result = dispatch(entry["method"], entry["payload"])
                    browser.send(json.dumps({"reply": entry["id"], "result": result}))
                except BaseException as error:
                    browser.send(json.dumps({"reply": entry["id"], "error": str(error)}))
            if browser.process.poll() is not None and completed is None:
                browser.read()
                if not browser.pending:
                    raise RuntimeError("browser driver exited before completion")
        story = json.loads((output/"rtc-cross-network.json").read_text())
        receipt["storyPassed"] = completed["passed"] and story["passed"]
        if not receipt["storyPassed"]:
            raise RuntimeError("actual member SDK story failed: "+story.get("failure", "unknown"))
        if expected_browser is not None and story["browserIdentity"] != expected_browser:
            raise RuntimeError("browser clone does not retain exact prepared identity")
        if mode == "prepare":
            source = story["sourceRecord"]
            receipt["sourceRecord"] = source
            receipt["browserIdentity"] = story["browserIdentity"]
        else:
            receipt["sourceRecord"] = source_record
        services.command("snapshot", "services-snapshot")
        stun.read()
        receipt["stunObservations"] = [entry for entry in stun.entries if entry.get("event") == "STUN_UDP_BINDING"]
        lab.snapshot("namespace-after-member")
        receipt["passed"] = True
    except BaseException as error:
        receipt["failure"] = str(error)
    finally:
        stop_processes(processes, lab, receipt)
        if not lab.close():
            receipt["passed"] = False
        receipt["cleanup"].extend(lab.cleanup_results)
        if mode != "prepare" and receipt["passed"]:
            try:
                receipt["networkEvidence"] = validate_network_evidence(output, mode, story, receipt["stunObservations"])
                costs = {path.stem[5:]: json.loads(path.read_text()) for path in output.glob("cost-*.json")}
                expected_costs = {"browser-driver", "mqtt-services", "stun", "broker"} | ({"relay"} if mode == "relay" else set())
                if costs.keys() != expected_costs:
                    raise RuntimeError("browser/harness/broker/STUN/relay cost set incomplete")
                browser_cost = costs["browser-driver"]["groups"]["browser"]
                if not browser_cost["processes"] or browser_cost["peakSimultaneousRssBytes"] <= 0 or any(value["exit"] != 0 for value in costs.values()):
                    raise RuntimeError("actual browser/service process accounting absent/incomplete")
                native_cost = json.loads((output/"native-accounting-1.json").read_text())
                if native_cost["exit"] != 0 or native_cost["timed_out"] or not native_cost["exec_samples"]:
                    raise RuntimeError("actual native wait4 accounting incomplete")
                receipt["cost"] = {"native": native_cost, "sampledProcesses": costs}
            except BaseException as error:
                receipt["qualificationFailure"] = str(error)
                receipt["passed"] = False
        if temporary:
            try:
                temporary.cleanup()
            except BaseException as error:
                receipt["cleanup"].append({"name": "private-case-remove", "error": str(error)})
                receipt["passed"] = False
        receipt["totalProtocolSeconds"] = time.monotonic()-lab.started
        receipt["sourceAfter"] = source_snapshot()
        receipt["sourceUnchanged"] = receipt["sourceBefore"] == receipt["sourceAfter"]
        receipt["builtArtifactSetAfter"] = artifacts(target)
        receipt["passed"] &= receipt["sourceUnchanged"] and receipt["builtArtifactSet"] == receipt["builtArtifactSetAfter"] and receipt["totalProtocolSeconds"] <= 120
        (output/"results.json").write_text(json.dumps(public_event(receipt), indent=2)+"\n")
    return receipt


def prerequisite(output, target):
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"scope": "STEP11 actual container/TLS/STUN prerequisite, not native ICE/member/WAN qualification",
               "passed": False, "sourceBefore": source_snapshot(), "cleanup": []}
    fixture = tempfile.TemporaryDirectory(prefix="mio-rtc-s11-prerequisite-")
    fixture_path = Path(fixture.name)
    lab = OwnedLab(output, fixture_path, target, mode="prerequisites")
    processes = []
    try:
        ca, leaf, key = certificates(fixture_path, dns_names=("localhost", "mio-mqtt.test", "mio-relay.test"))
        private_nss(fixture_path/"nssdb", ca)
        (fixture_path/"runtime").mkdir()
        for source, name in ((ca, "ca.der"), (leaf, "leaf.der")):
            lab.command("certificate-"+name, ["openssl", "x509", "-in", str(source), "-outform", "DER", "-out", str(fixture_path/name)])
        lab.command("fixture-pkcs8", ["openssl", "pkcs8", "-topk8", "-nocrypt", "-in", str(key), "-outform", "DER", "-out", str(fixture_path/"leaf-key.der")])
        receipt["certificate"] = {"normalVerification": True, "privateNssOnly": True, "caSha256": sha256(ca)}
        lab.start()
        service = lab.topology["service"]
        browser_outer = lab.topology["roles"]["nat-a"]["links"]["wan-a"]
        receipt["browserUserEnvironment"] = json.loads(lab.exec("browser", ["python3", "-c",
            "import os,pwd,json,pathlib; print(json.dumps({'uid':os.getuid(),'homeEnvironment':os.environ.get('HOME'),'homeExpanded':str(pathlib.Path.home()),'passwdHome':[p.pw_dir for p in pwd.getpwall() if p.pw_uid==os.getuid()],'xdgConfig':os.environ.get('XDG_CONFIG_HOME'),'paths':{p:os.access(p,os.W_OK) for p in ('/root','/root/.config','/root/.cache','/.config','/fixture/runtime')}}))"],
            name="browser-user-environment", user=f"{os.getuid()}:{os.getgid()}", capture=True))
        lab.exec("browser", ["env", "LD_LIBRARY_PATH=/opt/hostlib", "/opt/chromium/chrome", "--version"], name="browser-load", user=f"{os.getuid()}:{os.getgid()}")
        lab.exec("router", ["mosquitto", "-c", "/repo/scripts/mosquitto-test.conf", "-d"], name="broker-start", user=f"{os.getuid()}:{os.getgid()}")
        services = OwnedProcess(lab, "router", ["/opt/node/bin/node", "browser-probe/harness/owned-cross-network-services.mjs"], "mqtt-services", environment=[("MIO_SERVICE_IP", service)])
        processes.append(services)
        receipt["facility"] = services.wait("services-ready")["facility"]
        stun = OwnedProcess(lab, "router", ["python3", "/repo/scripts/stun_udp_observer.py", "--bind", service+":3478"], "stun")
        processes.append(stun)
        receipt["stunReady"] = stun.wait("STUN_UDP_READY")
        lab.exec("native", ["openssl", "s_client", "-connect", "mio-mqtt.test:8883", "-CAfile", "/fixture/ca.pem",
                "-verify_hostname", "mio-mqtt.test", "-verify_return_error", "-brief"], name="native-tls", user=f"{os.getuid()}:{os.getgid()}")
        browser = OwnedProcess(lab, "browser", ["/opt/node/bin/node", "browser-probe/harness/cross-network-preflight.mjs"],
            "browser-prerequisite", environment=[("MIO_SERVICE_IP", service), ("MIO_BROWSER_OUTER_IP", browser_outer), ("LD_LIBRARY_PATH", "/opt/hostlib")])
        processes.append(browser)
        receipt["browser"] = browser.wait("browser-prerequisite", budget=min(20, lab.remaining()))
        if not receipt["browser"]["passed"]:
            raise RuntimeError("actual browser TLS/STUN prerequisite failed")
        stun.read()
        receipt["stunObservations"] = [entry for entry in stun.entries if entry["event"] == "STUN_UDP_BINDING"]
        if not any(entry["actualSource"][0] == browser_outer for entry in receipt["stunObservations"]):
            raise RuntimeError("STUN did not observe translated browser source")
        services.command("snapshot", "services-snapshot")
        lab.snapshot("namespace-after-prerequisite")
        receipt["passed"] = True
    except BaseException as error:
        receipt["failure"] = str(error)
    finally:
        stop_processes(processes, lab, receipt)
        if not lab.close():
            receipt["passed"] = False
        receipt["cleanup"].extend(lab.cleanup_results)
        try:
            fixture.cleanup()
        except BaseException as error:
            receipt["cleanup"].append({"name": "private-fixture-remove", "error": str(error)})
            receipt["passed"] = False
        receipt["totalProtocolSeconds"] = time.monotonic()-lab.started
        receipt["sourceAfter"] = source_snapshot()
        receipt["sourceUnchanged"] = receipt["sourceBefore"] == receipt["sourceAfter"]
        receipt["passed"] &= receipt["sourceUnchanged"] and receipt["totalProtocolSeconds"] <= 120
        (output/"results.json").write_text(json.dumps(receipt, indent=2)+"\n")
    return receipt["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=ROOT/"target")
    parser.add_argument("--prerequisites-only", action="store_true")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--mode", choices=("all", "direct", "blocked", "relay"), default="all")
    args = parser.parse_args()
    reject_retired_fixture()
    def interrupted(signum, frame):
        raise KeyboardInterrupt("owned STEP11 runner cancelled")
    signal.signal(signal.SIGTERM, interrupted)
    if args.prerequisites_only:
        return 0 if prerequisite(args.output.resolve(), args.target_dir.resolve()) else 1
    output, target = args.output.resolve(), args.target_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    if not args.skip_build:
        build(target, output/"build")
    receipt = {"case": "rtc-cross-network", "scope": "controlled kernel topology only; " + ("one direct/relay comparison repetition and peer-UDP-negative" if args.mode == "all" else "diagnostic single condition: "+args.mode) + "; no WAN success-rate claim", "passed": False,
               "sourceBefore": source_snapshot(), "builtArtifactSet": artifacts(target), "cases": []}
    with tempfile.TemporaryDirectory(prefix="mio-rtc-s11-immutable-seed-") as directory:
        seed = Path(directory)
        prepared = member_case(output/"prepare", target, "prepare", seed)
        receipt["preparation"] = prepared
        if prepared["passed"]:
            receipt["immutableClosedSeedFiles"] = {str(path.relative_to(seed)): sha256(path) for path in seed.rglob("*") if path.is_file() and "key" not in path.name and path.name != "leaf-key.der"}
            modes = ("direct", "blocked", "relay") if args.mode == "all" else (args.mode,)
            for index, mode in enumerate(modes):
                # Spacing applies to repeated cases; this first iteration has
                # one case of each condition rather than invented success rates.
                result = member_case(output/mode, target, mode, seed, prepared["sourceRecord"], prepared["browserIdentity"])
                receipt["cases"].append(result)
                if not result["passed"]:
                    break
            receipt["passed"] = all(case["passed"] for case in receipt["cases"]) and len(receipt["cases"]) == len(modes)
            if args.mode == "all" and receipt["passed"]:
                receipt["comparison"] = {case["mode"]: {"metrics": json.loads((output/case["mode"]/"rtc-cross-network.json").read_text())["metrics"], "cost": case["cost"],
                    "wire": case["networkEvidence"]["endpointWire"]} for case in receipt["cases"]}
                receipt["comparisonLimits"] = ["one repetition, ordered direct/blocked/relay; no success-rate estimate",
                    "same closed seed/author/record/browser identity/built artifacts/TLS/CC/topology; fresh case clones",
                    "one36B body, save-inclusive finite transfer is noisy and not sustained throughput",
                    "shared host without CPU isolation or artificial link delay; not a WAN performance/benefit claim"]
            receipt["immutableSeedUnchanged"] = receipt["immutableClosedSeedFiles"] == {str(path.relative_to(seed)): sha256(path) for path in seed.rglob("*") if path.is_file() and "key" not in path.name and path.name != "leaf-key.der"}
            receipt["passed"] &= receipt["immutableSeedUnchanged"]
            receipt["qualifiedConditions"] = [case["mode"] for case in receipt["cases"] if case["passed"]]
            receipt["allConditionsQualified"] = receipt["passed"] and set(receipt["qualifiedConditions"]) == {"direct", "blocked", "relay"}
    receipt["sourceAfter"] = source_snapshot()
    receipt["sourceUnchanged"] = receipt["sourceBefore"] == receipt["sourceAfter"]
    receipt["passed"] &= receipt["sourceUnchanged"]
    (output/"results.json").write_text(json.dumps(public_event(receipt), indent=2)+"\n")
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
