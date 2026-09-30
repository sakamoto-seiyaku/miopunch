#!/usr/bin/env python3
"""Interleave one-versus-two first-side datagrams on bilateral regular NAT4."""

import csv
import hashlib
import json
import os
import resource
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from nat4_pair_matrix import parse_run


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_pair_tuning.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    runner = Path(__file__).with_name("nat4_pair_lab.sh")
    root.mkdir(parents=True, exist_ok=True)
    rows = []
    for topology in ("up1-up1", "up1-down1", "up2-down2", "down1-up2"):
        for dialer in ("a", "b"):
            for trial in (1, 2):
                order = (1, 2) if trial == 1 else (2, 1)
                seed = f"mio58-packet-tuning-20260930:{topology}:{dialer}:{trial}"
                with tempfile.TemporaryDirectory(prefix="mio58-tuning-keys-") as key_dir:
                    for name in ("client.key", "server.key", "secret.bin"):
                        Path(key_dir, name).write_bytes(hashlib.sha256(
                            f"{seed}:{name}".encode()).digest())
                    for first_packets in order:
                        directory = root / f"{topology}-{dialer}-{trial:02d}-first{first_packets}"
                        directory.mkdir(exist_ok=True)
                        env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY=topology,
                                   MIOPUNCH_NAT_DIALER=dialer, MIOPUNCH_NAT_KEY_DIR=key_dir,
                                   MIOPUNCH_NAT_LAB_DUAL_FIRST_PACKETS=str(first_packets))
                        env.pop("MIOPUNCH_NAT_STRATEGY", None)
                        before = resource.getrusage(resource.RUSAGE_CHILDREN)
                        started = time.monotonic()
                        with (directory / "harness.log").open("w") as out:
                            result = subprocess.run(["bash", str(runner), "tuning", probe, qad,
                                                     str(directory)], stdout=out,
                                                    stderr=subprocess.STDOUT, env=env,
                                                    timeout=90, check=False)
                        wall_ms = round((time.monotonic() - started) * 1000)
                        after = resource.getrusage(resource.RUSAGE_CHILDREN)
                        cpu_ms = round(1000 * ((after.ru_utime + after.ru_stime) -
                                               (before.ru_utime + before.ru_stime)))
                        row = parse_run(directory, topology, dialer, trial,
                                        f"first{first_packets}", seed, result.returncode,
                                        wall_ms, cpu_ms)
                        client = (directory / "client.log").read_text()
                        server = (directory / "server.log").read_text()
                        if row["outcome"] == "harness_error" or ("DATA_OK" in client) != ("DATA_OK" in server):
                            raise SystemExit(f"invalid tuning trial: {directory}")
                        rows.append(row)
                        print(f"{len(rows):02d}/32 {directory.name}: {row['outcome']}", flush=True)
                        (root / "trials.json").write_text(json.dumps(rows, indent=2) + "\n")
                        with (root / "trials.csv").open("w", newline="") as out:
                            writer = csv.DictWriter(out, fieldnames=rows[0].keys(),
                                                    lineterminator="\n")
                            writer.writeheader()
                            writer.writerows(rows)
    report = {arm: {"attempts": sum(row["strategy"] == arm for row in rows),
                    "success": sum(row["strategy"] == arm and row["outcome"] == "success"
                                   for row in rows)}
              for arm in ("first1", "first2")}
    (root / "summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
