#!/usr/bin/env python3
"""Verify current-version safety/compatibility evidence independently of outcomes."""
import argparse
import json
from pathlib import Path
import re
from collections import Counter

from nat4_irregular_acceptance import cell_name, cells
from nat4_irregular_verify import (PUBLIC, captured_costs, require, verify_capture,
                                  verify_harness, verify_preflight, verify_trial)
from nat4_pair_matrix import parse_run
from nat4_pair_pcap import packet_records


AUTH = {"nonmember", "revoked", "no_secret", "wrong_secret", "wrong_identity"}
NO_PEER = AUTH | {"stale", "expire_mapping", "late_control", "cancel", "old_round",
                  "network_after_plan", "revoke_after_plan", "unknown_mapping",
                  "wide_claim", "budget_exhausted"}
REASONS = {
    "stale": ("STALE_CANDIDATE rejected_before_exchange", "COORD_UNAVAILABLE reason=deadline has elapsed"),
    "expire_mapping": ("COORD_UNAVAILABLE reason=ordered QAD mapping evidence expired", "COORD_UNAVAILABLE reason=invalid NAT3 control frame"),
    "late_control": ("COORD_UNAVAILABLE reason=deadline has elapsed", "COORD_UNAVAILABLE reason=deadline has elapsed"),
    "cancel": ("COORD_UNAVAILABLE reason=mapping round cancelled", "COORD_UNAVAILABLE reason=mapping round cancelled"),
    "old_round": ("COORD_UNAVAILABLE reason=invalid NAT3 control frame", "COORD_UNAVAILABLE reason=old or replayed mapping round"),
    "old_receipt": ("PATH_UNAVAILABLE reason=invalid NAT3 control frame", "PATH_UNAVAILABLE reason=sender-prime signal missing or wrong round"),
    "network_after_plan": ("PATH_UNAVAILABLE reason=ordered QAD mapping evidence expired", "PATH_UNAVAILABLE reason=deadline has elapsed"),
    "revoke_after_plan": ("PATH_UNAVAILABLE reason=member path upgrade no longer authorized", "PATH_UNAVAILABLE reason=deadline has elapsed"),
    "unknown_mapping": ("COORD_UNAVAILABLE reason=invalid NAT3 control frame", "COORD_UNAVAILABLE reason=ordered QAD mapping evidence unknown or changed"),
    "wide_claim": ("COORD_UNAVAILABLE reason=invalid NAT3 control frame", "COORD_UNAVAILABLE reason=invalid or over-budget mapping claim"),
    "budget_exhausted": ("PATH_UNAVAILABLE reason=node maintenance peer-datagram budget exhausted", "PATH_UNAVAILABLE reason=deadline has elapsed"),
    "blocked": ("PATH_UNAVAILABLE reason=deadline has elapsed", "PATH_UNAVAILABLE reason=deadline has elapsed"),
    "wrong": ("PATH_UNAVAILABLE reason=deadline has elapsed", "PATH_UNAVAILABLE reason=deadline has elapsed"),
    "out_of_window": ("PATH_UNAVAILABLE reason=deadline has elapsed", "PATH_UNAVAILABLE reason=deadline has elapsed"),
}


