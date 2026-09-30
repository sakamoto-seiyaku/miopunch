#!/usr/bin/env python3
"""Independently verify every irregular NAT trial from pcap and runtime records.

Never merge packets by timestamp. SLL2 ingress observes each WAN peer send once;
QAD responses are WAN egress. This checker does not trust a lab outcome label as
evidence of mapping, filtering, a random hit, or original QUIC business success.
"""
import json
from pathlib import Path
import re
import statistics
import struct
import sys

from nat4_pair_pcap import packet_records, peer_ingress, primes, PUBLIC
from nat4_irregular_allocator import allocation


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_capture(directory, stem="wan", log_name="tcpdump.log"):
    """Require a fully drained capture, independently of packet parsing."""
    log = (directory / log_name).read_text()
    counts = [re.search(rf"^(\d+) packets {suffix}$", log, re.M)
              for suffix in ("captured", "received by filter", "dropped by kernel")]
    require(all(counts), f"missing {stem} capture shutdown counters in {directory}")
    captured, received, dropped = [int(match.group(1)) for match in counts]
    require(captured > 0 and captured == received and dropped == 0,
            f"incomplete/dropped {stem} capture in {directory}")
    with (directory / f"{stem}.pcap").open("rb") as capture:
        header = capture.read(24)
        require(len(header) == 24 and header[:4] == bytes.fromhex("d4c3b2a1")
                and struct.unpack("<I", header[20:24])[0] == 276,
                f"invalid {stem} capture header in {directory}")
        frames = 0
        while record := capture.read(16):
            require(len(record) == 16, f"truncated {stem} capture record in {directory}")
            length, original = struct.unpack("<IIII", record)[2:]
            require(length == original and len(capture.read(length)) == length,
                    f"truncated {stem} captured packet in {directory}")
            frames += 1
    require(frames == captured, f"{stem} capture/count mismatch in {directory}")
    return captured


def verify_harness(directory):
    lines = (directory / "harness.log").read_text().splitlines()
    passes = [index for index, line in enumerate(lines) if line.startswith("PASS ")]
    require(len(passes) == 1, f"missing/duplicate fixture completion in {directory}")
    # Five frozen historical runs emitted this iproute2 teardown-only warning.
    # Preserve it and require it to follow the completed measurement. Other
    # endpoint, allocator or runner errors are never traversal misses.
    warnings = []
    for index, line in enumerate(lines):
        if re.search(r"Error:|Traceback|FAILED|Segmentation fault", line):
            require(line == "Error: Peer netns reference is invalid." and index > passes[0],
                    f"fixture infrastructure error in {directory}: {line}")
            warnings.append(line)
    return warnings


def captured_costs(path):
    all_events = list(packet_records(path))
    prime_events = primes(path)
    # Preflight uses these exact plaintext marker payloads and never QUIC.
    preflight = {b"A-MAIN-MAP", b"A-ALT-MAP", b"B-MAIN-MAP", b"B-ALT-MAP",
                 b"B-OPEN-A", b"B-WRONG-PORT", b"B-RIGHT-PORT", b"A-OPEN-B",
                 b"A-WRONG-PORT", b"A-RIGHT-PORT", b"A-DIFFERENT-IP", b"B-DIFFERENT-IP",
                 b"A-WRONG-IP", b"B-WRONG-IP"}
    peer = [event for event in peer_ingress(path) if event["payload"] not in preflight]
    qad = [event for event in all_events
           if (event["packet_type"] == 0 and event["src"] in (*PUBLIC, "100.64.27.10")
               and event["dst"] == "100.64.27.1" and 46761 <= event["dport"] <= 46764)
           or (event["packet_type"] == 4 and event["src"] == "100.64.27.1"
               and event["dst"] in (*PUBLIC, "100.64.27.10") and 46761 <= event["sport"] <= 46764)]
    return {"application_primes": len(prime_events), "peer_udp_datagrams": len(peer),
            "peer_quic_datagrams": sum(event["payload"] != b"\0MIOPUNCH27" for event in peer),
            "peer_udp_payload_bytes": sum(event["length"] for event in peer),
            "qad_udp_datagrams": len(qad), "qad_udp_payload_bytes": sum(event["length"] for event in qad)}, prime_events, all_events


