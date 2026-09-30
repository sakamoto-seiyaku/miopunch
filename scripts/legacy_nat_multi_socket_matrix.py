#!/usr/bin/env python3
"""Frozen mode2/4 matrix. Every planned cell remains in the denominator."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import time


def cells():
    result = []
    for topology, mode, first in [("eim-random", 2, 0), ("eim-wide", 2, 0),
                                  ("up1-random", 4, 0), ("up1-wide", 4, 0),
                                  ("up1-up1", 4, 6)]:
        for dialer in "ab":
            for index in range(3):
                result.append(dict(case="positive", topology=topology, dialer=dialer,
                                   mode=mode, index=index, history=first + index,
                                   seed="mio61-frozen-01", variant="normal"))
    for topology, mode, dialer in [("eim-wide", 2, "a"), ("up1-wide", 4, "b")]:
        for case, variant in [("blocked", "normal"), ("positive", "ttl_unavailable")]:
            result.append(dict(case=case, topology=topology, dialer=dialer,
                               mode=mode, index=0, history=0,
                               seed="mio61-frozen-01", variant=variant))
    for case in ["nonmember", "revoked", "no_secret", "wrong_secret"]:
        result.append(dict(case=case, topology="eim-wide", dialer="a",
                           mode=None, index=None, history=0,
                           seed="mio61-frozen-01", variant="normal"))
    return result


def name(cell):
    return f"{cell['case']}-{cell['topology']}-{cell['dialer']}-h{cell['history']}-{cell['variant']}"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("runtime", type=pathlib.Path)
    p.add_argument("evidence", type=pathlib.Path)
    a = p.parse_args()
    if os.geteuid() != 0:
        p.error("run in disposable privileged NAT lab")
    repo = pathlib.Path(__file__).resolve().parents[1]
    verification = a.evidence / "verification"
    verification.mkdir(parents=True, exist_ok=True)
    source = json.loads((verification / "source-sha256.json").read_text())
    binaries = json.loads((a.runtime / "sha256.json").read_text())

    def unchanged():
        for file, digest in source.items():
            if hashlib.sha256((repo / file).read_bytes()).hexdigest() != digest:
                raise RuntimeError("frozen source/fixture changed during matrix: " + file)
        for file, digest in binaries.items():
            if hashlib.sha256((a.runtime / file).read_bytes()).hexdigest() != digest:
                raise RuntimeError("frozen binary/pair changed during matrix: " + file)

    matrix = cells()
    (verification / "cells.json").write_text(json.dumps(matrix, indent=2))
    output = a.evidence / "canonical"
    output.mkdir(exist_ok=True)
    rows = json.loads((verification / "runner.json").read_text()) if (verification / "runner.json").exists() else []
    # A failed infrastructure cell is retained separately before retrying that
    # exact cell. Completed natural trials are never repeated or discarded.
    if rows and rows[-1]["exit"] != 0:
        rows.pop()
    if any(row["exit"] != 0 for row in rows) or [row["cell"] for row in rows] != [name(cell) for cell in matrix[:len(rows)]]:
        raise RuntimeError("invalid resume prefix")
    for i, cell in enumerate(matrix):
        unchanged()
        if i < len(rows):
            continue
        target = output / name(cell)
        target.mkdir(exist_ok=True)
        env = dict(os.environ)
        for key in list(env):
            if key.startswith("MIOPUNCH_"):
                env.pop(key)
        env["MIOPUNCH_NAT_KEY_DIR"] = str(a.runtime / "keys")
        env["MIOPUNCH_LEGACY_TTL_UNAVAILABLE"] = "1" if cell["variant"] == "ttl_unavailable" else "0"
        started = time.monotonic()
        run = subprocess.run(["python3", "-B", str(repo / "scripts/legacy_nat_multi_socket_lab.py"),
                              cell["case"], cell["topology"], cell["dialer"], str(cell["history"]),
                              cell["seed"], str(a.runtime / "udp_nat_probe"),
                              str(a.runtime / "udp_observation_probe"), str(target)],
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (target / "harness.log").write_text(run.stdout)
        (target / "cell.json").write_text(json.dumps(cell, indent=2))
        rows.append(dict(cell=name(cell), exit=run.returncode,
                         elapsed_ms=round((time.monotonic() - started) * 1000)))
        (verification / "runner.json").write_text(json.dumps(rows, indent=2))
        print(f"CELL {i + 1}/{len(matrix)} {name(cell)} exit={run.returncode}", flush=True)
        if run.returncode:
            raise RuntimeError("infrastructure/fixture failure: " + name(cell))
        if cell["mode"] is not None:
            for side in ["client", "server"]:
                log = (target / (side + ".log")).read_text()
                if f"LEGACY_SELECTED mode={cell['mode']} index={cell['index']} " not in log:
                    raise RuntimeError("expected old plan did not execute: " + name(cell))
            if f"LEGACY_TEST_HISTORY previous_selections={cell['history']}" not in (target / "server.log").read_text():
                raise RuntimeError("required history feature missing")
        unchanged()


if __name__ == "__main__":
    main()