def verify_lan(directory):
    captures = {"wan": verify_capture(directory)}
    wan = list(packet_records(directory / "wan.pcap"))
    costs, prime_events, _ = captured_costs(directory / "wan.pcap")
    for side in ("a", "b"):
        captures["lan-" + side] = verify_capture(directory, "lan-" + side,
                                                "tcpdump-lan-" + side + ".log")
        lan = list(packet_records(directory / f"lan-{side}.pcap"))
        for suffix in ("WRONG-PORT", "WRONG-IP"):
            marker = f"{side.upper()}-{suffix}".encode()
            require(any(event["payload"] == marker for event in wan),
                    f"missing deliberate {side} {suffix} WAN probe")
            require(not any(event["payload"] == marker for event in lan),
                    f"APDF passed {side} {suffix} to physical endpoint")
        marker = f"{side.upper()}-RIGHT-PORT".encode()
        require(any(event["payload"] == marker and event["packet_type"] == 0 for event in lan),
                f"missing {side} exact reverse tuple at physical endpoint")
        wrong = (directory / f"filter-{side}-wrong-ip.txt").read_text()
        require(re.search(r"policy DROP [1-9]\d* packets", wrong),
                f"missing {side} wrong-IP kernel drop")
        private, source = ("10.27.1.2", 42027) if side == "a" else ("10.27.2.2", 43027)
        snapshot = (directory / f"map-{side}-different-ip.txt").read_text()
        require(any(f"src={private} dst=100.64.27.1 sport={source} dport=46001" in line
                    for line in snapshot.splitlines()), f"missing {side} different-IP mapping tuple")
        marker = f"{side.upper()}-DIFFERENT-IP".encode()
        require(any(event["payload"] == marker and event["sport"] == source
                    and event["src"] == private for event in lan),
                f"different-IP test changed {side} physical source socket")
        require(any(event["payload"] == marker and event["dst"] == "100.64.27.1"
                    for event in wan), f"missing {side} different-IP WAN mapping")
        public = PUBLIC[0 if side == "a" else 1]
        other = PUBLIC[1 if side == "a" else 0]
        outbound = [event for event in lan if event["packet_type"] == 4
                    and event["src"] == private and event["dst"] == other
                    and event["payload"] == b"\0MIOPUNCH27"]
        actual = [event for event in prime_events if event["src"] == public]
        require(Counter((event["dst"], event["dport"]) for event in outbound)
                == Counter((event["dst"], event["dport"]) for event in actual)
                and all(event["sport"] == source for event in outbound),
                f"prime did not use the original {side} physical UDP socket")
        if costs["peer_udp_datagrams"] == 0:
            require(not any(event["src"] == private and event["dst"] == other
                            and event["payload"] == b"\0MIOPUNCH27" for event in lan),
                    f"zero-peer rejection submitted a hidden {side} local prime")
    return captures


def verify_run(directory, cell):
    driver = json.loads((directory / "driver.json").read_text())
    require({key: driver[key] for key in cell} == cell and driver["runner_rc"] == 0,
            f"driver failure/provenance mismatch in {directory}")
    run = dict(field.split("=", 1) for field in (directory / "run.txt").read_text().split())
    expected = dict(case=cell["case"], strategy=cell["strategy"], topology=cell["topology"],
                    dialer=cell["dialer"], seed=cell["seed"], client_rc="0", server_rc="0")
    if cell["case"] in {"revoked", "no_secret", "wrong_secret", "wrong_identity", "revoke_after_plan"}:
        expected.update(client_rc="1", server_rc="1")
    require(run == expected, f"unexpected endpoint exits/provenance in {directory}")
    warnings = verify_harness(directory)
    verify_preflight(directory, cell["topology"])
    captures = verify_lan(directory)
    logs = {side: (directory / f"{side}.log").read_text() for side in ("client", "server")}
    for side in ("a", "b"):
        allocator = (directory / f"allocator-{side}.log").read_text()
        require(not re.search("Traceback|Error:|Exception", allocator), "allocator failure")
    return driver, logs, captures, warnings


