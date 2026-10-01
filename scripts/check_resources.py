#!/usr/bin/env python3
"""Source-fixed resource verification; preserve every real command exit."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import socket
import time

ROOT = Path(__file__).resolve().parents[1]


def snapshot(output):
    sources = sorted({path.relative_to(ROOT).as_posix() for prefix in ["src", "tests", "browser-probe/src", "vendor/iroh/src", "vendor/p2panda-net/src", "vendor/p2panda-auth/src", "vendor/noq/src", "vendor/noq-proto/src", "vendor/netwatch/src"] for path in (ROOT / prefix).rglob("*.rs")})
    sources += ["Cargo.toml", "Cargo.lock", "browser-probe/Cargo.toml", "scripts/check_resources.py", "scripts/mosquitto-test.conf"]
    sources += [path.relative_to(ROOT).as_posix() for path in (ROOT / "vendor").glob("*/Cargo.toml")]
    hashes = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in sources}
    (output / "prestart-source-sha256.json").write_text(json.dumps(hashes, indent=2) + "\n")
    with tarfile.open(output / "source-snapshot.tar.gz", "w:gz") as archive:
        for name in sources:
            archive.add(ROOT / name, arcname=name)
    return hashes


def unchanged(hashes):
    return all(hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == value for name, value in hashes.items())


def suite_checks(lookup_fixes=False):
    def test(*targets, selector=None):
        args = ["cargo", "test", "--locked"]
        for target in targets:
            args += ["--test", target]
        if selector:
            args.append(selector)
        return args + ["--", "--test-threads=1", "--nocapture"]
    full = [
        ("resources", test("resources")),
        ("native-and-gate", test("native_network", "admission_gate")),
        ("direct-and-signaling-carriers", test("native_probe", "mqtt_probe")),
        ("observation-contract", test("udp_observation")),
        ("nat-resource-budget", test("legacy_nat_runtime", selector="formal_entry_permission_and_complete_budget_stop_before_execution")),
        ("nat-udp-disable", test("legacy_nat_runtime", selector="formal_disabled_udp_blocks_business_before_stream_creation")),
        ("browser-wasm", ["cargo", "check", "--locked", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
        ("workspace-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests"]),
        ("fault-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests", "--features", "replica-test-hooks"]),
        ("format", ["cargo", "fmt", "--check"]),
        ("iroh-format", ["rustfmt", "--check", "--edition", "2024", "--config", "skip_children=true", *["vendor/iroh/src/" + name for name in ["endpoint.rs", "socket.rs", "address_lookup.rs", "address_lookup/pkarr.rs", "address_lookup/dns.rs", "socket/transports.rs", "socket/transports/custom.rs", "socket/transports/relay.rs", "socket/transports/relay/actor.rs"]]]),
    ]
    if lookup_fixes:
        return [full[0],
            ("native-signaling-admission", test("manual_admission", selector="another_invitation_holder_observes_ciphertext_but_cannot_take_an_approved_result")),
            ("memory-lookup-regression", test("native_probe", selector="p2panda_endpoint_reaches_iroh_lookup_without_an_address_book_entry")),
            ("gated-lookup-regression", test("admission_gate", selector="p2panda_endpoint_builder_hook_gates_the_actual_protocol_actor")),
            *full[6:]]
    return full


def run_suite(output, lookup_fixes=False):
    output.mkdir(parents=True, exist_ok=False)
    hashes = snapshot(output)
    receipt = {"started_utc": datetime.now(timezone.utc).isoformat(), "source_sha256": hashes, "checks": [], "success": False}
    container = None
    volumes = []
    def save():
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    save()
    try:
        container = subprocess.check_output(["rtk", "proxy", "docker", "run", "--detach", "--publish", "127.0.0.1::1883", "--mount", f"type=bind,source={ROOT / 'scripts/mosquitto-test.conf'},target=/mosquitto/config/mosquitto.conf,readonly", "eclipse-mosquitto:2"], cwd=ROOT, text=True).strip()
        receipt["owned_container"] = container
        save()
        info = json.loads(subprocess.check_output(["rtk", "proxy", "docker", "inspect", container], cwd=ROOT, text=True))[0]
        volumes = [entry["Name"] for entry in info["Mounts"] if entry["Type"] == "volume"]
        receipt["owned_volumes"] = volumes
        address = subprocess.check_output(["rtk", "proxy", "docker", "port", container, "1883/tcp"], cwd=ROOT, text=True).strip()
        host, port = address.rsplit(":", 1)
        deadline = time.monotonic() + 10
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=1):
                    break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        receipt["mqtt_broker"] = address
        save()
        environment = dict(os.environ, MIOPUNCH_MQTT_BROKER=address)
        for name, command in suite_checks(lookup_fixes):
            check = {"name": name, "started_utc": datetime.now(timezone.utc).isoformat(), "argv": command}
            receipt["checks"].append(check)
            save()
            print(f"Starting {name}", flush=True)
            result = subprocess.run(["rtk", "proxy", "python", "-B", str(Path(__file__).resolve()), "--output", str(output / name), "--", *command], cwd=ROOT, env=environment)
            check["actual_wrapper_exit"] = result.returncode
            (output / (name + "-exit.json")).write_text(json.dumps(check, indent=2) + "\n")
            save()
            if result.returncode or not unchanged(hashes):
                return result.returncode or 2
        receipt["success"] = True
    finally:
        if container:
            removed = subprocess.run(["rtk", "proxy", "docker", "rm", "--force", "--volumes", container], cwd=ROOT, capture_output=True, text=True)
            receipt["cleanup_exit"] = removed.returncode
            receipt["cleanup_output"] = removed.stdout + removed.stderr
            receipt["owned_resource_absence"] = []
            for kind, names in [("container", [container]), ("volume", volumes)]:
                for name in names:
                    result = subprocess.run(["rtk", "proxy", "docker", kind, "inspect", name], cwd=ROOT, capture_output=True, text=True)
                    raw = result.stdout + result.stderr
                    receipt["owned_resource_absence"].append({"kind": kind, "name": name, "actual_exit": result.returncode, "output": raw, "absent": result.returncode != 0 and f"no such {kind}" in raw.lower() and name.lower() in raw.lower()})
            receipt["success"] &= removed.returncode == 0 and all(entry["absent"] for entry in receipt["owned_resource_absence"])
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        receipt["source_unchanged"] = unchanged(hashes)
        receipt["success"] &= receipt["source_unchanged"]
        save()
    return 0 if receipt["success"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--suite", action="store_true", help="Run the resource and affected transport/admission checks with an owned real MQTT broker")
    parser.add_argument("--lookup-fixes", action="store_true", help="Run the current lookup/resource regressions and reuse unchanged prior transport/NAT evidence")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    output = args.output.resolve()
    if args.suite or args.lookup_fixes:
        return run_suite(output, args.lookup_fixes)
    output.mkdir(parents=True, exist_ok=False)
    hashes = snapshot(output)
    command = args.command or ["cargo", "test", "--locked", "--test", "resources", "--", "--test-threads=1", "--nocapture"]
    if command[0] == "--":
        command = command[1:]
    receipt = {"started_utc": datetime.now(timezone.utc).isoformat(), "argv": ["rtk", "proxy", *command], "source_sha256": hashes}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    with (output / "command.log").open("w") as log:
        result = subprocess.run(receipt["argv"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    receipt.update(exit_code=result.returncode, finished_utc=datetime.now(timezone.utc).isoformat(), source_unchanged=unchanged(hashes))
    # Persist the real boundary before executable freezing or log parsing.
    (output / "command-exit.json").write_text(json.dumps(receipt, indent=2) + "\n")
    # Freeze the executable from the actual Cargo run before any later compile.
    log_text = (output / "command.log").read_text()
    frozen = Path("/tmp/mio38-frozen-executables")
    frozen.mkdir(exist_ok=True)
    receipt["executables"] = []
    for name in re.findall(r"Running [^\n]+\(([^\n)]+)\)", log_text):
        executable = ROOT / name
        digest = hashlib.sha256(executable.read_bytes()).hexdigest()
        saved = frozen / (executable.name + "-" + digest)
        if not saved.exists():
            shutil.copy2(executable, saved)
        receipt["executables"].append({"actual_path": name, "frozen_path": str(saved), "sha256": digest, "frozen_after_actual_run": True})
    totals = [tuple(map(int, row)) for row in re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored", log_text)]
    receipt["test_totals"] = {label: sum(row[i] for row in totals) for i, label in enumerate(["passed", "failed", "ignored"])}
    (output / "command-exit.json").write_text(json.dumps(receipt, indent=2) + "\n")
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt[key] for key in ["exit_code", "source_unchanged"]}), flush=True)
    if result.returncode:
        return result.returncode
    if not receipt["source_unchanged"] or (command[:2] == ["cargo", "test"] and not sum(row[0] + row[1] for row in totals)):
        return 2
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
