#!/usr/bin/env python3
"""Check the saved MIOPUNCH-58 kernel matrix against endpoint evidence."""

import json
import re
import statistics
import sys
from pathlib import Path


def capture(log, pattern):
    match = re.search(pattern, log)
    if not match:
        raise ValueError(f"missing {pattern}")
    return match.groups()


def side(log):
    public, last, step = capture(
        log, r"MAPPING_CLASS pattern=Sequential \{ last: (\d+\.\d+\.\d+\.\d+):(\d+), step: (-?\d+) \}")
    predicted = f"{public}:{int(last) + int(step)}"
    role, target, round_id, stable = capture(
        log, r"AUTO_SELECTED role=(\w+) target=(\S+) round=([0-9a-f]+) stable_id=(\d+)")
    data_stable, peer, path_ms = capture(
        log, r"DATA_OK stable_id=(\d+) peer=(\S+) path_ms=(\d+)")
    if stable != data_stable or f"PathData(1, ip:{target})" not in log:
        raise ValueError("selected target or stable connection changed before business")
    if f"UDP_COMMIT_ACK round={round_id} stable_id={stable}" not in log:
        raise ValueError("business lacked round-bound path commit")
    if (len(re.findall(r"^CONTROL_FRAME_SENT ", log, flags=re.M)) != 5
            or len(re.findall(r"^CONTROL_FRAME_RECEIVED ", log, flags=re.M)) != 5):
        raise ValueError("unexpected bilateral control cost")
    if len(re.findall(r"^MAPPING_SAMPLE order=", log, flags=re.M)) != 4:
        raise ValueError("mapping sequence did not have four samples")
    return {"predicted": predicted, "role": role, "target": target,
            "round": round_id, "peer": peer, "identity": capture(log, r"CANDIDATE id=(\S+)")[0],
            "path_ms": int(path_ms)}


def main(root):
    rows = json.loads((root / "trials.json").read_text())
    if len(rows) != 128:
        raise SystemExit(f"expected 128 complete paired trials, got {len(rows)}")
    steps = ("up1", "down1", "up2", "down2")
    expected_cells = {(f"{a}-{b}", role, repeat)
                      for a in steps for b in steps for role in ("a", "b")
                      for repeat in (1, 2)}
    seen_cells = {}
    grouped = {"baseline": [], "auto": []}
    for row in rows:
        directory = root / row["evidence"]
        cell = (row["topology"], row["dialer"], row["trial"])
        seen_cells.setdefault(cell, set()).add(row["strategy"])
        client = (directory / "client.log").read_text()
        server = (directory / "server.log").read_text()
        if row["strategy"] == "baseline":
            if "DATA_OK" in client + server or "PATH_UNAVAILABLE" not in client or "PATH_UNAVAILABLE" not in server:
                raise SystemExit(f"baseline outcome mismatch: {directory}")
            if "MAPPING_SAMPLE" in client + server:
                raise SystemExit(f"baseline performed enhancement observations: {directory}")
        else:
            a, b = side(client), side(server)
            if (a["role"], b["role"]) != ("DualSecond", "DualFirst"):
                raise SystemExit(f"wrong bilateral role order: {directory}")
            if (a["target"], b["target"]) != (b["predicted"], a["predicted"]):
                raise SystemExit(f"target did not derive from remote QAD sequence: {directory}")
            if a["round"] != b["round"] or a["peer"] != b["identity"] or b["peer"] != a["identity"]:
                raise SystemExit(f"round or TLS identity mismatch: {directory}")
            if ("AUTO_PROBES_SENT count=1 distinct_targets=1" not in client
                    or "AUTO_PROBES_SENT count=1 distinct_targets=1" not in server):
                raise SystemExit(f"probe budget/target mismatch: {directory}")
            for name in ("map-a-same-ip.txt", "map-b-same-ip.txt", "filter-a-wrong.txt",
                         "filter-a-right.txt", "filter-b-wrong.txt", "filter-b-right.txt",
                         "nat-a-conntrack.txt", "nat-b-conntrack.txt", "wan.pcap"):
                if not (directory / name).is_file():
                    raise SystemExit(f"missing kernel evidence {name}: {directory}")
        grouped[row["strategy"]].append(row)
    for strategy, entries in grouped.items():
        if len(entries) != 64:
            raise SystemExit(f"incomplete {strategy} arm")
        if any(row["outcome"] != ("success" if strategy == "auto" else "no_path") for row in entries):
            raise SystemExit(f"unexpected {strategy} outcome")
    if set(seen_cells) != expected_cells or any(value != {"baseline", "auto"} for value in seen_cells.values()):
        raise SystemExit("missing or duplicate topology/role/repeat pair")
    report = {"verified_pairs": 64, "baseline_success": 0, "auto_success": 64,
              "auto_client_path_median_ms": statistics.median(
                  row["client_path_ms"] for row in grouped["auto"]),
              "auto_client_path_range_ms": [
                  min(row["client_path_ms"] for row in grouped["auto"]),
                  max(row["client_path_ms"] for row in grouped["auto"])],
              "baseline_wall_median_ms": statistics.median(
                  row["wall_ms"] for row in grouped["baseline"]),
              "auto_wall_median_ms": statistics.median(
                  row["wall_ms"] for row in grouped["auto"])}
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
