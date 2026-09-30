#!/usr/bin/env python3
"""Independent formal-entry profile, packet, identity, plan and cost verifier.

The archived Go behavior table is the oracle. No product selector/executor is
imported. Natural denominators and functional execution chains remain separate.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import re
from legacy_nat_scheduler_verify import (BEHAVIOR, PRIVATE, PUBLIC, behavior_tuple,
                                         capture, expected_plan, feature, raw,
                                         target, unique_adjacent)
from legacy_nat_multi_socket_verify import (BINDING, DATAGRAM, LISTENER, STOP, TRACE,
                                           WINNER, EvidencePath, address, records,
                                           split_random_stream)
from legacy_nat_verify import SAMPLE, SCORE, require


def sha(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def profile(directory, cell):
    wan, count = capture(directory / "wan.pcap", (directory / "tcpdump.log").read_text())
    lan, counts, allocations = {}, dict(wan=count), {}
    for node in "ab":
        lan[node], counts[node] = capture(directory / f"lan-{node}.pcap", (directory / f"tcpdump-lan-{node}.log").read_text())
        kind = cell["topology"].split("-")[0 if node == "a" else 1]
        mapping = dict(item.split("=", 1) for item in (directory / f"preflight-{node}-mapping.txt").read_text().split())
        require(mapping["primary"] != mapping["extra_source"], "physical source ports merged")
        require((mapping["primary"] == mapping["alternate_destination"]) == (kind in ("eim", "adf", "eif")), "same-IP mapping preflight differs")
        connections = (directory / f"preflight-{node}-conntrack.txt").read_text()
        primary = mapping["primary_private"]
        other_ip = re.search(rf"src={PRIVATE[node]} dst=100\.64\.27\.1 sport={primary} dport=46761 .*?src=100\.64\.27\.1 dst={PUBLIC[node]} sport=46761 dport=(\d+)", connections)
        require(other_ip, "different-IP mapping tuple missing")
        require((other_ip[1] == mapping["primary"]) == (kind in ("eim", "adf", "eif")), "different-IP mapping dependency differs")
        for source in [primary, mapping["extra_private"]]:
            for suffix in ["wrong", "wrong-ip", "right"]:
                accepted = suffix == "right" or kind == "eif" or kind == "adf" and suffix == "wrong"
                text = (directory / f"filter-{node}-{source}-{suffix}.txt").read_text()
                if accepted:
                    require(re.search(r"^\s*[1-9][0-9]*\s+\d+\s+ACCEPT.*wan0\s+lan0", text, re.M), "profile did not forward permitted source")
                else:
                    require(re.search(r"policy DROP [1-9][0-9]* packets", text), "profile did not drop forbidden source")
                marker = f"PREFLIGHT-{node}-{'wrong-port' if suffix == 'wrong' else suffix}-{source}".encode()
                delivered = [e for e in lan[node] if e["dst"] == PRIVATE[node] and e["payload"] == marker]
                wire = [e for e in wan if e["dst"] == PUBLIC[node] and e["payload"] == marker]
                require(wire and bool(delivered) == accepted, "actual filter delivery differs from counters")
                if kind not in ("eif", "adf") and suffix == "right":
                    require(all(e["sport"] == 50000 for e in wire), "matched APDF source was rewritten")
        fixture = (directory / "fixture.sh").read_text()
        require('-t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP' in fixture, "missing Internet low-TTL loss")
        nft = (directory / f"nft-{node}.txt").read_text()
        require("type mark : inet_service" in nft and f"snat to {PUBLIC[node]}:meta mark map @ports" in nft, "not real kernel per-tuple SNAT")
        text = (directory / f"allocator-{node}.log").read_text().split("ALLOCATOR_RESET\n")
        require(len(text) == 2, "preflight did not reset once")
        allocations[node] = [json.loads(line) for line in text[1].splitlines() if line.startswith("{")]
        require(allocations[node] and allocations[node][-1].get("summary"), "allocator not joined/accounted")
    return wan, lan, counts, allocations


def phase(directory, cell, logs, samples, number, wan, lan, allocations, oracle):
    plan, scores, behavior = expected_plan(samples, number, oracle)
    selected = {s: [line for line in log.splitlines() if line.startswith("LEGACY_SELECTED ")][number] for s, log in logs.items()}
    round_ids = [re.search(r" round=([0-9a-f]{32}) ", line)[1] for line in selected.values()]
    require(len(set(round_ids)) == 1, "protected peer round mismatch")
    round_id = round_ids[0]
    result = dict(mode=plan[0], index=plan[1], round=round_id, costs={})
    starts = []
    for side, log in logs.items():
        node = cell["dialer"] if side == "client" else ("b" if cell["dialer"] == "a" else "a")
        first = next((e["time"] for e in lan[node] if raw(e) and e["src"] == PRIVATE[node]
                      and e["payload"][11:27].hex() == round_id and e["payload"][27] == 0), None)
        elapsed = re.search(rf"^LEGACY_RECV_START round={round_id} .* bind_elapsed_ms=(\d+) ", log, re.M)
        if first is not None and elapsed:
            starts.append(first - int(elapsed[1]) / 1000)
    phase_start = min(starts, default=None)
    for side, log in logs.items():
        node = cell["dialer"] if side == "client" else ("b" if cell["dialer"] == "a" else "a")
        other = "b" if node == "a" else "a"
        peer_side = "server" if side == "client" else "client"
        current = selected[side]
        require(SCORE.findall(current) == [(str(m), str(i), str(score)) for m, i, score in scores], "first-max recommendation/one penalty/cache sequence differs")
        require(BEHAVIOR.findall(current) == [behavior_tuple(behavior["server"]), behavior_tuple(behavior["client"])], "old parameters or role swap differs")
        require(re.search(r"mode=(\d+) index=(\d+)", current).groups() == tuple(map(str, plan)), "selected mode/index differs")
        stable = re.search(r" stable_id=(\d+) ", current)[1]
        require(set(re.findall(r"stable_id=(\d+)", log)) == {stable}, "original stable Connection changed")
        b = behavior[side]
        total = max(behavior["server"]["SendDelayMs"], behavior["client"]["SendDelayMs"]) + 5000
        if any(behavior[s]["ListenRandomPorts"] for s in behavior):
            total += 30000
        start = next(line for line in log.splitlines() if line.startswith(f"LEGACY_RECV_START round={round_id} "))
        require(f"total_budget_ms: {total}, read_budget_ms: {total - b['SendDelayMs']}" in start, "old absolute/read deadline differs")
        listener = [m for m in LISTENER.findall(log) if m[0] == round_id]
        bindings = {int(m[2]): address(m[3])[1] for m in listener}
        require(bindings and len(bindings) == len(listener) == len(set(bindings.values())), "physical bindings merged/missing")
        require(all(m[1] == stable and (m[4] == "true") == (m[2] == "0") for m in listener), "listener ownership differs")
        local = [e for e in lan[node] if raw(e) and e["src"] == PRIVATE[node] and e["payload"][11:27].hex() == round_id]
        incoming = [e for e in lan[node] if raw(e) and e["dst"] == PRIVATE[node] and e["payload"][11:27].hex() == round_id]
        # Log blocks end with the exact round's phase/trace. Scope checks and
        # owned futures cancel before the next round's registration.
        begin = log.index(start)
        trace_match = next(m for m in TRACE.finditer(log, begin) if m[1] == round_id)
        block = log[begin:trace_match.end()]
        sends = DATAGRAM.findall(block)
        expected = collections.Counter((dst, 64 if ttl == "0" or degraded == "true" else int(ttl), response == "true", bindings[int(socket)]) for dst, ttl, response, degraded, socket in sends)
        observed = collections.Counter((target(e), e["ttl"], e["payload"][27] == 1, e["sport"]) for e in local)
        require(expected == observed and local, "actual LAN target/TTL/physical submissions differ")
        require(all(len(e["payload"]) == 76 for e in local + incoming), "raw frame truncated/foreign")
        requests = [e for e in local if e["payload"][27] == 0]
        responses = [e for e in local if e["payload"][27] == 1]
        if requests and b["SendDelayMs"]:
            require(phase_start is not None and requests[0]["time"] - phase_start >= b["SendDelayMs"] / 1000 - .25, "packet emitted before actual old send delay")
        trace = {key: int(value) for key, value in (part.split("=", 1) for part in trace_match[3].split())}
        require(trace["probes"] == len(requests) and trace["responses"] == len(responses), "probe/response cost differs")
        require(trace["send_attempts"] == len(local) + trace["send_errors"] + trace["send_aborted"] and trace["send_errors"] == 0, "attempted/actual/error/aborted distinction missing")
        require(trace["ttl_degraded"] == sum(m[3] == "true" for m in sends), "TTL degradation ledger differs")
        require(trace["listener_attempted"] == b["ListenRandomPorts"] and trace["listener_bound"] + trace["listener_failed"] == trace["listener_attempted"] and len(bindings) == 1 + trace["listener_bound"], "old256 bind attempts shrunk or unaccounted")
        require(0 <= trace["random_iterations"] <= b["PortsRandomNumber"] and trace["random_iterations"] <= trace["random_draws"] <= trace["random_iterations"] * 10, "old random iteration/ten-draw bound differs")
        explicit = unique_adjacent(samples[peer_side]) if b["Role"] == "sender" or b["PortsRangeNumber"] == 0 else []
        ranges = []
        if b["PortsRangeNumber"]:
            last = address(samples[peer_side][-1])[1]
            difference = feature(samples[peer_side])[2]
            radius = b["PortsRangeNumber"]
            low, high = max(last - difference - 5, last - radius, 1), min(last + difference + 5, last + radius, 65535)
            ranges = [f"{ip}:{port}" for ip in unique_adjacent([address(s)[0] for s in samples[peer_side]]) for port in range(low, high + 1)]
        by_socket = collections.defaultdict(list)
        for e in requests:
            by_socket[e["sport"]].append(e)
        require(set(by_socket) == set(bindings.values()), "bound socket skipped its complete phase")
        random_packets = 0
        for events in by_socket.values():
            first = explicit + ranges
            require([target(e) for e in events[:len(first)]] == first, "first explicit/inclusive range order differs")
            if ranges:
                burst = events[len(explicit):len(first)]
                require(all(.0015 <= y["time"] - x["time"] <= .3 for x, y in zip(burst, burst[1:])), "range2ms pacing differs")
                require(not explicit or [target(e) for e in events[len(first):]] == explicit * (len(events[len(first):]) // len(explicit)), "range restarted or explicit repeats changed")
            elif b["PortsRandomNumber"]:
                attempted = trace["random_iterations"] - trace["random_skipped"]
                for count in range(attempted, max(-1, attempted - trace["send_aborted"] - 1), -1):
                    try:
                        draws, repeated = split_random_stream(events[len(first):], explicit, count, events[len(first) - 1]["time"])
                        break
                    except ValueError:
                        continue
                else:
                    raise ValueError("random stream/explicit timing cannot reconcile")
                require(all(1024 <= e["dport"] <= 65534 for e in draws) and len({e["dport"] for e in draws}) == len(draws), "old random interval/uniqueness differs")
                random_packets += len(draws)
            else:
                require([target(e) for e in events] == explicit * (len(events) // len(explicit)), "ordinary repeats differ")
        if explicit and not b["PortsRandomNumber"]:
            burst = [(dst, port) for dst in explicit for port in bindings.values()]
            initial_explicit = len(explicit) * len(bindings)
            initial_range = len(ranges) * len(bindings)
            ordinary_events = requests[:initial_explicit] + requests[initial_explicit + initial_range:]
            ordinary = [(target(e), e["sport"]) for e in ordinary_events]
            require(ordinary == [burst[i % len(burst)] for i in range(len(ordinary))], "target-outer/socket-inner repeats differ")
            ticks = [ordinary_events[i]["time"] for i in range(initial_explicit, len(ordinary_events), len(burst))]
            require(all(.16 <= y - x <= .6 for x, y in zip(ticks, ticks[1:])), "global explicit200ms timer differs")
        require(0 <= trace["random_iterations"] - trace["random_skipped"] - random_packets <= trace["send_aborted"], "random iteration vs actual sends differs")
        if responses:
            require(b["Role"] == "receiver" and all(e["ttl"] == 64 for e in responses), "role/defaultTTL response differs")
            groups = collections.defaultdict(list)
            for e in responses:
                groups[(e["sport"], e["payload"][28:44])].append(e)
                require(any(q["dport"] == e["sport"] and q["src"] == e["dst"] and q["sport"] == e["dport"] and q["payload"][28:44] == e["payload"][28:44] for q in incoming), "response source/socket/nonce differs")
            require(any(len(events) == 3 for events in groups.values()), "winning response burst incomplete")
            require(all(1 <= len(events) <= 3 and all(.035 <= y["time"] - x["time"] <= .3 for x, y in zip(events, events[1:])) for events in groups.values()), "three responses/50ms pacing differs")
        mapped = {tuple(row["tuple"]): tuple(row["mapped"]) for row in allocations[node] if "tuple" in row}
        packets = collections.defaultdict(list)
        for e in wan:
            if raw(e) and e["src"] == PUBLIC[node] and e["packet_type"] == 0 and e["payload"][11:27].hex() == round_id:
                packets[e["payload"]].append(e)
        for e in local:
            require(packets[e["payload"]], "actual LAN submission absent from WAN")
            translated = packets[e["payload"]].pop(0)
            key = (PRIVATE[node], e["sport"], e["dst"], e["dport"])
            require(key in mapped and (translated["src"], translated["sport"]) == mapped[key], "physical four-tuple source mapping differs")
            require((translated["dst"], translated["dport"], translated["ttl"]) == (e["dst"], e["dport"], e["ttl"] - 1), "WAN target/hop differs")
        require(not any(packets.values()), "unaccounted WAN raw submissions")
        other_in = [e for e in lan[other] if raw(e) and e["dst"] == PRIVATE[other]]
        require(not any(e["payload"] == q["payload"] for e in other_in for q in requests if q["ttl"] < 8), "lowTTL reached opposite LAN")
        stop = next(m for m in STOP.finditer(log) if m[1] == round_id)
        require(stop[2] == stable and int(stop[3]) <= total + 1000, "phase ownership/absolute deadline changed")
        if stop[4] == "failed":
            require(int(stop[3]) >= total - 50, "early/infrastructure failure masquerades as bounded NAT miss")
            require(trace["random_iterations"] == b["PortsRandomNumber"], "complete miss skipped old random work")
        if "DATA_OK" in log:
            winner = next(m for m in WINNER.finditer(log) if m[1] == round_id)
            binding = BINDING.search(log)
            require(binding and binding[1] == stable and binding[2] == cell["pair"][peer_side] and binding[4] == winner[9] and binding[5] == winner[10] and binding[6] == winner[3], "business/original TLS peer/exact physical winner path differs")
            selected_path = re.search(r"selected: Some\(PathId\((\d+)\)\)", next(line for line in log.splitlines() if line.startswith("DATA_OK ")))
            require(selected_path and selected_path[1] == binding[3], "selected pathID differs from business path")
            port = address(winner[10])[1]
            remote = address(winner[3])
            require(int(winner[9]) in bindings and bindings[int(winner[9])] == port, "winner physical ownership differs")
            quic = [e for e in lan[node] if e["src"] == PRIVATE[node] and e["sport"] == port and (e["dst"], e["dport"]) == remote and e["payload"][0] & 0x40]
            require(quic and all(e["ttl"] == 64 for e in quic), "no original QUIC on winner/defaultTTL contaminated")
            require(any(e["dport"] == port and (e["src"], e["sport"]) == remote for e in incoming), "no received physical winner tuple")
            require(re.search(rf"^LEGACY_WINNER_READY round={round_id} stable_id={stable} remaining_ms=[1-9]\d*$", log, re.M), "protected bilateral winner readiness absent")
            if side == "server":
                require("outcome=confirmed_business learned=true score_delta=2" in log, "confirmed business did not receive single+2 feedback")
        result["costs"][side] = dict(trace, elapsed_ms=int(stop[3]), local_sockets=len(bindings))
    return result


def verify(directory, original_cell, plan, generation):
    directory = EvidencePath(directory)
    require(json.loads((directory / "cell.json").read_text()) == original_cell, "frozen cell identity changed")
    cell = dict(original_cell, pair=plan["pair"])
    run = dict(item.split("=", 1) for item in (directory / "run.txt").read_text().split())
    require(run == dict(case=cell["case"], strategy=cell["strategy"], topology=cell["topology"], dialer=cell["dialer"], seed=cell["seed"], client_rc="0", server_rc="0"), "exact exit/seed/provenance mismatch")
    require(not re.search(r"Traceback|Operation not permitted|No such file", (directory / "harness.log").read_text()), "fixture failure is not NAT miss")
    wan, lan, counts, allocations = profile(directory, cell)
    logs = {side: (directory / f"{side}.log").read_text() for side in ["server", "client"]}
    require(all(not re.search(r"^Error:|panicked|Traceback", log, re.M) for log in logs.values()), "unexpected runtime error")
    success = ["DATA_OK" in log for log in logs.values()]
    require(success[0] == success[1], "peer business outcomes disagree")
    result = dict(cell=cell["id"], generation=generation, captures=counts, outcome="confirmed_business" if all(success) else "bounded_miss", phases=[], cpu={})
    for side, log in logs.items():
        cpu = json.loads((directory / f"{side}-cpu.json").read_text())
        require(cpu["collector"] == "namespace-local wait4" and cpu["scope"] == "actual native node process" and cpu["exit"] == 0 and not cpu["timed_out"] and cpu["executable_sha256"] == plan["runtime"]["udp_nat_probe"], "wrong process CPU scope/version/exit")
        require(cpu["user_cpu_seconds"] > 0 and cpu["system_cpu_seconds"] >= 0 and cpu["wall_seconds"] > 0 and cpu["max_rss_kib"] > 0 and cpu["sampled_exec_vm_hwm_kib"] > 0 and cpu["exec_samples"] > 0 and cpu["sampled_maximum_fd"] > 0, "required actual node costs absent")
        result["cpu"][side] = cpu
        ids = set(re.findall(r"stable_id=(\d+)", log))
        require(len(ids) == 1, "original Connection continuity missing")
        local = re.search(r"^NODE_SESSION local=([0-9a-f]{64}) peer=([0-9a-f]{64})", log, re.M) if cell["strategy"] == "formal" else re.search(r"^CANDIDATE id=([0-9a-f]{64})", log, re.M)
        require(local and local[1] == cell["pair"][side], "actual authenticated local identity changed")
        if cell["strategy"] == "formal":
            require(local[2] == cell["pair"]["client" if side == "server" else "server"], "TLS peer changed")
        if not all(success):
            require(re.search(rf"^EARLY_FAILURE_CONTROL_ALIVE stable_id={next(iter(ids))}$", log, re.M) and "learned=true" not in log, "miss lost original authorized control or learned")
    if cell["strategy"] == "baseline":
        require(all("LEGACY_SELECTED" not in log and "PROBES_SENT count=1 distinct_targets=1" in log for log in logs.values()), "baseline is not frozen one-candidate slice")
        require(all(not raw(e) for e in wan), "baseline unexpectedly ran full executor")
        result["kind"] = "baseline"
    elif any("NAT_DIRECT_SUCCESS" in log for log in logs.values()):
        require(all(success) and all("NAT_DIRECT_SUCCESS" in log and "punching_recommendations=0" in log and "LEGACY_SELECTED" not in log and "learned=true" not in log for log in logs.values()), "direct result masquerades as punching/learning")
        result["kind"] = "direct"
    else:
        result["kind"] = "punching"
        samples = {}
        for side, log in logs.items():
            ss = SAMPLE.findall(log)
            require([int(m[0]) for m in ss] == [0, 1, 2, 3] * cell["rounds"], "fresh per-round actual S observations missing")
            samples[side] = [m[2] for m in ss]
            for _, dst, src in ss:
                sip, sport = address(src); dip, dport = address(dst)
                require(any((e["src"], e["sport"], e["dst"], e["dport"]) == (sip, sport, dip, dport) for e in wan) and any((e["src"], e["sport"], e["dst"], e["dport"]) == (dip, dport, sip, sport) for e in wan), "actual observer request/reply tuple missing")
            require(len(re.findall(r"^LEGACY_SELECTED ", log, re.M)) == cell["rounds"], "more/fewer recommendations than bounded rounds")
        oracle = json.loads((pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/legacy_nat_behaviors.json").read_text())
        for number in range(cell["rounds"]):
            current = {side: addresses[number * 4:number * 4 + 4] for side, addresses in samples.items()}
            result["phases"].append(phase(directory, cell, logs, current, number, wan, lan, allocations, oracle))
        require(len({p["round"] for p in result["phases"]}) == cell["rounds"], "round nonce reused")
        for side in logs:
            costs = [p["costs"][side] for p in result["phases"]]
            require(result["cpu"][side]["wall_seconds"] >= sum(p["elapsed_ms"] for p in costs) / 1000 - .05, "wrong child walltime versus actual owned phases")
            require(result["cpu"][side]["sampled_maximum_fd"] >= max(p["local_sockets"] for p in costs), "actual socket resources exceed measured node FD inventory")
    for node in "ab":
        side = "client" if node == cell["dialer"] else "server"
        product = [e for e in lan[node] if e["src"] == PRIVATE[node] and not e["payload"].startswith(b"PREFLIGHT")]
        result.setdefault("packet_costs", {})[side] = dict(udp_datagrams=len(product), udp_payload_bytes=sum(len(e["payload"]) for e in product), qad_datagrams=sum(e["dst"] == "100.64.27.1" and 46761 <= e["dport"] <= 46764 for e in product), quic_peer_datagrams=sum(e["dst"] == PUBLIC["b" if node == "a" else "a"] and bool(e["payload"]) and bool(e["payload"][0] & 0x40) for e in product))
    if cell.get("core", 100) <= 4 and cell["strategy"] == "formal":
        require(all(success), "old easy core01-04 requires real QUIC/payload positive")
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    parser.add_argument("--partial", action="store_true")
    args = parser.parse_args()
    v = args.evidence / "verification"
    plans = {1: json.loads((v / "frozen-plan.json").read_text()), 2: json.loads((v / "frozen-plan-generation2.json").read_text())}
    generations = json.loads((v / "runtime-by-cell.json").read_text())
    results = {}
    for group in ["comparison", "functional"]:
        rows = []
        for cell in plans[1][group]:
            directory = args.evidence / group / cell["id"]
            if not (directory / "harness.log").exists():
                require(args.partial, "frozen cell missing: " + cell["id"])
                break
            generation = generations[cell["id"]] if group == "comparison" else 2
            rows.append(verify(directory, cell, plans[generation], generation))
        results[group] = rows
    if not args.partial:
        require(len(results["comparison"]) == 40 and len(results["functional"]) == 3, "declared denominator changed")
        covered = {(p["mode"], p["index"]) for row in results["functional"] for p in row["phases"]}
        expected = {(row["Mode"], row["Index"]) for row in json.loads((pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/legacy_nat_behaviors.json").read_text())}
        require(covered == expected, "full30 formal executor behaviors absent")
    result = dict(result="PARTIAL" if args.partial else "PASS", comparison_denominator=40, functional_probability_samples_added=0, **results)
    (v / ("strict-partial.json" if args.partial else "strict.json")).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"result":result["result"],"comparison_checked":len(results["comparison"]),"functional_checked":len(results["functional"])}))


if __name__ == "__main__":
    main()
