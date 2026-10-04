#!/usr/bin/env python3
"""STEP7: one owned namespace, actual selected-RTC UDP DROP, <=60s including cleanup."""
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
from check_browser_rtc import ROOT, sha256, source_snapshot

IMAGE = "sha256:c9dbb88a37f58ae5105720e60bc5cc6eb40d8ad37106e57ce621d345b5ec62a2"


def cleanup_command(receipt, name, argv, budget):
    try:
        result = subprocess.run(["rtk", "proxy", *argv], capture_output=True, text=True, timeout=budget)
        entry = {"name": name, "exit": result.returncode}
        receipt["cleanup"].append(entry)
        if result.returncode:
            receipt["passed"] = False
        if name.endswith("-absent"):
            entry["empty"] = result.returncode == 0 and not result.stdout.strip()
            if not entry["empty"]:
                receipt["passed"] = False
    except BaseException as error:
        receipt["cleanup"].append({"name": name, "error": str(error)})
        # No completed query means no absence proof. Still attempt every
        # subsequent exact owned removal within the original cleanup clock.
        receipt["passed"] = False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=("relay", "none"), default="relay")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--wasm-bindgen", default=shutil.which("wasm-bindgen") or "/tmp/miopunch-wasm-tools/bin/wasm-bindgen")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    target = args.target_dir.resolve()
    receipt = {"case": "rtc-data-cut", "mode": args.mode, "passed": False, "image": IMAGE,
               "sourceBefore": source_snapshot(), "commands": [], "cleanup": [],
               "totalBudgetSeconds": 60, "workBudgetSeconds": 45, "cleanupReserveSeconds": 15,
               "scope": "one owned kernel namespace; not NAT or WAN qualification"}

    def save():
        (output / "results.json").write_text(json.dumps(receipt, indent=2) + "\n")

    def command(name, argv, budget, *, capture=False, expected=(0,)):
        started = time.monotonic()
        with (output / f"{name}.log").open("w") as log:
            process = subprocess.Popen(["rtk", "proxy", *argv], cwd=ROOT,
                stdout=subprocess.PIPE if capture else log, stderr=log, text=True, start_new_session=True)
            try:
                stdout, _ = process.communicate(timeout=max(0.001, budget))
            except BaseException:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.communicate(timeout=min(1, max(0.001, budget)))
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.communicate()
                raise
            entry = {"name": name, "argv": argv, "exit": process.returncode, "expectedExitCodes": list(expected), "seconds": round(time.monotonic()-started, 3)}
            receipt["commands"].append(entry)
            if capture:
                log.write(stdout or "")
            save()
            if process.returncode not in expected:
                raise RuntimeError(f"{name} failed: see {output / (name+'.log')}")
            return stdout

    def interrupted(signum, frame):
        raise KeyboardInterrupt("owned STEP7 runner cancelled")

    signal.signal(signal.SIGTERM, interrupted)
    network = None
    container = None
    fixture = None
    protocol_started = None
    try:
        command("revision", ["git", "rev-parse", "HEAD"], 3)
        if not args.skip_build:
            command("native-build", ["cargo", "build", "--locked", "--offline", "-j", "2", "--features", "rtc-probe",
                "--bin", "browser_network_native", "--bin", "browser-probe-native", "--bin", "browser_wss_member_native", "--target-dir", str(target)], 600)
            command("wasm-build", ["cargo", "build", "--locked", "--offline", "-j", "2", "-p", "miopunch-browser-probe",
                "--features", "rtc-probe", "--target", "wasm32-unknown-unknown", "--target-dir", str(target)], 600)
            command("bindgen", [args.wasm_bindgen, "--target", "web", "--out-dir", str(ROOT / "browser-probe/pkg"),
                str(target / "wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm")], 30)
        binaries = [target / "debug/browser_network_native", target / "debug/browser-probe-native", target / "debug/browser_wss_member_native",
                    ROOT / "browser-probe/pkg/miopunch_browser_probe.js", ROOT / "browser-probe/pkg/miopunch_browser_probe_bg.wasm"]
        receipt["builtArtifacts"] = {str(path): sha256(path) for path in binaries}
        node = Path(shutil.which("node")).resolve().parent.parent
        rtk = Path(shutil.which("rtk")).resolve()
        chrome = next((Path.home() / ".cache/ms-playwright").glob("chromium-1243/chrome-linux64"))
        # Fixed private image and only its new internal bridge/container; no host
        # firewall, published port, Docker socket, credentials or HOME mount.
        command("image-identity", ["docker", "image", "inspect", IMAGE, "--format", "{{.Id}}"], 3)
        protocol_started = time.monotonic()
        fixture = tempfile.TemporaryDirectory(prefix="mio-rtc-s7-")
        fixture_path = Path(fixture.name)

        def remaining():
            value = 45 - (time.monotonic()-protocol_started)
            if value <= 0:
                raise TimeoutError("owned STEP7 work budget ended; cleanup reserve starts")
            return value

        ca, leaf, key = certificates(fixture_path)
        private_nss(fixture_path / "nssdb", ca)
        for source, name in ((ca, "ca.der"), (leaf, "leaf.der")):
            command("certificate-"+name, ["openssl", "x509", "-in", str(source), "-outform", "DER", "-out", str(fixture_path / name)], min(3, remaining()))
        command("fixture-pkcs8", ["openssl", "pkcs8", "-topk8", "-nocrypt", "-in", str(key), "-outform", "DER", "-out", str(fixture_path / "leaf-key.der")], min(3, remaining()))
        receipt["certificate"] = {"normalVerification": True, "privateNssOnly": True, "caSha256": sha256(ca)}
        container = "mio-rtc-s7-" + str(time.time_ns())
        network = container + "-network"
        command("network-create", ["docker", "network", "create", "--internal", "--label", "miopunch.run="+container, network], min(3, remaining()))
        mounts = [(ROOT, "/repo", True), (target, "/build", True), (node, "/opt/node", True), (chrome, "/opt/chromium", True),
                  (rtk, "/usr/local/bin/rtk", True), (Path("/usr/lib/x86_64-linux-gnu"), "/opt/hostlib", True),
                  (fixture_path, "/fixture", False), (fixture_path / "nssdb", "/root/.pki/nssdb", False), (output, "/evidence", False)]
        argv = ["docker", "run", "--detach", "--name", container, "--network", network, "--label", "miopunch.run="+container,
                "--cap-add", "NET_ADMIN", "--cap-add", "NET_RAW", "--workdir", "/repo",
                "--env", "PATH=/opt/node/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "--env", "PLAYWRIGHT_CHROMIUM_EXECUTABLE=/opt/chromium/chrome", "--env", "MIO_PRIVATE_NSS_NAMESPACE=1",
                "--env", "MIO_OWNED_CUT_ID="+container]
        for source, destination, readonly in mounts:
            argv += ["--mount", f"type=bind,src={source},target={destination}"+(",readonly" if readonly else "")]
        argv += ["--entrypoint", "/bin/sleep", IMAGE, "58"]
        command("container-start", argv, min(3, remaining()))
        receipt["ownedContainer"] = container
        receipt["ownedNetwork"] = network
        command("namespace-tools", ["docker", "exec", container, "rtk", "proxy", "ss", "-uapn"], min(2, remaining()))
        addresses = json.loads(command("namespace-addresses", ["docker", "exec", container, "rtk", "proxy", "ip", "-j", "address"], min(2, remaining()), capture=True))
        command("namespace-routes-before", ["docker", "exec", container, "rtk", "proxy", "ip", "-j", "route"], min(2, remaining()))
        # Match the fixed native mDNS driver's actual Linux socket syscall.
        # An internal bridge has no default route; joining on INADDR_ANY may
        # therefore fail before the native driver binds any ICE data socket.
        probe = """import socket,json,sys
s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEPORT,1)
s.setblocking(False)
try:
 s.bind(('224.0.0.251',5353))
 s.setsockopt(socket.IPPROTO_IP,socket.IP_ADD_MEMBERSHIP,socket.inet_aton('224.0.0.251')+socket.inet_aton('0.0.0.0'))
 print(json.dumps({'joined':True}))
except OSError as error:
 print(json.dumps({'joined':False,'errno':error.errno}));sys.exit(1)
finally:s.close()
"""
        observed = json.loads(command("mdns-join-before", ["docker", "exec", container, "rtk", "proxy", "python3", "-c", probe], min(2, remaining()), capture=True, expected=(0, 1)))
        receipt["mdnsJoinBefore"] = observed
        if not observed["joined"]:
            if observed.get("errno") != 19:
                raise RuntimeError("native multicast prerequisite failed for an unconfirmed reason")
            interfaces = [entry["ifname"] for entry in addresses if entry["ifname"] != "lo" and any(address.get("family") == "inet" for address in entry["addr_info"])]
            if len(interfaces) != 1:
                raise RuntimeError("owned fixture must have one actual IPv4 interface")
            command("owned-multicast-route", ["docker", "exec", container, "rtk", "proxy", "ip", "route", "add", "224.0.0.251/32", "dev", interfaces[0]], min(2, remaining()))
            receipt["mdnsJoinAfter"] = json.loads(command("mdns-join-after", ["docker", "exec", container, "rtk", "proxy", "python3", "-c", probe], min(2, remaining()), capture=True))
            command("namespace-routes-after", ["docker", "exec", container, "rtk", "proxy", "ip", "-j", "route"], min(2, remaining()))
        command("browser-load", ["docker", "exec", "--env", "LD_LIBRARY_PATH=/opt/hostlib", container, "/opt/chromium/chrome", "--version"], min(2, remaining()))
        command("broker-start", ["docker", "exec", container, "rtk", "proxy", "mosquitto", "-c", "/repo/scripts/mosquitto-test.conf", "-d"], min(2, remaining()))
        work_ms = max(1, int((remaining()-0.5)*1000))
        command("rtc-data-cut", ["docker", "exec", "--env", f"MIO_CUT_WORK_MS={work_ms}", container, "rtk", "proxy", "/opt/node/bin/node",
                "browser-probe/harness/run-rtc.mjs", "--case", "rtc-data-cut", "--cut-mode", args.mode,
                "--relay", "owned", "--fixture", "/fixture", "--evidence", "/evidence", "--broker", "127.0.0.1:1883",
                "--native", "/build/debug/browser_network_native", "--wss-native", "/build/debug/browser_wss_member_native", "--relay-fixture", "/build/debug/browser-probe-native"], remaining())
        actual = json.loads((output / "rtc-data-cut.json").read_text())
        if not actual["passed"] or not actual["cut"]["tableRemoved"]:
            raise RuntimeError("kernel cut story/owned rule cleanup did not pass")
        receipt["passed"] = True
    except BaseException as error:
        receipt["failure"] = str(error)
        receipt["passed"] = False
        raise
    finally:
        # All cleanup uses the original 60-second clock. Collection failures
        # never skip exact owned removal. No prune/flush or shared cache edits.
        if protocol_started is not None:
            def cleanup(name, argv):
                budget = max(0.001, min(3, 60 - (time.monotonic()-protocol_started)))
                cleanup_command(receipt, name, argv, budget)
            if container:
                cleanup("owned-files-owner", ["docker", "exec", container, "chown", "-R", f"{os.getuid()}:{os.getgid()}", "/fixture", "/evidence"])
                cleanup("container-remove", ["docker", "rm", "--force", container])
                cleanup("container-absent", ["docker", "ps", "-a", "--filter", "label=miopunch.run="+container, "--format", "{{.ID}}"])
            if network:
                cleanup("network-remove", ["docker", "network", "rm", network])
                cleanup("network-absent", ["docker", "network", "ls", "--filter", "label=miopunch.run="+container, "--format", "{{.ID}}"])
            receipt["totalProtocolSeconds"] = round(time.monotonic()-protocol_started, 3)
            if receipt["totalProtocolSeconds"] > 60:
                receipt["passed"] = False
        if fixture:
            try:
                fixture.cleanup()
            except BaseException as error:
                receipt["cleanup"].append({"name": "temporary-directory-remove", "error": str(error)})
                receipt["passed"] = False
        if protocol_started is not None:
            receipt["totalProtocolSeconds"] = round(time.monotonic()-protocol_started, 3)
            if receipt["totalProtocolSeconds"] > 60:
                receipt["passed"] = False
        receipt["sourceAfter"] = source_snapshot()
        receipt["sourceUnchanged"] = receipt["sourceBefore"] == receipt["sourceAfter"]
        if not receipt["sourceUnchanged"]:
            receipt["passed"] = False
        save()
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