def verify_preflight(directory, topology):
    for side, kind in zip(("a", "b"), topology.split("-")):
        maps = (directory / f"map-{side}-same-ip.txt").read_text()
        source = "10.27.1.2" if side == "a" else "10.27.2.2"
        peer = PUBLIC[1] if side == "a" else PUBLIC[0]
        port = 42027 if side == "a" else 43027
        mapped = []
        targets = []
        for line in maps.splitlines():
            if f"src={source} dst={peer} sport={port}" not in line:
                continue
            ports = re.findall(r"dport=(\d+)", line)
            require(len(ports) == 2, f"bad conntrack tuple in {directory}")
            targets.append(int(ports[0])); mapped.append(int(ports[1]))
        require(len(mapped) == 2 and len(set(targets)) == 2, f"two peer destinations not proven in {directory}/{side}")
        require((mapped[0] == mapped[1]) == (kind == "eim"), f"mapping behavior differs from fixture in {directory}/{side}")
        wrong = (directory / f"filter-{side}-wrong.txt").read_text()
        right = (directory / f"filter-{side}-right.txt").read_text()
        require(re.search(r"policy DROP [1-9]\d* packets", wrong), f"APDF wrong source not dropped in {directory}/{side}")
        require(any(re.match(r"\s*[1-9]\d*\s+\d+\s+ACCEPT.*wan0\s+lan0", line)
                    for line in right.splitlines()), f"APDF exact reverse tuple not accepted in {directory}/{side}")
        require("ALLOCATOR_RESET" in (directory / f"allocator-{side}.log").read_text(), f"preflight state not reset in {directory}/{side}")


