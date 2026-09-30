#!/usr/bin/env python3
"""Real-kernel automatic branch boundaries for MIOPUNCH-58."""

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path


CASES = (
    ("eim-eim", "a", "auto", None, None),
    ("eim-eim", "a", "baseline", None, None),
    ("up1-eim", "a", "auto", "MappingSideSender", "StableSideReceiver"),
    ("up1-eim", "b", "auto", "StableSideReceiver", "MappingSideSender"),
    ("eim-up1", "a", "auto", "StableSideReceiver", "MappingSideSender"),
    ("eim-up1", "b", "auto", "MappingSideSender", "StableSideReceiver"),
)


def main():
    if len(sys.argv) != 4 or os.geteuid() != 0:
        raise SystemExit("usage (root): nat4_pair_boundaries.py PROBE_BIN QAD_BIN EVIDENCE_DIR")
    probe, qad, root = sys.argv[1], sys.argv[2], Path(sys.argv[3]).resolve()
    runner = Path(__file__).with_name("nat4_pair_lab.sh")
    root.mkdir(parents=True, exist_ok=True)
    rows = []
    for topology, dialer, strategy, client_role, server_role in CASES:
        directory = root / f"{topology}-{dialer}-{strategy}"
        directory.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="mio58-boundary-keys-") as keys:
            env = dict(os.environ, MIOPUNCH_NAT_TOPOLOGY=topology,
                       MIOPUNCH_NAT_DIALER=dialer, MIOPUNCH_NAT_KEY_DIR=keys)
            env.pop("MIOPUNCH_NAT_STRATEGY", None)
            if strategy == "baseline":
                env["MIOPUNCH_NAT_STRATEGY"] = "baseline"
            started = time.monotonic()
            with (directory / "harness.log").open("w") as out:
                result = subprocess.run(["bash", str(runner), "positive", probe, qad,
                                         str(directory)], env=env, stdout=out,
                                        stderr=subprocess.STDOUT, timeout=90, check=False)
            wall_ms = round((time.monotonic() - started) * 1000)
        client = (directory / "client.log").read_text()
        server = (directory / "server.log").read_text()
        if result.returncode or wall_ms >= 45000:
            raise RuntimeError(f"boundary fixture failed: {directory}")
        if client_role is None:
            expected = "DATA_OK" if strategy == "baseline" else "COORD_UNAVAILABLE"
            if expected not in client or expected not in server:
                raise RuntimeError(f"wrong stable/stable branch: {directory}")
            if strategy == "auto" and ("AUTO_SELECTED" in client or "AUTO_SELECTED" in server):
                raise RuntimeError(f"stable/stable entered regular branch: {directory}")
        elif (f"AUTO_SELECTED role={client_role}" not in client
              or f"AUTO_SELECTED role={server_role}" not in server
              or "DATA_OK" not in client or "DATA_OK" not in server):
            raise RuntimeError(f"wrong single-regular branch: {directory}")
        rows.append({"topology": topology, "dialer": dialer, "strategy": strategy,
                     "client_role": client_role, "server_role": server_role,
                     "success": "DATA_OK" in client and "DATA_OK" in server,
                     "wall_ms": wall_ms, "evidence": directory.name})
        print(f"{len(rows)}/{len(CASES)} {directory.name}: pass", flush=True)
        (root / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
