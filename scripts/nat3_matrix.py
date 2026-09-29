#!/usr/bin/env python3
"""Interleaved real-kernel NAT3 baseline/coordinated trials (MIOPUNCH-56)."""

import csv
import hashlib
import json
import os
import re
import resource
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def metric(log: str, pattern: str):
    values = re.findall(pattern, log)
    return int(values[-1]) if values else None


def parse_run(directory: Path, topology: str, dialer: str, trial: int, strategy: str,
              seed: str, returncode: int, wall_ms: int, cpu_ms: int):
    client = (directory / "client.log").read_text() if (directory / "client.log").exists() else ""
    server = (directory / "server.log").read_text() if (directory / "server.log").exists() else ""
    meta = (directory / "run.txt").read_text() if (directory / "run.txt").exists() else ""
    success = "DATA_OK" in client and "DATA_OK" in server
    no_path = "PATH_UNAVAILABLE" in client or "PATH_UNAVAILABLE" in server
    outcome = "success" if success else "no_path" if no_path and meta else "harness_error"
    if returncode == 0 and not success:
        outcome = "harness_error"
    if returncode != 0 and success:
        outcome = "harness_error"
    return {
        "topology": topology, "dialer": dialer, "trial": trial,
        "strategy": strategy, "fixture_seed": seed, "outcome": outcome,
        "runner_rc": returncode, "wall_ms": wall_ms, "fixture_cpu_ms": cpu_ms,
        "client_path_ms": metric(client, r"DATA_OK[^\n]*path_ms=(\d+)"),
        "server_path_ms": metric(server, r"DATA_OK[^\n]*path_ms=(\d+)"),
        "client_probes": metric(client, r"PROBES_SENT count=(\d+)"),
        "server_probes": metric(server, r"PROBES_SENT count=(\d+)"),
        "client_control_sent": metric(client, r"CONTROL_MSGS sent=(\d+)"),
        "server_control_sent": metric(server, r"CONTROL_MSGS sent=(\d+)"),
        "client_mqtt_packets": metric(client, r"(?:mqtt_after_commit=|MQTT_TRAFFIC sent_packets=)(\d+)"),
        "server_mqtt_packets": metric(server, r"(?:mqtt_after_commit=|MQTT_TRAFFIC sent_packets=)(\d+)"),
        "client_udp_sockets": len(re.findall(r"(?:0\.0\.0\.0|\[::\]):\d+", client.split("\n")[0])),
        "server_udp_sockets": len(re.findall(r"(?:0\.0\.0\.0|\[::\]):\d+", server.split("\n")[0])),
        "evidence": directory.name,
    }


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat3_matrix.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    repeat = int(os.environ.get("MIOPUNCH_NAT_REPEAT", "4"))
    base_seed = os.environ.get("MIOPUNCH_NAT_TEST_SEED", "mio56-20260929")
    if not 2 <= repeat <= 20:
        raise SystemExit("MIOPUNCH_NAT_REPEAT must be 2..20")
    root.mkdir(parents=True, exist_ok=True)
    runner = Path(__file__).with_name("nat3_lab.sh")
    rows = []
    for topology in ("nat3_nat3", "nat3_adf"):
        for dialer in ("a", "b"):
            for trial in range(1, repeat + 1):
                # AB/BA order alternates within each fixed topology and role.
                order = ("baseline", "coordinated") if trial % 2 else ("coordinated", "baseline")
                seed = f"{base_seed}:{topology}:{dialer}:{trial}"
                with tempfile.TemporaryDirectory(prefix="mio56-matrix-keys-") as key_dir:
                    for name in ("client.key", "server.key", "secret.bin"):
                        # Lab-only deterministic identities/secret keep a pair
                        # comparable; key material never enters evidence.
                        Path(key_dir, name).write_bytes(hashlib.sha256(
                            f"{seed}:{name}".encode()).digest())
                    for strategy in order:
                        directory = root / f"{topology}-{dialer}-{trial:02d}-{strategy}"
                        directory.mkdir(exist_ok=True)
                        env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY=topology,
                                   MIOPUNCH_NAT_DIALER=dialer, MIOPUNCH_NAT_STRATEGY=strategy,
                                   MIOPUNCH_NAT_KEY_DIR=key_dir)
                        before = resource.getrusage(resource.RUSAGE_CHILDREN)
                        started = time.monotonic()
                        with (directory / "harness.log").open("w") as out:
                            result = subprocess.run(["bash", str(runner), "positive", probe, qad,
                                                     str(directory)], stdout=out, stderr=subprocess.STDOUT,
                                                    env=env, timeout=90, check=False)
                        wall_ms = round((time.monotonic() - started) * 1000)
                        after = resource.getrusage(resource.RUSAGE_CHILDREN)
                        cpu_ms = round(1000 * ((after.ru_utime + after.ru_stime) -
                                               (before.ru_utime + before.ru_stime)))
                        row = parse_run(directory, topology, dialer, trial, strategy, seed,
                                        result.returncode, wall_ms, cpu_ms)
                        rows.append(row)
                        print(f"{len(rows):02d}/{4*repeat*2} {topology} {dialer} {trial} "
                              f"{strategy}: {row['outcome']} ({wall_ms} ms)", flush=True)
                        (root / "trials.json").write_text(json.dumps(rows, indent=2) + "\n")
                        with (root / "trials.csv").open("w", newline="") as out:
                            writer = csv.DictWriter(out, fieldnames=rows[0].keys(),
                                                    lineterminator="\n")
                            writer.writeheader()
                            writer.writerows(rows)
                        if row["outcome"] == "harness_error":
                            raise SystemExit(f"harness error; inspect {directory}")
    grouped = {}
    for row in rows:
        key = (row["topology"], row["dialer"], row["strategy"])
        group = grouped.setdefault(key, {"success": 0, "attempts": 0})
        group["attempts"] += 1
        group["success"] += row["outcome"] == "success"
    (root / "summary.json").write_text(json.dumps([
        {"topology": key[0], "dialer": key[1], "strategy": key[2], **value}
        for key, value in grouped.items()
    ], indent=2) + "\n")


if __name__ == "__main__":
    main()
