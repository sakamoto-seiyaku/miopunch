#!/usr/bin/env python3
"""Bilateral NAT4 prediction failures on independent real kernel gateways."""

import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from nat4_pair_pcap import PUBLIC, primes


CASES = (
    "wrong", "stale", "expire_mapping", "late_control", "cancel", "old_round",
    "unknown_mapping", "jump_b", "reversal", "out_of_window", "wide_claim", "blocked",
    "nonmember", "revoked", "no_secret", "wrong_secret",
)
ZERO_PROBES = {
    "stale", "expire_mapping", "late_control", "cancel", "old_round", "unknown_mapping", "wide_claim",
    "nonmember", "revoked", "no_secret", "wrong_secret",
}
PATH_MISSES = {"wrong", "jump_b", "reversal", "out_of_window", "blocked"}


def run_case(runner, probe, qad, root, name):
    directory = root / name
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mio58-failure-keys-") as keys:
        env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY="up1-up1", MIOPUNCH_NAT_DIALER="a",
                   MIOPUNCH_NAT_KEY_DIR=keys)
        env.pop("MIOPUNCH_NAT_STRATEGY", None)
        started = time.monotonic()
        with (directory / "harness.log").open("w") as out:
            completed = subprocess.run(["bash", str(runner), name, probe, qad, str(directory)],
                                       env=env, stdout=out, stderr=subprocess.STDOUT,
                                       timeout=90, check=False)
        wall_ms = round((time.monotonic() - started) * 1000)
    client = (directory / "client.log").read_text() if (directory / "client.log").exists() else ""
    server = (directory / "server.log").read_text() if (directory / "server.log").exists() else ""
    if completed.returncode or wall_ms >= 45000:
        raise RuntimeError(f"{name} runner failed or exceeded bound: {directory}")
    if "DATA_OK" in client or "DATA_OK" in server or "NAT_DATA" in (client + server):
        raise RuntimeError(f"{name} released business: {directory}")
    if "CONTROL_LIVENESS_FAILED" in client or "CONTROL_LIVENESS_FAILED" in server:
        raise RuntimeError(f"{name} lost a failed-path control acknowledgment: {directory}")
    packets = primes(directory / "wan.pcap")
    probes = [sum(event["src"] == public for event in packets) for public in PUBLIC]
    for log, count in zip((client, server), probes):
        summary = re.search(r"(?:AUTO_)?PROBES_SENT count=(\d+)", log)
        if summary and int(summary.group(1)) != count:
            raise RuntimeError(f"{name} endpoint/pcap probe count mismatch: {directory}")
    if name in ZERO_PROBES and any(probes):
        raise RuntimeError(f"{name} sent a probe before valid agreement: {directory}")
    if any(count > 1 for count in probes):
        raise RuntimeError(f"{name} exceeded the one-datagram probe budget: {directory}")
    for event in packets:
        if event["dport"] not in {9, 42035, 43035}:
            raise RuntimeError(f"{name} broadened its probe targets: {directory}")
    if name in PATH_MISSES and (
        "CONTROL_ALIVE_AFTER_UDP_FAILURE" not in client
        or "CONTROL_ALIVE_AFTER_UDP_FAILURE" not in server
    ):
        raise RuntimeError(f"{name} lost its authenticated control connection: {directory}")
    return {
        "case": name, "wall_ms": wall_ms, "client_probes": probes[0],
        "server_probes": probes[1],
        "client_control_sent": len(re.findall(r"^CONTROL_FRAME_SENT ", client, flags=re.M)),
        "server_control_sent": len(re.findall(r"^CONTROL_FRAME_SENT ", server, flags=re.M)),
        "client_outcome": next((line.split()[0] for line in client.splitlines()
                                if line.startswith(("PATH_UNAVAILABLE", "COORD_UNAVAILABLE",
                                                    "STALE_CANDIDATE", "ADMISSION_REJECTED", "Error:"))), ""),
        "server_outcome": next((line.split()[0] for line in server.splitlines()
                                if line.startswith(("PATH_UNAVAILABLE", "COORD_UNAVAILABLE",
                                                    "ADMISSION_REJECTED", "Error:"))), ""),
        "evidence": directory.name,
    }


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_failures.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    runner = Path(__file__).with_name("nat4_pair_lab.sh")
    root.mkdir(parents=True, exist_ok=True)
    rows = []
    for name in CASES:
        row = run_case(runner, probe, qad, root, name)
        rows.append(row)
        print(f"{name}: {row['client_outcome']} / {row['server_outcome']}", flush=True)
        (root / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