def verify_safety(directory, cell):
    driver, logs, captures, warnings = verify_run(directory, cell)
    case = cell["case"]
    costs, prime_events, _ = captured_costs(directory / "wan.pcap")
    require(all("DATA_OK" not in log and "UDP_COMMIT_ACK" not in log for log in logs.values()),
            f"negative released business in {directory}")
    expected_errors = {side: [] for side in logs}
    if case in {"revoked", "no_secret", "wrong_secret"}:
        expected_errors = {"client": ["Error: Connection was rejected locally"],
                           "server": ["Error: Custom { kind: Other, error: Connection was rejected locally }"]}
    elif case == "wrong_identity":
        expected_errors = {"client": ["Error: Elapsed(())"],
                           "server": ['Error: "unexpected authenticated peer"']}
    elif case == "revoke_after_plan":
        expected_errors = {"client": ["Error: ConnectionLost(LocallyClosed)"],
                           "server": ["Error: Elapsed(())"]}
    for side, log in logs.items():
        require([line for line in log.splitlines() if line.startswith("Error:")] == expected_errors[side],
                f"unrelated endpoint error in {directory}/{side}")
        require(not re.search("panicked at|Traceback|CONTROL_LIVENESS_FAILED", log),
                f"unexpected runtime failure in {directory}/{side}")
        require(re.search(r"^CANDIDATE id=[0-9a-f]{64} sockets=\[0\.0\.0\.0:\d+, \[::\]:\d+\]", log, re.M),
                f"wrong endpoint/socket inventory in {directory}/{side}")
        if case in AUTH:
            require("CONTROL_FRAME_SENT" not in log and "AUTO_SELECTED" not in log,
                    f"rejected identity disclosed enhanced candidate in {directory}")
            if case == "nonmember":
                require(re.search(r"^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=\d+ probes=0$", log, re.M),
                        f"missing precise member rejection in {directory}")
        else:
            marker = REASONS[case][0 if side == "client" else 1]
            require(any(line.startswith(marker + " ") for line in log.splitlines()),
                    f"wrong negative stage/reason in {directory}/{side}: expected {marker}")
            if case != "revoke_after_plan":
                alive = re.search(r"^EARLY_FAILURE_CONTROL_ALIVE stable_id=(\d+)$", log, re.M)
                require(alive and set(re.findall(r"stable_id=(\d+)", log)) == {alive.group(1)},
                        f"original protected control liveness missing in {directory}/{side}")
        costs[f"{side}_coord_frames_sent"] = len(re.findall(r"^CONTROL_FRAME_SENT ", log, re.M))
        costs[f"{side}_coord_bytes_sent"] = sum(map(int, re.findall(r"CONTROL_FRAME_SENT kind=\w+ bytes=(\d+)", log)))
        traffic = re.findall(r"MQTT_TRAFFIC sent_packets=(\d+) sent_bytes=(\d+)", log)
        costs[f"{side}_mqtt_packets_at_failure"] = int(traffic[-1][0]) if traffic else None
        costs[f"{side}_mqtt_bytes_at_failure"] = int(traffic[-1][1]) if traffic else None
    if case in NO_PEER:
        require(costs["peer_udp_datagrams"] == 0, f"early rejection spent peer UDP in {directory}")
    else:
        require(len(prime_events) == (1 if case == "old_receipt" else 2),
                f"wrong bounded prime count in {directory}")
    if case == "network_after_plan":
        require("FAULT_INJECTED network_changed_after_plan" in logs["client"], "network mutation absent")
    if case == "revoke_after_plan":
        require("FAULT_INJECTED peer_revoked_after_plan" in logs["client"], "revocation mutation absent")
    if case == "out_of_window":
        require(any(event["src"] == PUBLIC[0] and event["sport"] == 42131 for event in prime_events),
                "configured irregular mapping change did not execute")
    if case == "wrong":
        require("attempted=100.64.27.6:9" in logs["client"]
                and any(event["src"] == PUBLIC[0] and event["dport"] == 9 for event in prime_events),
                "wrong candidate injection did not execute")
    return dict(cell, acceptance="PASS", captures=captures, warnings=warnings,
                original_control_alive=case not in AUTH | {"revoke_after_plan"},
                authorized_termination=case == "revoke_after_plan",
                wall_ms=driver["wall_ms"], fixture_cpu_ms=driver["fixture_cpu_ms"], **costs)


