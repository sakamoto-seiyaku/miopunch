#!/usr/bin/env python3
"""Reproduce MIOPUNCH-31 with an owned MQTT broker and immutable source receipts."""
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
    "src/management_store.rs", "src/offline_replica.rs", "tests/offline_replica.rs",
]


def hashes():
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in SOURCES}


def utc():
    return datetime.now(timezone.utc).isoformat()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path,
                        help="New evidence directory; existing results are never overwritten")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"baseline": "84f4bf933a6fc39044508e799fc00bab9e0014ca",
               "started_utc": utc(), "source_sha256": hashes(), "commands": []}
    container = None
    success = False
    try:
        container = subprocess.check_output([
            "rtk", "proxy", "docker", "run", "--detach",
            "--publish", "127.0.0.1::1883", "--mount",
            f"type=bind,source={ROOT / 'scripts/mosquitto-test.conf'},target=/mosquitto/config/mosquitto.conf,readonly",
            "eclipse-mosquitto:2",
        ], cwd=ROOT, text=True).strip()
        receipt["owned_container_id"] = container
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
            ("full-real-broker-suite", ["cargo", "test", "--workspace", "--features", "replica-test-hooks", "--", "--test-threads=1", "--nocapture"]),
            ("browser-wasm", ["cargo", "check", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
            ("workspace-typecheck", ["cargo", "check", "--workspace", "--tests"]),
            ("fault-build-typecheck", ["cargo", "check", "--workspace", "--tests", "--features", "replica-test-hooks"]),
            ("format", ["cargo", "fmt", "--check"]),
        ]
        for name, command in checks:
            check = {"name": name, "argv": ["rtk", "proxy", *command], "started_utc": utc()}
            receipt["commands"].append(check)
            print(f"Starting {name}; raw log: {output / (name + '.log')}", flush=True)
            started = time.monotonic()
            with (output / (name + ".log")).open("w") as log:
                result = subprocess.run(check["argv"], cwd=ROOT, env=environment,
                                        stdout=log, stderr=subprocess.STDOUT)
            check.update(exit_code=result.returncode, finished_utc=utc(),
                         wall_seconds=time.monotonic() - started)
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
            cleanup = subprocess.run(["rtk", "proxy", "docker", "rm", "--force", container],
                                     cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     text=True)
            receipt["owned_container_cleanup_exit"] = cleanup.returncode
            receipt["owned_container_removed"] = cleanup.returncode == 0
        receipt["finished_utc"] = utc()
        receipt["success"] = success and receipt.get("owned_container_removed", False)
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt.get(key) for key in ["success", "suite_totals", "actual_mqtt_capture", "owned_container_removed"]}), flush=True)
    return 0 if receipt["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
