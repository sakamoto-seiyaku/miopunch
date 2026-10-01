#!/usr/bin/env python3
"""Reproduce MIOPUNCH-36 with an owned MQTT broker and immutable source receipts."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
SOURCES = [
    "Cargo.toml", "Cargo.lock", "src/lib.rs", "src/management_auth.rs",
    "src/management_store.rs", "src/native_network.rs", "src/genesis_recovery.rs",
    "src/management_recovery.rs", "tests/management_recovery.rs",
    "src/daily_management.rs", "src/manual_admission.rs", "src/precode.rs",
    "src/admission_gate.rs", "src/offline_replica.rs",
    "tests/revocation.rs", "tests/admission_gate.rs", "tests/precode.rs",
    "tests/manual_admission.rs", "tests/admission_completion.rs",
    "scripts/check_management_recovery.py", "scripts/mosquitto-test.conf",
]
SOURCES = sorted(set(SOURCES + [
    str(path.relative_to(ROOT))
    for directory in ["src", "tests", "browser-probe/src"]
    for path in (ROOT / directory).rglob("*.rs")
] + ["browser-probe/Cargo.toml"]))


def hashes():
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in SOURCES}


def utc():
    return datetime.now(timezone.utc).isoformat()


def owned_resource_absent(kind, name):
    inspected = subprocess.run(["rtk", "proxy", "docker", kind, "inspect", name],
                               cwd=ROOT, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    # Docker uses "No such container" but "no such volume" on this daemon.
    return {"kind": kind, "name": name, "actual_exit": inspected.returncode,
            "output": inspected.stdout,
            "absent": inspected.returncode != 0
            and f"no such {kind}" in inspected.stdout.lower()
            and name.lower() in inspected.stdout.lower()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path,
                        help="New evidence directory; existing results are never overwritten")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"baseline": "f691d2b4c2c0f8188d9e9418ef691c21978a806c",
               "started_utc": utc(), "source_sha256": hashes(), "commands": []}
    container = None
    success = False
    # Persist the manifest before even creating the broker, not only after
    # returning from Cargo. Recovery can inspect it if this runner disappears.
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    (output / "prestart-source-sha256.json").write_text(json.dumps(receipt["source_sha256"], indent=2) + "\n")
    try:
        container = subprocess.check_output([
            "rtk", "proxy", "docker", "run", "--detach",
            "--publish", "127.0.0.1::1883", "--mount",
            f"type=bind,source={ROOT / 'scripts/mosquitto-test.conf'},target=/mosquitto/config/mosquitto.conf,readonly",
            "eclipse-mosquitto:2",
        ], cwd=ROOT, text=True).strip()
        receipt["owned_container_id"] = container
        inspection = json.loads(subprocess.check_output([
            "rtk", "proxy", "docker", "inspect", container
        ], cwd=ROOT, text=True))[0]
        receipt["owned_anonymous_volumes"] = [mount["Name"] for mount in inspection["Mounts"] if mount["Type"] == "volume"]
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        address = subprocess.check_output([
            "rtk", "proxy", "docker", "port", container, "1883/tcp"
        ], cwd=ROOT, text=True).strip()
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
        receipt["broker_tcp_preflight"] = True
        environment = os.environ.copy()
        environment["MIOPUNCH_MQTT_BROKER"] = address
        checks = [
            ("full-real-broker-suite", ["cargo", "test", "--locked", "--workspace", "--features", "replica-test-hooks", "--", "--test-threads=1", "--nocapture"]),
            ("browser-wasm", ["cargo", "check", "--locked", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
            ("workspace-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests"]),
            ("fault-build-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests", "--features", "replica-test-hooks"]),
            ("format", ["cargo", "fmt", "--check"]),
        ]
        for name, command in checks:
            check = {"name": name, "argv": ["rtk", "proxy", *command], "started_utc": utc()}
            receipt["commands"].append(check)
            print(f"Starting {name}; raw log: {output / (name + '.log')}", flush=True)
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            started = time.monotonic()
            with (output / (name + ".log")).open("w") as log:
                result = subprocess.run(check["argv"], cwd=ROOT, env=environment,
                                        stdout=log, stderr=subprocess.STDOUT)
            check.update(exit_code=result.returncode, finished_utc=utc(),
                         wall_seconds=time.monotonic() - started)
            # Save each actual exit boundary before parsing or broader checks.
            (output / (name + "-exit.json")).write_text(json.dumps(check, indent=2) + "\n")
            receipt["source_unchanged_through_checks"] = hashes() == receipt["source_sha256"]
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            print(f"Finished {name}: exit {result.returncode}", flush=True)
            if result.returncode or not receipt["source_unchanged_through_checks"]:
                raise RuntimeError(f"{name} failed or source changed")
        suite = (output / "full-real-broker-suite.log").read_text()
        totals = [tuple(map(int, row)) for row in re.findall(
            r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", suite)]
        receipt["suite_totals"] = dict(zip(["passed", "failed", "ignored"],
                                            [sum(row[i] for row in totals) for i in range(3)]))
        receipt["broker_tests_not_skipped"] = "SKIPPED" not in suite
        capture = re.search(r"MIO30_MQTT_CIPHERTEXT[^\n]*", suite)
        receipt["actual_mqtt_capture"] = capture.group(0) if capture else None
        if not totals or not receipt["broker_tests_not_skipped"] or not capture:
            raise RuntimeError("full suite lacks required real-broker evidence")
        success = True
    finally:
        if container:
            cleanup = subprocess.run(["rtk", "proxy", "docker", "rm", "--force", "--volumes", container],
                                     cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     text=True)
            receipt["owned_container_cleanup_exit"] = cleanup.returncode
            receipt["owned_container_cleanup_output"] = cleanup.stdout
            receipt["owned_container_removed"] = cleanup.returncode == 0
            receipt["owned_resource_absence"] = {}
            for kind, names in [("container", [container]), ("volume", receipt.get("owned_anonymous_volumes", []))]:
                for owned_name in names:
                    receipt["owned_resource_absence"][owned_name] = owned_resource_absent(kind, owned_name)
            receipt["owned_container_removed"] = receipt["owned_container_removed"] and all(item["absent"] for item in receipt["owned_resource_absence"].values())
        receipt["finished_utc"] = utc()
        receipt["success"] = success and receipt.get("owned_container_removed", False)
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt.get(key) for key in ["success", "suite_totals", "actual_mqtt_capture", "owned_container_removed"]}), flush=True)
    return 0 if receipt["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
