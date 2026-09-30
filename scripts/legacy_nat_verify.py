#!/usr/bin/env python3
"""Verify retained 62 traces against literal legacy expectations and WAN tuples."""
import argparse
import json
import pathlib
import re
import struct

from nat4_pcap_timing import packets


SAMPLE = re.compile(r"LEGACY_SAMPLE order=(\d+) destination=([^ ]+) mapped=([^\n]+)")
SCORE = re.compile(r"BehaviorId \{ mode: (\d+), index: (\d+) \}, score: (-?\d+)")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_capture(directory):
    log = (directory / "tcpdump.log").read_text()
    fixture = (directory / "fixture.sh").read_text()
    capture_filter = re.search(r"\btcpdump\b[^\n]*?'(udp[^'\n]+)'", fixture)
    require(capture_filter and capture_filter.group(1) in (
        "udp and (host 100.64.27.2 or host 100.64.27.6)",
        "udp and (host 100.64.27.2 or host 100.64.27.6 or host 100.64.27.10)"),
        "capture filter does not cover both peer directions")
    counts = [re.search(rf"^(\d+) packets {suffix}$", log, re.M)
              for suffix in ("captured", "received by filter", "dropped by kernel")]
    require(all(counts), "missing capture shutdown counters")
    captured, received, dropped = [int(match.group(1)) for match in counts]
    require(captured > 0 and captured == received and dropped == 0,
            "incomplete/dropped capture cannot prove zero peer traffic")
    with (directory / "wan.pcap").open("rb") as capture:
        header = capture.read(24)
        require(len(header) == 24 and header[:4] == bytes.fromhex("d4c3b2a1"), "unsupported or empty capture")
        frames = 0
        while packet := capture.read(16):
            require(len(packet) == 16, "truncated capture record")
            length = struct.unpack("<IIII", packet)[2]
            require(len(capture.read(length)) == length, "truncated captured packet")
            frames += 1
    require(frames == captured, "capture file does not match tcpdump shutdown count")
    return captured, capture_filter.group(1)