def verify_trial(directory, row):
    topology, dialer = row["topology"], row["dialer"]
    logs = {name: (directory / f"{name}.log").read_text() for name in ("client", "server")}
    require(row["runner_rc"] == 0 and row["outcome"] != "harness_error", f"harness failure in final set {directory}")
    run = dict(field.split("=", 1) for field in (directory / "run.txt").read_text().split())
    strategy = "baseline" if row["strategy"] == "baseline" else "auto"
    require(run == dict(case="positive", strategy=strategy, topology=topology, dialer=dialer,
                        client_rc="0", server_rc="0", seed=row["fixture_seed"]),
            f"wrong runtime provenance/exit in {directory}")
    captured = verify_capture(directory)
    warnings = verify_harness(directory)
    verify_preflight(directory, topology)
    costs, prime_events, events = captured_costs(directory / "wan.pcap")
    server_public = PUBLIC[1] if dialer == "a" else PUBLIC[0]
    client_public = PUBLIC[0] if dialer == "a" else PUBLIC[1]
    roles = {}
    selected = {}
    rounds = {}
    endpoint_logs = {}
    mapped_samples = {}
    for name, public in (("server", server_public), ("client", client_public)):
        log = logs[name]
        require(not re.search(r"^Error:|panicked at|^CONTROL_LIVENESS_FAILED", log, re.M),
                f"endpoint error is not a traversal miss in {directory}/{name}")
        require(not re.search(r"CONTROL_LIVENESS_FAILED", log), f"control failure in {directory}")
        require(re.search(r"CANDIDATE .*sockets=\[0\.0\.0\.0:\d+, \[::\]:\d+\]", log), f"endpoint sockets changed in {directory}")
        samples = re.findall(r"MAPPING_SAMPLE order=(\d+) .*destination=([^ ]+) mapped=([^ ]+) t_ms=(\d+) age_ms=(\d+)", log)
        require(len(samples) == (0 if strategy == "baseline" else 4),
                f"missing/extra experimental samples in {directory}/{name}")
        if samples:
            require(len(samples) == 4 and [int(s[0]) for s in samples] == [0, 1, 2, 3], f"wrong sample order in {directory}/{name}")
            require(len({s[1] for s in samples}) == 4, f"not independent destinations in {directory}/{name}")
            ports = [int(s[2].split(":")[-1]) for s in samples]
            deltas = [ports[i + 1] - ports[i] for i in range(3)]
            ips = [s[2].rsplit(":", 1)[0] for s in samples]
            mapped_samples[public] = (ips, ports)
            physical_side = "a" if public == PUBLIC[0] else "b"
            kind = topology.split("-")[0 if physical_side == "a" else 1]
            base = 42031 if physical_side == "a" else 43031
            step = 1 if kind == "up1" else -2 if kind == "down2" else 0
            if kind == "down2": base += 18
            if kind == "eim": base -= 4
            expected_ports = allocation(kind, base, step, row["fixture_seed"] + ":" + physical_side)[:4]
            require(ports == expected_ports and ips == [public] * 4,
                    f"frozen fixture seed/sample mismatch in {directory}/{name}")
            expected = "Unknown" if len(set(ips)) != 1 else "Stable" if all(x == 0 for x in deltas) else "Sequential" if len(set(deltas)) == 1 and 0 < abs(deltas[0]) <= 2 else "Irregular" if len(set(deltas)) > 1 else "Unknown"
            require(f"MAPPING_CLASS pattern={expected}" in log, f"class differs from real samples in {directory}/{name}")
            for _, destination, mapped, _, age in samples:
                ip, port = mapped.rsplit(":", 1)
                observer_ip, observer_port = destination.rsplit(":", 1)
                require(any(e["packet_type"] == 0 and e["src"] == ip and e["sport"] == int(port)
                            and e["dst"] == observer_ip and e["dport"] == int(observer_port)
                            for e in events), f"QAD source mapping missing from pcap in {directory}/{name}")
                require(any(e["packet_type"] == 4 and e["src"] == observer_ip
                            and e["sport"] == int(observer_port) and e["dst"] == ip
                            and e["dport"] == int(port) for e in events),
                        f"QAD response missing from pcap in {directory}/{name}")
                require(int(age) <= 2000, f"sample stale on collection in {directory}/{name}")
        branch = re.search(r"AUTO_SELECTED role=(\w+) target=(\S+) round=([a-f0-9]+) stable_id=(\d+)", log)
        dual_irregular = all(kind in ("random", "wide") for kind in topology.split("-"))
        require(bool(branch) == (strategy == "auto" and not dual_irregular),
                f"strategy did not execute expected protected selection in {directory}/{name}")
        if strategy == "auto" and dual_irregular:
            require("COORD_UNAVAILABLE" in log and row["outcome"] == "bounded_fallback",
                    f"dual irregular historical fallback missing in {directory}/{name}")
        if branch:
            roles[public] = branch.group(1)
            selected[public] = branch.group(2)
            rounds[public] = branch.group(3)
            endpoint_logs[public] = log
            ids = re.findall(r"stable_id=(\d+)", log)
            require(set(ids) == {branch.group(4)}, f"new connection counted as original in {directory}/{name}")
            require(re.search(r"CONTROL_FRAME_(SENT|RECEIVED) kind=SENDER_PRIMED", log), f"prime barrier missing in {directory}/{name}")
            local_primes = [event for event in prime_events if event["src"] == public]
            require(len(local_primes) == 1, f"wrong local packet budget in {directory}/{name}")
            side = "a" if public == PUBLIC[0] else "b"
            private = "10.27.1.2" if side == "a" else "10.27.2.2"
            source_port = 42027 if side == "a" else 43027
            entries = (directory / f"nat-{side}-conntrack.txt").read_text()
            require(any(f"src={private} dst={local_primes[0]['dst']} sport={source_port} dport={local_primes[0]['dport']}" in line
                        and f"dport={local_primes[0]['sport']}" in line.rsplit("src=", 1)[-1]
                        for line in entries.splitlines()), f"prime did not originate on original local QUIC UDP socket {directory}/{name}")
            require(f"{local_primes[0]['dst']}:{local_primes[0]['dport']}" == branch.group(2), f"probe exceeded approved target in {directory}/{name}")
            ages = []
            for observer_port in range(46761, 46765):
                qad_times = [e["time"] for e in events if e["src"] == "100.64.27.1"
                             and e["sport"] == observer_port and e["dst"] == public
                             and e["time"] < local_primes[0]["time"]]
                require(qad_times and (local_primes[0]["time"] - max(qad_times)) <= 2,
                        f"expired QAD {observer_port} at send in {directory}/{name}")
                ages.append(round(1000 * (local_primes[0]["time"] - max(qad_times))))
            costs[f"{name}_oldest_qad_age_at_send_ms"] = max(ages)
            if "RandomTargetSender" in branch.group(1):
                draw = re.search(r"RANDOM_CANDIDATE observed_last=(\S+) low=(\d+) high=(\d+) draws=1 ports=1 sockets=0", log)
                require(draw is not None, f"missing bounded random draw in {directory}/{name}")
                target_port = int(branch.group(2).rsplit(":", 1)[1])
                require(int(draw.group(2)) <= target_port <= int(draw.group(3)), f"random draw outside range in {directory}/{name}")
                require(branch.group(2).rsplit(":", 1)[0] == draw.group(1).rsplit(":", 1)[0], f"random draw changed peer IP in {directory}/{name}")
        costs[f"{name}_coord_frames_sent"] = len(re.findall(r"^CONTROL_FRAME_SENT ", log, re.M))
        costs[f"{name}_coord_bytes_sent"] = sum(int(x) for x in re.findall(r"CONTROL_FRAME_SENT kind=\w+ bytes=(\d+)", log))
        costs[f"{name}_coord_frames_received"] = len(re.findall(r"^CONTROL_FRAME_RECEIVED ", log, re.M))
        costs[f"{name}_mqtt_packets"] = row[f"{name}_mqtt_packets"]
        costs[f"{name}_udp_sockets"] = row[f"{name}_udp_sockets"]
    if roles:
        require(len(roles) == 2 and len(prime_events) == 2, f"incomplete automatic pair in {directory}")
        if topology == "eim-up1":
            expected_roles = {"MappingSideSender", "StableSideReceiver"}
        elif "eim" in topology:
            expected_roles = {"IrregularReceiver", "RandomTargetSender"}
        else:
            expected_roles = {"IrregularRegularReceiver", "RegularRandomTargetSender"}
        require(set(roles.values()) == expected_roles, f"wrong automatic mode2/4 pair in {directory}")
        require(len(set(rounds.values())) == 1, f"different protected rounds in {directory}")
        for public, role in roles.items():
            other = next(ip for ip in PUBLIC if ip != public)
            ips, ports = mapped_samples[other]
            if "RandomTargetSender" in role:
                draw = re.search(r"RANDOM_CANDIDATE observed_last=(\S+) low=(\d+) high=(\d+) draws=1 ports=1 sockets=0", endpoint_logs[public])
                padding = int(row["strategy"][7:]) if row["strategy"].startswith("padding") else 4
                low, high = ((max(1, min(ports) - padding), min(65535, max(ports) + padding))
                             if max(ports) - min(ports) <= 15 else (1, 65535))
                require(draw.groups() == (f"{ips[-1]}:{ports[-1]}", str(low), str(high)),
                        f"random candidate window differs from frozen arm in {directory}")
            else:
                step = ports[-1] - ports[-2]
                require(selected[public] == f"{ips[-1]}:{ports[-1] + step}",
                        f"prime target differs from peer evidence in {directory}")
        irregular = next(public for public, role in roles.items()
                         if role.startswith("Irregular") or role == "MappingSideSender")
        sender = next(public for public in roles if public != irregular)
        require("CONTROL_FRAME_SENT kind=SENDER_PRIMED " in endpoint_logs[irregular]
                and "CONTROL_FRAME_RECEIVED kind=SENDER_PRIMED_ACK " in endpoint_logs[irregular]
                and "CONTROL_FRAME_RECEIVED kind=SENDER_PRIMED " in endpoint_logs[sender]
                and "CONTROL_FRAME_SENT kind=SENDER_PRIMED_ACK " in endpoint_logs[sender],
                f"complementary protected submission barrier missing in {directory}")
        # Local UDP completion does not acknowledge an external NAT gateway.
        # Keep actual negative deltas rather than discarding queued/reordered misses.
        times = {e["src"]: e["time"] for e in prime_events}
        costs["sender_wan_ms_after_irregular"] = round(1000 * (times[sender] - times[irregular]), 3)
        costs["wan_prime_order_reversed"] = times[sender] < times[irregular]
        costs["local_submission_barrier"] = True
        hit = all(f"{e['src']}:{e['sport']}" == selected[e["dst"]] for e in prime_events)
        costs["random_target_hit"] = hit
        if row["outcome"] == "success":
            require(hit, f"successful path with mismatched prime mappings in {directory}")
        else:
            require(all("CONTROL_ALIVE_AFTER_UDP_FAILURE" in log and "DATA_OK" not in log for log in logs.values()), f"failed path lost original control or delivered business {directory}")
    elif row["outcome"] == "bounded_fallback":
        require(costs["peer_udp_datagrams"] == 0, f"unsupported pair sent peer packets {directory}")
        require(all("DATA_OK" not in log for log in logs.values()), f"fallback delivered business {directory}")
    if strategy == "baseline":
        require(len(prime_events) == 2, f"baseline did not submit exactly two primes in {directory}")
        for name, public in (("server", server_public), ("client", client_public)):
            local_primes = [event for event in prime_events if event["src"] == public]
            require(len(local_primes) == 1 and "PROBES_SENT count=1 distinct_targets=1" in logs[name],
                    f"missing baseline local prime in {directory}/{name}")
            exchanged = re.search(r"EXCHANGED peer=\w+ peer_candidate=(\S+)(?: attempted=(\S+))? stable_id=(\d+)", logs[name])
            require(exchanged and f"{local_primes[0]['dst']}:{local_primes[0]['dport']}" == (exchanged.group(2) or exchanged.group(1)),
                    f"baseline prime target not exchanged in {directory}/{name}")
            require(set(re.findall(r"stable_id=(\d+)", logs[name])) == {exchanged.group(3)},
                    f"baseline connection changed in {directory}/{name}")
    if row["outcome"] == "success":
        require(all("DATA_OK" in log and re.search(r"DATA_OK stable_id=\d+.*paths=PathList.*ip:", log)
                    for log in logs.values()), f"original QUIC business not proven {directory}")
        # Identity is verified by QUIC; do not equate a NAT port with identity.
        client_id = re.search(r"CANDIDATE id=(\w+)", logs["client"]).group(1)
        server_id = re.search(r"CANDIDATE id=(\w+)", logs["server"]).group(1)
        require(f"EXCHANGED peer={server_id}" in logs["client"] and f"EXCHANGED peer={client_id}" in logs["server"], f"wrong TLS peer {directory}")
        require(all("UDP_COMMIT_ACK" in log for log in logs.values()), f"business before commit confirmation {directory}")
        require(costs["peer_quic_datagrams"] > 0, f"missing QUIC packets {directory}")
        for public, log in endpoint_logs.items():
            success = next(line for line in log.splitlines() if line.startswith("DATA_OK "))
            require(f"ip:{selected[public]}" in success
                    and re.search(r"selected: Some\(PathId\(\d+\)\)", success),
                    f"business did not select the approved original IP path in {directory}")
    else:
        require(all("DATA_OK" not in log for log in logs.values()), f"inconsistent success {directory}")
        if row["outcome"] != "bounded_fallback":
            require(all("PATH_UNAVAILABLE" in log for log in logs.values()),
                    f"bounded path outcome missing in {directory}")
    return {"trial": directory.name, "topology": topology, "dialer": dialer,
            "strategy": row["strategy"], "outcome": row["outcome"],
            "wall_ms": row["wall_ms"], "fixture_cpu_ms": row["fixture_cpu_ms"],
            "client_path_ms": row["client_path_ms"], "wan_capture_records": captured,
            "historical_teardown_warnings": warnings, **costs}