def verify_compatibility(directory, cell):
    driver, logs, captures, warnings = verify_run(directory, cell)
    row = parse_run(directory, cell["topology"], cell["dialer"], 1, cell["strategy"], cell["seed"],
                    driver["runner_rc"], driver["wall_ms"], driver["fixture_cpu_ms"])
    if all("COORD_UNAVAILABLE" in log for log in logs.values()):
        row["outcome"] = "bounded_fallback"
    verified = verify_trial(directory, row)
    if verified["outcome"] == "success":
        endpoint_names = (("a", "client"), ("b", "server")) if cell["dialer"] == "a" else (("a", "server"), ("b", "client"))
        for side, name in endpoint_names:
            private, port = ("10.27.1.2", 42027) if side == "a" else ("10.27.2.2", 43027)
            target = re.search(r"AUTO_SELECTED role=\w+ target=(\S+)", logs[name]).group(1)
            remote, remote_port = target.rsplit(":", 1)
            events = list(packet_records(directory / f"lan-{side}.pcap"))
            require(any(event["src"] == private and event["sport"] == port
                        and event["dst"] == remote and event["dport"] == int(remote_port)
                        and event["length"] > 11 for event in events), "missing original-socket QUIC send")
            require(any(event["src"] == remote and event["sport"] == int(remote_port)
                        and event["dst"] == private and event["dport"] == port
                        and event["length"] > 11 for event in events), "missing original-socket QUIC receive")
    return dict(verified, acceptance="PASS", captures=captures, warnings=warnings)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=("safety", "compatibility"))
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--retry-map", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    planned = cells(args.kind)
    require(json.loads((args.evidence / "plan.json").read_text()) == planned, "plan changed after freeze")
    expected = {cell_name(cell) for cell in planned}
    actual = {path.name for path in args.evidence.iterdir() if path.is_dir()}
    require(actual == expected, "missing/extra acceptance cells")
    results = json.loads((args.evidence / "results.json").read_text())
    require(len(results) == len(planned), "incomplete/duplicate driver results")
    retry_map = json.loads(args.retry_map.read_text()) if args.retry_map else {}
    permitted_retry = "budget_exhausted" if args.kind == "safety" else "eim-up1-a-auto"
    require(set(retry_map) <= {permitted_retry}, "undeclared extra acceptance retries")
    for row, cell in zip(results, planned):
        name = cell_name(cell)
        require({key: row[key] for key in cell} == cell
                and (row["runner_rc"] == 0 or name in retry_map), "driver result mismatch")
        require(json.loads((args.evidence / cell_name(cell) / "driver.json").read_text()) == row, "cell/driver results disagree")
    verify = verify_safety if args.kind == "safety" else verify_compatibility
    rows = []
    for cell in planned:
        name = cell_name(cell)
        directory = args.evidence / name
        if name in retry_map:
            retry = (args.evidence / retry_map[name]).resolve()
            require(retry == (args.evidence.parent / "acceptance-retries" / name).resolve(),
                    "unexpected retry evidence location")
            original = json.loads((directory / "driver.json").read_text())
            require({key: original[key] for key in cell} == cell, "retry changes original fixture")
            if args.kind == "safety":
                require(original["runner_rc"] == 1
                        and "COORD_UNAVAILABLE reason=ordered QAD mapping evidence expired" in (directory / "server.log").read_text(),
                        "budget retry lacks retained freshness rejection")
            else:
                require(original["runner_rc"] == 0
                        and re.search(r"^[1-9]\d* packets dropped by kernel$",
                                      (directory / "tcpdump.log").read_text(), re.M),
                        "positive retry lacks retained capture rejection")
            verified = verify(retry, cell)
            verified["supersedes_invalid_evidence"] = str(directory)
            verified["canonical_evidence"] = str(retry)
        else:
            verified = verify(directory, cell)
            verified["canonical_evidence"] = str(directory)
        rows.append(verified)
    output = dict(verified_cases=len(rows), kind=args.kind, rows=rows)
    (args.output or args.evidence / "verification.json").write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(dict(verified_cases=len(rows), kind=args.kind, acceptance="PASS")))


if __name__ == "__main__":
    main()
