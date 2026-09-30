#!/usr/bin/env python3
"""Paired AB/BA irregular NAT trials, with isolated sequential worker costs."""
import concurrent.futures
import csv
import hashlib
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile
import time

from nat4_pair_matrix import parse_run


def pair(job):
    probe, qad, root, topology, dialer, trial, arms, seed = job
    runner = Path(__file__).with_name("nat4_irregular_lab.sh")
    rows = []
    order = arms if trial % 2 else tuple(reversed(arms))
    with tempfile.TemporaryDirectory(prefix="mio59-keys-") as key_dir:
        for name in ("client.key", "server.key", "secret.bin"):
            Path(key_dir, name).write_bytes(hashlib.sha256(f"{seed}:{name}".encode()).digest())
        for arm in order:
            directory = Path(root) / f"{topology}-{dialer}-{trial:03d}-{arm}"
            directory.mkdir()
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith("MIOPUNCH_NAT_")}
            env.update(MIOPUNCH_NAT_TOPOLOGY=topology, MIOPUNCH_NAT_DIALER=dialer,
                       MIOPUNCH_NAT_KEY_DIR=key_dir, MIOPUNCH_NAT_TEST_SEED=seed,
                       MIOPUNCH_NAT_LAB_PATH_WAIT="3")
            if arm == "baseline":
                env["MIOPUNCH_NAT_STRATEGY"] = "baseline"
            else:
                env.pop("MIOPUNCH_NAT_STRATEGY", None)
                if arm.startswith("padding"):
                    env["MIOPUNCH_NAT_LAB_RANDOM_PADDING"] = arm[7:]
            started = time.monotonic()
            before = resource.getrusage(resource.RUSAGE_CHILDREN)
            with (directory / "harness.log").open("w") as out:
                result = subprocess.run(["bash", str(runner), "positive", probe, qad, str(directory)],
                                        env=env, stdout=out, stderr=subprocess.STDOUT,
                                        timeout=90, check=False)
            after = resource.getrusage(resource.RUSAGE_CHILDREN)
            cpu_ms = round(1000 * (after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime))
            row = parse_run(directory, topology, dialer, trial, arm, seed,
                            result.returncode, round(1000 * (time.monotonic() - started)), cpu_ms)
            logs = "\n".join((directory / f"{name}.log").read_text()
                             for name in ("client", "server") if (directory / f"{name}.log").exists())
            if result.returncode == 0 and "COORD_UNAVAILABLE" in logs:
                row["outcome"] = "bounded_fallback"
            row["padding"] = arm[7:] if arm.startswith("padding") else "default"
            rows.append(row)
    return rows


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_irregular_matrix.py PROBE QAD EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    repeat = int(os.environ.get("MIOPUNCH_NAT_REPEAT", "16"))
    workers = int(os.environ.get("MIOPUNCH_NAT_WORKERS", "4"))
    if not 2 <= repeat <= 128 or not 1 <= workers <= 4:
        raise SystemExit("repeat must be 2..128, workers 1..4")
    topologies = os.environ.get("MIOPUNCH_NAT_MATRIX_TOPOLOGIES", "random-eim,random-up1,random-down2,wide-eim,wide-up1,random-random,wide-wide").split(",")
    supported = {"random-eim", "random-up1", "random-down2", "wide-eim", "wide-up1", "random-random", "wide-wide"}
    if not topologies or len(set(topologies)) != len(topologies) or not set(topologies) <= supported:
        raise SystemExit("invalid topologies")
    arms = tuple(os.environ.get("MIOPUNCH_NAT_MATRIX_ARMS", "baseline,auto").split(","))
    if arms not in (("baseline", "auto"), ("padding0", "padding4")):
        raise SystemExit("invalid comparison arms")
    base_seed = os.environ.get("MIOPUNCH_NAT_TEST_SEED", "mio59-20260930")
    root.mkdir(parents=True, exist_ok=True)
    jobs = [(probe, qad, str(root), topology, dialer, trial, arms,
             f"{base_seed}:{topology}:{dialer}:{trial}")
            for topology in topologies for dialer in ("a", "b") for trial in range(1, repeat + 1)]
    rows = []
    with concurrent.futures.ProcessPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(pair, job) for job in jobs]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            rows.extend(result)
            rows.sort(key=lambda row: (row["topology"], row["dialer"], row["trial"], row["strategy"]))
            (root / "trials.json").write_text(json.dumps(rows, indent=2) + "\n")
            with (root / "trials.csv").open("w", newline="") as out:
                writer = csv.DictWriter(out, fieldnames=rows[0], lineterminator="\n")
                writer.writeheader(); writer.writerows(rows)
            for row in result:
                print(f"{len(rows)}/{2 * len(jobs)} {row['evidence']} {row['outcome']}", flush=True)
                if row["outcome"] == "harness_error":
                    for pending in futures:
                        pending.cancel()
                    raise SystemExit(f"harness error: {row['evidence']}")
    grouped = {}
    for row in rows:
        group = grouped.setdefault((row["topology"], row["dialer"], row["strategy"]),
                                   {"success": 0, "attempts": 0, "bounded_fallback": 0})
        group["attempts"] += 1
        group["success"] += row["outcome"] == "success"
        group["bounded_fallback"] += row["outcome"] == "bounded_fallback"
    (root / "summary.json").write_text(json.dumps([
        {"topology": k[0], "dialer": k[1], "strategy": k[2], **v}
        for k, v in grouped.items()], indent=2) + "\n")


if __name__ == "__main__":
    main()
