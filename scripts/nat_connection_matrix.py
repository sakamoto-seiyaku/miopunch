#!/usr/bin/env python3
"""Execute exactly the frozen formal/baseline cells and functional round chains."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import time


def sha(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("plan", type=pathlib.Path)
    parser.add_argument("runtime", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--group", choices=["comparison", "functional"], default="comparison")
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("run inside the dedicated disposable fixture")
    plan = json.loads(args.plan.read_text())
    repo = pathlib.Path(__file__).resolve().parents[1]
    rows_file = args.output / "verification" / (args.group + "-runner.json")
    rows = json.loads(rows_file.read_text()) if rows_file.exists() else []
    cells = plan[args.group]
    if [row["cell"] for row in rows] != [cell["id"] for cell in cells[:len(rows)]] or any(row["exit"] != 0 for row in rows):
        raise RuntimeError("invalid resume prefix; retain failed evidence and diagnose explicitly")

    def unchanged():
        for name, expected in plan["source"].items():
            if sha(repo / name) != expected:
                raise RuntimeError("frozen runtime/fixture source changed: " + name)
        for name, expected in plan["runtime"].items():
            if sha(args.runtime / name) != expected:
                raise RuntimeError("frozen executable changed: " + name)

    for cell in cells[len(rows):]:
        unchanged()
        target = args.output / args.group / cell["id"]
        env = {key: value for key, value in os.environ.items() if not key.startswith("MIOPUNCH_")}
        env["MIOPUNCH_NAT_KEY_DIR"] = "/keys"
        command = ["python3", "-B", str(repo / "scripts/nat_connection_lab.py"), cell["case"],
                   cell["topology"], cell["dialer"], cell["seed"], str(args.runtime), str(target),
                   "--strategy", cell["strategy"], "--rounds", str(cell["rounds"])]
        started = time.monotonic()
        run = subprocess.run(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (target / "harness.log").write_text(run.stdout)
        (target / "cell.json").write_text(json.dumps(cell, indent=2) + "\n")
        rows.append(dict(cell=cell["id"], exit=run.returncode, wall_seconds=time.monotonic() - started))
        rows_file.write_text(json.dumps(rows, indent=2) + "\n")
        print(f"{args.group.upper()} {len(rows)}/{len(cells)} {cell['id']} exit={run.returncode}", flush=True)
        if run.returncode:
            raise RuntimeError("fixture/runtime failure; not a normal NAT miss")
        for side in ["client", "server"]:
            cpu = json.loads((target / (side + "-cpu.json")).read_text())
            if cpu["executable_sha256"] != plan["runtime"]["udp_nat_probe"] or cpu["exit"] != 0 or cpu["timed_out"] or cpu["exec_samples"] < 1 or cpu["sampled_exec_vm_hwm_kib"] < 1:
                raise RuntimeError("actual node accounting missing or wrong")
        unchanged()


if __name__ == "__main__":
    main()
