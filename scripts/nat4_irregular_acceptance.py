#!/usr/bin/env python3
"""Frozen current-version acceptance supplement; never adds statistical trials."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import resource
import subprocess
import time


SAFETY_CASES = (
    "nonmember", "revoked", "no_secret", "wrong_secret", "wrong_identity",
    "stale", "expire_mapping", "late_control", "cancel", "old_round",
    "old_receipt", "network_after_plan", "revoke_after_plan", "unknown_mapping",
    "wide_claim", "budget_exhausted", "blocked", "wrong", "out_of_window",
)
COMPAT_TOPOLOGIES = ("random-eim", "random-up1", "wide-eim", "random-random")


def cells(kind):
    if kind == "safety":
        return [dict(case=case, topology="random-up1", dialer="a", strategy="auto",
                     seed=f"mio59-safety-20260930:{case}") for case in SAFETY_CASES]
    result = [dict(case="positive", topology=topology, dialer=dialer, strategy=strategy,
                   seed=f"mio59-20260930:{topology}:{dialer}:1")
              for topology in COMPAT_TOPOLOGIES for dialer in ("a", "b")
              for strategy in ("baseline", "auto")]
    # A deterministic original-Connection positive supplements the random
    # misses without selecting a random draw or changing a measured denominator.
    result.append(dict(case="positive", topology="eim-up1", dialer="a", strategy="auto",
                       seed="mio59-compat-deterministic-20260930"))
    return result


def cell_name(cell):
    if cell["case"] != "positive":
        return cell["case"]
    return f"{cell['topology']}-{cell['dialer']}-{cell['strategy']}"


def run(job):
    probe, qad, root, kind, cell = job
    directory = Path(root) / cell_name(cell)
    directory.mkdir()
    # Do not inherit lab tunables from a developer's shell.
    env = {key: value for key, value in os.environ.items()
           if not key.startswith("MIOPUNCH_NAT_")}
    env.update(MIOPUNCH_NAT_TOPOLOGY=cell["topology"], MIOPUNCH_NAT_DIALER=cell["dialer"],
               MIOPUNCH_NAT_STRATEGY=cell["strategy"], MIOPUNCH_NAT_TEST_SEED=cell["seed"],
               MIOPUNCH_NAT_LAB_PATH_WAIT="3",
               MIOPUNCH_NAT_LAB_FAILURE_LIVENESS="1" if kind == "safety" else "0")
    started = time.monotonic()
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    with (directory / "harness.log").open("w") as output:
        outcome = subprocess.run(
            ["bash", str(Path(__file__).with_name("nat4_irregular_lab.sh")), cell["case"],
             probe, qad, str(directory)], env=env, stdout=output, stderr=subprocess.STDOUT,
            timeout=90, check=False,
        )
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    row = dict(cell, runner_rc=outcome.returncode,
               wall_ms=round(1000 * (time.monotonic() - started)),
               fixture_cpu_ms=round(1000 * (after.ru_utime + after.ru_stime
                                          - before.ru_utime - before.ru_stime)))
    (directory / "driver.json").write_text(json.dumps(row, indent=2) + "\n")
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=("safety", "compatibility"))
    parser.add_argument("probe")
    parser.add_argument("qad")
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("run in the dedicated root NAT lab container")
    args.evidence.mkdir(parents=True)
    planned = cells(args.kind)
    (args.evidence / "plan.json").write_text(json.dumps(planned, indent=2) + "\n")
    jobs = [(args.probe, args.qad, str(args.evidence), args.kind, cell) for cell in planned]
    rows = []
    with concurrent.futures.ProcessPoolExecutor(max_workers=4) as pool:
        for row in pool.map(run, jobs):
            rows.append(row)
            print(cell_name(row), row["runner_rc"], flush=True)
            (args.evidence / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
    if any(row["runner_rc"] for row in rows):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
