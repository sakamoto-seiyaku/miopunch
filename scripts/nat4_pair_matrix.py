#!/usr/bin/env python3
"""Interleaved real-kernel bilateral regular NAT4 trials (MIOPUNCH-58)."""

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
    if returncode == 0 and not (success or no_path):
        outcome = "harness_error"
    if returncode != 0:
        outcome = "harness_error"
    return {
        "topology": topology, "dialer": dialer, "trial": trial,
        "strategy": strategy, "fixture_seed": seed, "outcome": outcome,
        "runner_rc": returncode, "wall_ms": wall_ms, "fixture_cpu_ms": cpu_ms,
        "client_path_ms": metric(client, r"DATA_OK[^\n]*path_ms=(\d+)"),
        "server_path_ms": metric(server, r"DATA_OK[^\n]*path_ms=(\d+)"),
        "client_probes": metric(client, r"(?:AUTO_)?PROBES_SENT count=(\d+)"),
        "server_probes": metric(server, r"(?:AUTO_)?PROBES_SENT count=(\d+)"),
        "client_control_sent": len(re.findall(r"^CONTROL_FRAME_SENT ", client, flags=re.M)),
        "server_control_sent": len(re.findall(r"^CONTROL_FRAME_SENT ", server, flags=re.M)),
        "client_control_received": len(re.findall(r"^CONTROL_FRAME_RECEIVED ", client, flags=re.M)),
        "server_control_received": len(re.findall(r"^CONTROL_FRAME_RECEIVED ", server, flags=re.M)),
        "client_mqtt_packets": metric(client, r"(?:mqtt_after_commit=|MQTT_TRAFFIC sent_packets=)(\d+)"),
        "server_mqtt_packets": metric(server, r"(?:mqtt_after_commit=|MQTT_TRAFFIC sent_packets=)(\d+)"),
        "client_udp_sockets": len(re.findall(r"(?:0\.0\.0\.0|\[::\]):\d+", client.split("\n")[0])),
        "server_udp_sockets": len(re.findall(r"(?:0\.0\.0\.0|\[::\]):\d+", server.split("\n")[0])),
        "evidence": directory.name,
    }


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_matrix.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    repeat = int(os.environ.get("MIOPUNCH_NAT_REPEAT", "2"))
    base_seed = os.environ.get("MIOPUNCH_NAT_TEST_SEED", "mio58-20260930")
    if not 2 <= repeat <= 20:
        raise SystemExit("MIOPUNCH_NAT_REPEAT must be 2..20")
    root.mkdir(parents=True, exist_ok=True)
    runner = Path(__file__).with_name("nat4_pair_lab.sh")
    rows = []
    steps = ("up1", "down1", "up2", "down2")
    supported = tuple(f"{a}-{b}" for a in steps for b in steps)
    requested = os.environ.get("MIOPUNCH_NAT_MATRIX_TOPOLOGIES")
    topologies = tuple(requested.split(",")) if requested else supported
    if not topologies or len(set(topologies)) != len(topologies) or any(
        topology not in supported for topology in topologies
    ):
        raise SystemExit("invalid matrix topology subset")
    for topology in topologies:
        for dialer in ("a", "b"):
            for trial in range(1, repeat + 1):
                # AB/BA order alternates within each fixed topology and role.
                order = ("baseline", "auto") if trial % 2 else ("auto", "baseline")
                seed = f"{base_seed}:{topology}:{dialer}:{trial}"
                with tempfile.TemporaryDirectory(prefix="mio58-matrix-keys-") as key_dir:
                    for name in ("client.key", "server.key", "secret.bin"):
                        # Lab-only deterministic identities/secret keep a pair
                        # comparable; key material never enters evidence.
                        Path(key_dir, name).write_bytes(hashlib.sha256(
                            f"{seed}:{name}".encode()).digest())
                    for strategy in order:
                        directory = root / f"{topology}-{dialer}-{trial:02d}-{strategy}"
                        directory.mkdir(exist_ok=True)
                        env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY=topology,
                                   MIOPUNCH_NAT_DIALER=dialer, MIOPUNCH_NAT_KEY_DIR=key_dir)
                        if strategy == "auto":
                            env.pop("MIOPUNCH_NAT_STRATEGY", None)
                        else:
                            env["MIOPUNCH_NAT_STRATEGY"] = "baseline"
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
                        expected = "success" if strategy == "auto" else "no_path"
                        print(f"{len(rows):02d}/{len(topologies)*repeat*4} {topology} {dialer} {trial} "
                              f"{strategy}: {row['outcome']} ({wall_ms} ms)", flush=True)
                        (root / "trials.json").write_text(json.dumps(rows, indent=2) + "\n")
                        with (root / "trials.csv").open("w", newline="") as out:
                            writer = csv.DictWriter(out, fieldnames=rows[0].keys(),
                                                    lineterminator="\n")
                            writer.writeheader()
                            writer.writerows(rows)
                        if row["outcome"] != expected:
                            raise SystemExit(f"unexpected outcome; inspect {directory}")
                        if strategy == "auto" and (
                            "AUTO_SELECTED role=DualFirst" not in (directory / "server.log").read_text()
                            or "AUTO_SELECTED role=DualSecond" not in (directory / "client.log").read_text()
                        ):
                            raise SystemExit(f"wrong automatic branch; inspect {directory}")
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
