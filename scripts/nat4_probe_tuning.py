#!/usr/bin/env python3
"""Paired one-vs-two datagram comparison on the real NAT4×NAT3 lab."""

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

from nat4_matrix import parse_run


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_probe_tuning.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    root.mkdir(parents=True, exist_ok=True)
    runner = Path(__file__).with_name("nat4_lab.sh")
    rows = []
    for topology in ("up1", "down1", "up2", "down2"):
        for dialer in ("a", "b"):
            for trial in (1, 2):
                order = (1, 2) if trial == 1 else (2, 1)
                seed = f"mio57-probe-tuning:{topology}:{dialer}:{trial}"
                with tempfile.TemporaryDirectory(prefix="mio57-tuning-keys-") as keys:
                    for name in ("client.key", "server.key", "secret.bin"):
                        Path(keys, name).write_bytes(hashlib.sha256(f"{seed}:{name}".encode()).digest())
                    for count in order:
                        directory = root / f"{topology}-{dialer}-{trial:02d}-{count}packet"
                        directory.mkdir(exist_ok=True)
                        env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY=topology,
                                   MIOPUNCH_NAT_DIALER=dialer, MIOPUNCH_NAT_STRATEGY="auto",
                                   MIOPUNCH_NAT_LAB_PROBES=str(count), MIOPUNCH_NAT_KEY_DIR=keys)
                        before = resource.getrusage(resource.RUSAGE_CHILDREN)
                        started = time.monotonic()
                        with (directory / "harness.log").open("w") as out:
                            result = subprocess.run(["bash", str(runner), "positive", probe, qad,
                                                     str(directory)], env=env, stdout=out,
                                                    stderr=subprocess.STDOUT, timeout=90, check=False)
                        wall_ms = round((time.monotonic() - started) * 1000)
                        after = resource.getrusage(resource.RUSAGE_CHILDREN)
                        cpu_ms = round(1000 * ((after.ru_utime + after.ru_stime) -
                                               (before.ru_utime + before.ru_stime)))
                        row = parse_run(directory, topology, dialer, trial, f"auto-{count}", seed,
                                        result.returncode, wall_ms, cpu_ms)
                        rows.append(row)
                        print(f"{len(rows):02d}/32 {topology} {dialer} {trial} "
                              f"{count}pkt: {row['outcome']} ({wall_ms} ms)", flush=True)
                        (root / "trials.json").write_text(json.dumps(rows, indent=2) + "\n")
                        with (root / "trials.csv").open("w", newline="") as out:
                            writer = csv.DictWriter(out, fieldnames=rows[0].keys(), lineterminator="\n")
                            writer.writeheader()
                            writer.writerows(rows)
                        if row["outcome"] != "success" or row["client_probes"] != count or row["server_probes"] != count:
                            raise SystemExit(f"unexpected tuning result; inspect {directory}")
    summary = []
    for count in (1, 2):
        group = [row for row in rows if row["strategy"] == f"auto-{count}"]
        summary.append({"peer_datagrams_per_node": count, "success": sum(row["outcome"] == "success" for row in group),
                        "attempts": len(group)})
    (root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