def main(root, repeat, topologies, arms):
    root = Path(root)
    trials = json.loads((root / "trials.json").read_text())
    expected = {(topology, dialer, trial, arm) for topology in topologies
                for dialer in ("a", "b") for trial in range(1, repeat + 1) for arm in arms}
    actual = [(row["topology"], row["dialer"], row["trial"], row["strategy"]) for row in trials]
    require(len(actual) == len(set(actual)) and set(actual) == expected,
            f"incomplete/duplicate matrix cells: expected {len(expected)}, got {len(actual)}, missing={expected - set(actual)}")
    captures = {path.parent.name for path in root.glob("*/wan.pcap")}
    require(captures == {row["evidence"] for row in trials}, "captures and declared trials differ")
    for topology in topologies:
        for dialer in ("a", "b"):
            for trial in range(1, repeat + 1):
                pair = [row for row in trials if (row["topology"], row["dialer"], row["trial"]) == (topology, dialer, trial)]
                require(len({row["fixture_seed"] for row in pair}) == 1, "paired arms use different mapping seeds")
                for name in ("client", "server"):
                    identities = {re.search(r"CANDIDATE id=(\w+)", (root / row["evidence"] / f"{name}.log").read_text()).group(1) for row in pair}
                    require(len(identities) == 1, "paired arms use different node identities")
    verified = [verify_trial(root / row["evidence"], row) for row in trials]
    (root / "verification.json").write_text(json.dumps({"trials": len(verified), "expected_repeat": repeat,
                            "expected_topologies": topologies, "expected_arms": arms, "rows": verified}, indent=2) + "\n")
    grouped = {}
    for row in verified:
        group = grouped.setdefault((row["topology"], row["strategy"]), [])
        group.append(row)
    summary = []
    for (topology, strategy), rows in grouped.items():
        successful = [r for r in rows if r["outcome"] == "success"]
        summary.append({"topology": topology, "strategy": strategy, "attempts": len(rows),
                        "success": len(successful), "hits": sum(bool(r.get("random_target_hit")) for r in rows),
                        "median_path_ms": statistics.median(r["client_path_ms"] for r in successful) if successful else None,
                        "median_fixture_cpu_ms": statistics.median(r["fixture_cpu_ms"] for r in rows),
                        **{f"median_{field}": statistics.median(r[field] for r in rows)
                           for field in ("application_primes", "peer_udp_datagrams", "qad_udp_datagrams", "peer_udp_payload_bytes", "qad_udp_payload_bytes")}})
    (root / "timing-and-costs.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"verified_trials": len(verified), "groups": summary}, indent=2))


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        raise SystemExit("usage: nat4_irregular_verify.py DIR EXPECTED_REPEAT EXPECTED_TOPOLOGIES [ARMS]")
    main(sys.argv[1], int(sys.argv[2]), sys.argv[3].split(","),
         sys.argv[4].split(",") if len(sys.argv) == 5 else ["baseline", "auto"])