def verify(directory):
    logs = {side: (directory / f"{side}.log").read_text() for side in ("client", "server")}
    run = dict(field.split("=", 1) for field in (directory / "run.txt").read_text().split())
    require(run["strategy"] == "legacy", "wrong product strategy")
    expected_rc = "1" if run["case"] in ("revoked", "no_secret", "wrong_secret") else "0"
    require(run["client_rc"] == expected_rc and run["server_rc"] == expected_rc,
            "unexpected runner exit is not a traversal/admission negative")
    captured, capture_filter = verify_capture(directory)
    events = list(packets(directory / "wan.pcap"))
    primes = [event for event in events if event[1] in ("100.64.27.2", "100.64.27.6", "100.64.27.10")
              and event[2] in ("100.64.27.2", "100.64.27.6", "100.64.27.10") and event[-1] == 11]
    negative_admission = run["case"] in ("nonmember", "revoked", "no_secret", "wrong_secret")
    row = {"case": run["case"], "topology": run["topology"], "dialer": run["dialer"],
           "captured_packets": captured, "peer_prime_capture_occurrences": len(primes), "samples": {}}
    for side, log in logs.items():
        if negative_admission:
            require("LEGACY_SAMPLE" not in log and "LEGACY_SELECTED" not in log and "PROBES_SENT" not in log,
                    f"{side}: enhanced work before admission")
            if run["case"] == "nonmember":
                require(re.search(r"^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=\d+ probes=0$", log, re.M), f"{side}: missing explicit member rejection")
            else:
                expected_error = "Error: Connection was rejected locally" if side == "client" else "Error: Custom { kind: Other, error: Connection was rejected locally }"
                require(expected_error in log.splitlines(), f"{side}: unexpected error is not an admission rejection")
            require("DATA_OK" not in log and "learned=true" not in log, f"{side}: rejected session gained business or learning")
            continue
        samples = SAMPLE.findall(log)
        require([int(sample[0]) for sample in samples] == [0, 1, 2, 3], f"{side}: missing ordered S samples")
        row["samples"][side] = [sample[2] for sample in samples]
        for _, destination, mapped in samples:
            dst, dport = destination.rsplit(":", 1)
            src, sport = mapped.rsplit(":", 1)
            require(f"host {src}" in capture_filter,
                    f"{side}: capture filter omits observed source IP")
            require(any(event[1:5] == (src, dst, int(sport), int(dport)) for event in events),
                    f"{side}: S sample absent from actual outbound WAN tuple {mapped}->{destination}")
            require(any(event[1:5] == (dst, src, int(dport), int(sport)) for event in events),
                    f"{side}: missing observer response at {mapped}")
        selected = next((line for line in log.splitlines() if line.startswith("LEGACY_SELECTED")), None)
        require(selected is not None, f"{side}: missing protected plan completion")
        ids = set(re.findall(r"stable_id=(\d+)", log))
        require(len(ids) == 1, f"{side}: original connection identity changed")
        row[side + "_stable_id"] = next(iter(ids))
        row[side + "_round"] = re.search(r" round=([0-9a-f]{32}) ", selected).group(1)
        counts = {"eim": [(0, 12)], "nonmonotone": [(1, 6), (2, 3), (0, 12)],
                  "ipchange": [(2, 3), (1, 6), (0, 12)]}[run["topology"]]
        expected_scores = [(str(mode), str(index), "0") for mode, count in counts for index in range(count)]
        require(SCORE.findall(selected) == expected_scores, f"{side}: scene insertion/scores differ from legacy")
        require(f"mode={counts[0][0]} index=0 " in selected, f"{side}: first maximum was not selected")
        require("fallback=false" in selected, f"{side}: real features confused with missing evidence")
        if run["topology"] == "nonmonotone" and side == ("client" if run["dialer"] == "a" else "server"):
            require(row["samples"][side] == [f"100.64.27.2:{p}" for p in (42031, 42033, 42032, 42031)], "nonmonotone fixture drift")
            require("behavior: PortChanged, ports_difference: 2, regular_ports_change: true" in log, "small spread misclassified")
        if run["topology"] == "ipchange" and side == ("client" if run["dialer"] == "a" else "server"):
            require(row["samples"][side] == [f"{ip}:42031" for ip in ("100.64.27.2", "100.64.27.10", "100.64.27.2", "100.64.27.10")], "IP-change fixture drift")
            require("behavior: IpChanged, ports_difference: 0, regular_ports_change: false" in log, "IP change misclassified")
        if run["topology"] == "ipchange":
            require("ExecutionUnavailable" in log and "PROBES_SENT" not in log and "DATA_OK" not in log, "unimplemented executor claimed execution")
        elif run["case"] == "positive" and run["topology"] == "eim":
            require("DATA_OK" in log and "partial_execution=1" in log, f"{side}: no actual original-connection business")
            if side == "server":
                require("outcome=confirmed_business learned=true score_delta=2" in log, "business success did not learn")
        else:
            require("PATH_UNAVAILABLE" in log and "DATA_OK" not in log and "learned=true" not in log, "bounded miss counted as success")
            require("CONTROL_ALIVE_AFTER_UDP_FAILURE" in log, "bounded miss lost its protected control exchange")
    if negative_admission or run["topology"] == "ipchange":
        require(not primes, "disabled/unavailable attempt spent peer datagrams")
    else:
        require(primes, "partial execution had no actual WAN probe")
    if not negative_admission:
        require(row["client_round"] == row["server_round"], "protected round mismatch")
    row["result"] = "PASS"
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directories", nargs="+", type=pathlib.Path)
    args = parser.parse_args()
    rows = []
    for directory in args.directories:
        try:
            rows.append({"evidence": str(directory), **verify(directory)})
        except (ValueError, KeyError, StopIteration, AttributeError) as error:
            raise SystemExit(f"FAIL {directory}: {error}") from error
    print(json.dumps({"verified_cases": len(rows), "rows": rows}, indent=2))


if __name__ == "__main__":
    main()
