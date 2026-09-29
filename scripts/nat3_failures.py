#!/usr/bin/env python3
"""Bounded NAT3 failure and admission matrix with a real prior-round replay."""

import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path


CASES = (
    "wrong", "stale", "late_control", "cancel", "old_round", "replay",
    "blocked", "nonmember", "revoked", "no_secret", "wrong_secret",
)


def run_case(runner, probe, qad, root, name, key_dir, prior_round=None):
    directory = root / name
    directory.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, MIOPUNCH_NAT_STRATEGY="coordinated",
               MIOPUNCH_NAT_TOPOLOGY="nat3_nat3", MIOPUNCH_NAT_DIALER="a",
               MIOPUNCH_NAT_KEY_DIR=str(key_dir))
    if prior_round:
        env["MIOPUNCH_NAT_REPLAY_ROUND"] = prior_round
    started = time.monotonic()
    with (directory / "harness.log").open("w") as out:
        completed = subprocess.run(["bash", str(runner), name, probe, qad, str(directory)],
                                   env=env, stdout=out, stderr=subprocess.STDOUT,
                                   timeout=90, check=False)
    wall_ms = round((time.monotonic() - started) * 1000)
    client = (directory / "client.log").read_text() if (directory / "client.log").exists() else ""
    server = (directory / "server.log").read_text() if (directory / "server.log").exists() else ""
    if completed.returncode != 0:
        raise RuntimeError(f"{name} runner failed: {directory}")
    if wall_ms >= 45000:
        raise RuntimeError(f"{name} exceeded the bounded 40-second node attempt: {directory}")
    if "DATA_OK" in client or "DATA_OK" in server or "NAT_DATA" in (client + server):
        raise RuntimeError(f"{name} released business: {directory}")
    if name in {"nonmember", "revoked", "no_secret", "wrong_secret"} and (
        "PROBES_SENT" in client or "PROBES_SENT" in server
    ):
        raise RuntimeError(f"{name} sent an unauthorized probe: {directory}")
    if name == "old_round" and "old or replayed round" not in server:
        raise RuntimeError(f"{name} failed without a round rejection: {directory}")
    if name == "replay" and "REPLAY_SENT" not in client:
        raise RuntimeError(f"{name} did not send the duplicate START: {directory}")
    return {
        "case": name,
        "wall_ms": wall_ms,
        "client_outcome": next((line.split()[0] for line in client.splitlines()
                                if line.startswith(("PATH_UNAVAILABLE", "COORD_UNAVAILABLE",
                                                    "STALE_CANDIDATE", "ADMISSION_REJECTED", "Error:"))), ""),
        "server_outcome": next((line.split()[0] for line in server.splitlines()
                                if line.startswith(("PATH_UNAVAILABLE", "COORD_UNAVAILABLE",
                                                    "CONTROL_UNAVAILABLE", "ADMISSION_REJECTED", "Error:"))), ""),
        "client_probes": int(re.search(r"PROBES_SENT count=(\d+)", client).group(1))
                         if "PROBES_SENT" in client else 0,
        "server_probes": int(re.search(r"PROBES_SENT count=(\d+)", server).group(1))
                         if "PROBES_SENT" in server else 0,
        "evidence": directory.name,
    }


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat3_failures.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    runner = Path(__file__).with_name("nat3_lab.sh")
    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mio56-keys-") as keys:
        seed_dir = root / "prior-round-success"
        env = dict(os.environ, MIOPUNCH_NAT_STRATEGY="coordinated",
                   MIOPUNCH_NAT_TOPOLOGY="nat3_nat3", MIOPUNCH_NAT_DIALER="a",
                   MIOPUNCH_NAT_KEY_DIR=keys)
        seed_dir.mkdir(exist_ok=True)
        with (seed_dir / "harness.log").open("w") as out:
            seed = subprocess.run(["bash", str(runner), "positive", probe, qad, str(seed_dir)],
                                  env=env, stdout=out, stderr=subprocess.STDOUT,
                                  timeout=90, check=False)
        if seed.returncode:
            raise RuntimeError(f"prior authenticated round failed: {seed_dir}")
        client = (seed_dir / "client.log").read_text()
        prior_round = re.search(r"COORD_START[^\n]*round=([0-9a-f]{32})", client)
        if not prior_round:
            raise RuntimeError("prior round challenge was not captured")
        rows = []
        for name in CASES:
            row = run_case(runner, probe, qad, root, name, keys, prior_round.group(1))
            rows.append(row)
            print(f"{name}: {row['client_outcome']} / {row['server_outcome']}", flush=True)
            (root / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
