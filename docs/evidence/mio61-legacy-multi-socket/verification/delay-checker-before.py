#!/usr/bin/env python3
"""Strict independent packet/physical-binding/old-plan check for MIOPUNCH-61."""
import argparse
import collections
import hashlib
import json
import pathlib
import re
import sys
from legacy_nat_scheduler_verify import (
    BEHAVIOR, PRIVATE, PUBLIC, behavior_tuple, capture, expected_plan, feature,
    raw, target, unique_adjacent,
)
from legacy_nat_verify import SAMPLE, SCORE, require
from legacy_nat_multi_socket_matrix import cells, name

DATAGRAM = re.compile(r"^LEGACY_DATAGRAM target=(\S+) ttl=(\d+) response=(true|false) degraded=(true|false) local_socket_id=(\d+)$", re.M)
LISTENER = re.compile(r"^LEGACY_LISTENER round=([0-9a-f]{32}) stable_id=(\d+) local_socket_id=(\d+) local_addr=(\S+) primary=(true|false)$", re.M)
WINNER = re.compile(r"^LEGACY_WINNER round=([0-9a-f]{32}) stable_id=(\d+) remote=(\S+) response=(true|false) before_probe=(true|false) probes=(\d+) responses=(\d+) ttl_degraded=(\d+) local_socket_id=(\d+) local_addr=(\S+)$", re.M)
BINDING = re.compile(r"^LEGACY_BUSINESS_BINDING stable_id=(\d+) peer=([0-9a-f]{64}) path_id=(\d+) local_socket_id=(\d+) local_addr=(\S+) remote=ip:(\S+)$", re.M)
TRACE = re.compile(r"^LEGACY_TRACE round=([0-9a-f]{32}) stable_id=(\d+) (.+)$", re.M)
STOP = re.compile(r"^LEGACY_PHASE_STOP round=([0-9a-f]{32}) stable_id=(\d+) elapsed_ms=(\d+) outcome=(winner|failed)$", re.M)


def address(value):
    ip, port = value.rsplit(":", 1)
    return ip, int(port)


def records(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]


def nonce(event):
    return event["payload"][28:44]


def split_random_stream(events, explicit, random_count, initial_time):
    """Find the actual paced random stream among interleaved repeat packets.

    A random draw may equal an explicit target. The exact trace iteration
    count, ordered explicit subsequence, timestamps and one-use random ports
    constrain that ambiguity; contiguous burst assumptions would be wrong.
    """
    explicit_count = len(events) - random_count
    require(explicit_count >= 0, "random ledger exceeds actual remaining sends")
    target_bits = {value: 1 << i for i, value in enumerate(dict.fromkeys(explicit))}
    forced = [e["dport"] for e in events if target(e) not in target_bits]
    require(len(forced) == len(set(forced)), "random call repeated a non-explicit target")
    require(explicit or explicit_count == 0, "unexpected repeated work without explicit targets")
    sys.setrecursionlimit(max(sys.getrecursionlimit(), len(events) * 2 + 1000))
    decisions = []
    failed = set()

    def search(index, sent_explicit, last_random, group_start, used_ambiguous):
        state = (index, sent_explicit, last_random, group_start, used_ambiguous)
        if state in failed:
            return False
        if index == len(events):
            return sent_explicit == explicit_count
        event = events[index]
        value = target(event)
        if sent_explicit < explicit_count and value == explicit[sent_explicit % len(explicit)]:
            new_group = sent_explicit % len(explicit) == 0
            previous = initial_time if group_start < 0 else events[group_start]["time"]
            valid = (.16 <= event["time"] - previous <= .45) if new_group else (event["time"] - events[group_start]["time"] <= .15)
            if valid:
                decisions.append(False)
                if search(index + 1, sent_explicit + 1, last_random,
                          index if new_group else group_start, used_ambiguous):
                    return True
                decisions.pop()
        sent_random = index - sent_explicit
        bit = target_bits.get(value, 0)
        if sent_random < random_count and not (used_ambiguous & bit):
            if last_random < 0 or event["time"] - events[last_random]["time"] >= .012:
                decisions.append(True)
                if search(index + 1, sent_explicit, index, group_start, used_ambiguous | bit):
                    return True
                decisions.pop()
        failed.add(state)
        return False

    require(search(0, 0, -1, -1, 0), "random/explicit order, uniqueness or 15ms/200ms pacing differs")
    return ([event for event, is_random in zip(events, decisions) if is_random],
            [event for event, is_random in zip(events, decisions) if not is_random])


def verify(directory, cell, oracle):
    require(json.loads((directory / "cell.json").read_text()) == cell, "cell identity mismatch")
    run = dict(x.split("=", 1) for x in (directory / "run.txt").read_text().split())
    expected_rc = "1" if cell["case"] in ("revoked", "no_secret", "wrong_secret") else "0"
    require(run == dict(case=cell["case"], strategy="legacy", topology=cell["topology"],
                        dialer=cell["dialer"], seed=cell["seed"], client_rc=expected_rc,
                        server_rc=expected_rc), "exact exit/run provenance mismatch")
    harness = (directory / "harness.log").read_text()
    require(not re.search(r"Traceback|Operation not permitted|No such file or directory|infrastructure/fixture failure", harness), "infrastructure error is not a NAT miss")
    fixture = (directory / "fixture.sh").read_text()
    require('-p udp -d "$peer_ip" -j NFQUEUE --queue-num 61' in fixture, "not every physical source enters allocator")
    require('-p udp --sport "$source" -d "$peer_ip" -j NFQUEUE' not in fixture, "extra physical sources bypass allocator")
    require('-t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP' in fixture, "missing pre-conntrack Internet TTL loss")
    wan, nwan = capture(directory / "wan.pcap", (directory / "tcpdump.log").read_text())
    lan, counts = {}, {"wan": nwan}
    allocations = {}
    for node in "ab":
        lan[node], counts[node] = capture(directory / f"lan-{node}.pcap", (directory / f"tcpdump-lan-{node}.log").read_text())
        mapping = dict(x.split("=", 1) for x in (directory / f"preflight-{node}-mapping.txt").read_text().split())
        require(mapping["primary"] != mapping["extra_source"], "different physical ports merged in preflight")
        kind = cell["topology"].split("-")[0 if node == "a" else 1]
        require((mapping["primary"] == mapping["alternate_destination"]) == (kind == "eim"), "mapping dependency not independently established")
        for source in [mapping["primary_private"], mapping["extra_private"]]:
            for suffix in ["wrong", "wrong-ip", "right"]:
                content = (directory / f"filter-{node}-{source}-{suffix}.txt").read_text()
                if suffix == "right":
                    require(re.search(r"^\s*[1-9][0-9]*\s+\d+\s+ACCEPT.*wan0\s+lan0", content, re.M), "matched physical-source reply not forwarded")
                else:
                    require(re.search(r"policy DROP [1-9][0-9]* packets", content), "wrong source IP/port not dropped")
        nft = (directory / f"nft-{node}.txt").read_text()
        require("type mark : inet_service" in nft and f"snat to {PUBLIC[node]}:meta mark map @ports" in nft, "no real-kernel mark-to-port SNAT")
        data = records(directory / f"allocator-{node}.log")
        require(data and data[-1].get("summary"), "allocator did not join or record final costs")
        # Preflight is reset before product, so retain only the product ledger.
        text = (directory / f"allocator-{node}.log").read_text().split("ALLOCATOR_RESET\n")
        require(len(text) == 2, "preflight allocations were not reset exactly once")
        allocations[node] = [json.loads(line) for line in text[1].splitlines() if line.startswith("{")]
    logs = {side: (directory / (side + ".log")).read_text() for side in ["server", "client"]}
    all_outgoing_raw = [event for node in "ab" for event in lan[node] if raw(event) and event["src"] == PRIVATE[node]]
    first_raw_time = min((event["time"] for event in all_outgoing_raw), default=None)
    peers = {side: re.search(r"^CANDIDATE id=([0-9a-f]{64})", log, re.M).group(1) for side, log in logs.items()}
    for side, log in logs.items():
        errors = re.findall(r"^Error:.*$", log, re.M)
        allowed = []
        if cell["case"] in ("revoked", "no_secret", "wrong_secret"):
            allowed = ["Error: Connection was rejected locally" if side == "client" else "Error: Custom { kind: Other, error: Connection was rejected locally }"]
        require(errors == allowed and not re.search(r"thread .*panicked|^error:|Traceback", log, re.M), "unexpected runtime failure")
    row = dict(cell=name(cell), captured_packets=counts, peer_probes=0, peer_responses=0)
    if cell["mode"] is None:
        require(not any(raw(e) for e in wan), "admission negative sent traversal traffic")
        for side, log in logs.items():
            require(not any(marker in log for marker in ["LEGACY_SAMPLE", "LEGACY_SELECTED", "LEGACY_LISTENER", "LEGACY_DATAGRAM", "DATA_OK", "learned=true"]), "work or data before admission")
            if cell["case"] == "nonmember":
                other = "client" if side == "server" else "server"
                require(re.search(rf"^ADMISSION_REJECTED peer={peers[other]} stable_id=\d+ probes=0$", log, re.M), "wrong nonmember identity rejection")
        row.update(outcome="admission_rejection")
        return row
    samples = {side: [m[2] for m in SAMPLE.findall(log)] for side, log in logs.items()}
    for side, log in logs.items():
        ss = SAMPLE.findall(log)
        require([int(m[0]) for m in ss] == [0, 1, 2, 3], "four ordered S observations missing")
        for _, dst, src in ss:
            sip, sport = address(src); dip, dport = address(dst)
            require(any(e["src"] == sip and e["sport"] == sport and e["dst"] == dip and e["dport"] == dport for e in wan), "S outbound source tuple absent")
            require(any(e["src"] == dip and e["sport"] == dport and e["dst"] == sip and e["dport"] == sport for e in wan), "actual observer reply absent")
    plan, scores, behaviors = expected_plan(samples, cell["history"], oracle)
    require(plan == (cell["mode"], cell["index"]), "expected old mode/index not selected")
    require(f"LEGACY_TEST_HISTORY previous_selections={cell['history']}" in logs["server"], "history feature did not run")
    rounds, successes, nonprimary = [], [], []
    costs = {}
    for side, log in logs.items():
        node = cell["dialer"] if side == "client" else ("b" if cell["dialer"] == "a" else "a")
        other = "b" if node == "a" else "a"
        peer_side = "client" if side == "server" else "server"
        selected = next(line for line in log.splitlines() if line.startswith("LEGACY_SELECTED "))
        require(SCORE.findall(selected) == [(str(m), str(i), str(score)) for m, i, score in scores], "protected score/tie history differs")
        require(BEHAVIOR.findall(selected) == [behavior_tuple(behaviors["server"]), behavior_tuple(behaviors["client"])], "old behavior parameters or role swap differs")
        round_id = re.search(r" round=([0-9a-f]{32}) ", selected).group(1)
        rounds.append(round_id)
        stable = re.search(r" stable_id=(\d+) ", selected).group(1)
        require(set(re.findall(r"stable_id=(\d+)", log)) == {stable}, "original connection stable identity changed")
        behavior = behaviors[side]
        receive = next(line for line in log.splitlines() if line.startswith("LEGACY_RECV_START "))
        require("total_budget_ms: 38000, read_budget_ms: " + str(38000 - behavior["SendDelayMs"]) in receive, "old total/read budget differs")
        require(log.index(receive) < log.index("LEGACY_DATAGRAM"), "receive registration came after send")
        listeners = LISTENER.findall(log)
        require(len(listeners) == 1 + behavior["ListenRandomPorts"], "not all actual physical listeners reported")
        bindings = {int(m[2]): address(m[3])[1] for m in listeners}
        require(len(bindings) == len(listeners) and len(set(bindings.values())) == len(listeners), "physical identities or bound ports merged")
        require(all(m[0] == round_id and m[1] == stable and (m[4] == "true") == (m[2] == "0") for m in listeners), "listener ownership/primary metadata differs")
        local = [e for e in lan[node] if raw(e) and e["src"] == PRIVATE[node]]
        incoming = [e for e in lan[node] if raw(e) and e["dst"] == PRIVATE[node]]
        require(all(len(e["payload"]) == 76 and e["payload"][11:27].hex() == round_id for e in local + incoming), "raw round/frame mismatch")
        listed = DATAGRAM.findall(log)
        observed = collections.Counter((target(e), e["ttl"], e["payload"][27] == 1, e["sport"]) for e in local)
        expected = collections.Counter()
        for dst, ttl, response, degraded, socket_id in listed:
            require(int(socket_id) in bindings, "send used foreign physical socket")
            actual_ttl = 64 if ttl == "0" or degraded == "true" else int(ttl)
            require(degraded == ("true" if cell["variant"] == "ttl_unavailable" and int(ttl) > 0 else "false"), "TTL degradation reporting differs")
            expected[(dst, actual_ttl, response == "true", bindings[int(socket_id)])] += 1
        require(expected == observed and len(local) == len(listed) > 0, "actual LAN packets differ from target/TTL/physical-socket trace")
        requests = [e for e in local if e["payload"][27] == 0]
        responses = [e for e in local if e["payload"][27] == 1]
        require(all(e["ttl"] == 64 for e in responses), "reply burst lowered default TTL")
        row["peer_probes"] += len(requests); row["peer_responses"] += len(responses)
        if behavior["SendDelayMs"] and requests:
            require(first_raw_time is not None and requests[0]["time"] - first_raw_time >= behavior["SendDelayMs"] / 1000 - .25, "sender emitted before the old3000ms delay")
        if responses:
            require(behavior["Role"] == "receiver", "sender answered request")
            by_reply = collections.defaultdict(list)
            for event in responses:
                by_reply[(event["sport"], nonce(event))].append(event)
                require(any(q["dport"] == event["sport"] and q["src"] == event["dst"] and q["sport"] == event["dport"] and nonce(q) == nonce(event) and q["payload"][27] == 0 for q in incoming), "response did not use the receiving physical socket/source/nonce")
            require(any(len(events) == 3 for events in by_reply.values()), "no complete winning three-response burst")
            for events in by_reply.values():
                require(1 <= len(events) <= 3, "too many responses per nonce/socket")
                require(all(.035 <= events[i + 1]["time"] - events[i]["time"] <= .3 for i in range(len(events) - 1)), "50ms reply interval differs")
        explicit = unique_adjacent(samples[peer_side]) if behavior["Role"] == "sender" or behavior["PortsRangeNumber"] == 0 else []
        range_targets = []
        if behavior["PortsRangeNumber"]:
            last = address(samples[peer_side][-1])[1]
            difference = feature(samples[peer_side])[2]
            radius = behavior["PortsRangeNumber"]
            low, high = max(last - difference - 5, last - radius, 1), min(last + difference + 5, last + radius, 65535)
            range_targets = [f"{ip}:{port}" for ip in unique_adjacent([address(s)[0] for s in samples[peer_side]]) for port in range(low, high + 1)]
            require(len(range_targets) <= 5, "mode4 range2 exceeds inclusive five ports")
        trace_match = TRACE.search(log)
        require(trace_match and trace_match[1] == round_id and trace_match[2] == stable, "trace identity missing")
        trace = {key: int(value) for key, value in (field.split("=", 1) for field in trace_match[3].split())}
        # Generation1 logged entered submissions and actual success/errors;
        # its explicit aborted category is derived from those recorded costs.
        # Generation2 reports the same category directly from owned RAII work.
        recorded_aborted = trace["send_attempts"] - len(local) - trace["send_errors"]
        require(recorded_aborted >= 0, "negative aborted-send ledger")
        trace.setdefault("send_aborted", recorded_aborted)
        require(trace["listener_attempted"] == behavior["ListenRandomPorts"] and trace["listener_bound"] == behavior["ListenRandomPorts"] and trace["listener_failed"] == 0, "256 attempts/resources silently reduced")
        require(trace["probes"] == len(requests) and trace["responses"] == len(responses) and trace["send_attempts"] == len(local) + trace["send_errors"] + trace["send_aborted"] and trace["send_aborted"] == recorded_aborted and trace["send_errors"] == 0, "submitted/error/aborted packet cost ledger differs")
        require(trace["ttl_degraded"] == sum(m[3] == "true" for m in listed), "TTL cost ledger differs")
        require(0 <= trace["random_iterations"] <= 1000 and trace["random_iterations"] <= trace["random_draws"] <= 10 * trace["random_iterations"], "random iterations/ten-draw bounds differ")
        require(trace["random_skipped"] <= trace["random_iterations"], "random skip ledger invalid")
        by_port = collections.defaultdict(list)
        for event in requests:
            by_port[event["sport"]].append(event)
        require(set(by_port) == set(bindings.values()), "a successful physical listener never sent its old phase")
        random_count = 0
        for port, events in by_port.items():
            first = explicit + range_targets
            require([target(e) for e in events[:len(first)]] == first, "physical first explicit/range burst order/count differs")
            if range_targets:
                burst = events[len(explicit):len(first)]
                require(all(.0015 <= burst[i + 1]["time"] - burst[i]["time"] <= .3 for i in range(len(burst) - 1)), "2ms range pacing differs")
                require(len(events) == len(first), "range repeated on physical listener")
            elif behavior["PortsRandomNumber"]:
                # Literal per-socket random stream interleaves with the ordered
                # explicit repetitions. Remove complete repetition groups.
                rest = events[len(first):]
                # A winner can cancel the last entered random submission
                # before any packet reaches the kernel. Try only the explicit
                # recorded aborted count; never hide missing captured sends.
                attempted_random = trace["random_iterations"] - trace["random_skipped"]
                actual_random = attempted_random
                while actual_random >= max(0, attempted_random - trace["send_aborted"]):
                    try:
                        random_events, repeated = split_random_stream(rest, explicit, actual_random, events[len(first) - 1]["time"])
                        break
                    except ValueError:
                        actual_random -= 1
                else:
                    raise ValueError("random submissions cannot be reconciled with recorded aborted work")
                repeats = [event["time"] for i, event in enumerate(repeated) if i % len(explicit) == 0]
                require(all(1024 <= e["dport"] <= 65534 for e in random_events), "random target outside exact old bounds")
                require(len({e["dport"] for e in random_events}) == len(random_events), "random call did not deduplicate ports")
                require(all(random_events[i + 1]["time"] - random_events[i]["time"] >= .012 for i in range(len(random_events) - 1)), "random per-IP 15ms interval missing")
                require(all(.16 <= repeats[i + 1] - repeats[i] <= .45 for i in range(len(repeats) - 1)), "explicit 200ms interval differs")
                random_count += len(random_events)
            else:
                require(not explicit or [target(e) for e in events] == explicit * (len(events) // len(explicit)), "explicit repeated sequence differs")
        if explicit and not behavior["PortsRandomNumber"]:
            # The old timer repeats a target-outer/socket-inner round. A large
            # round can stretch: per-socket gaps vary with its position within
            # that round, so check the actual round starts and global order.
            burst = [(dst, port) for dst in explicit for port in bindings.values()]
            require([(target(e), e["sport"]) for e in requests] == [burst[i % len(burst)] for i in range(len(requests))], "explicit target-outer/physical-socket-inner order differs")
            ticks = [requests[i]["time"] for i in range(0, len(requests), len(burst))]
            require(all(.16 <= ticks[i + 1] - ticks[i] <= .6 for i in range(len(ticks) - 1)), "global explicit 200ms timer pacing differs")
        if behavior["PortsRandomNumber"]:
            require(0 <= trace["random_iterations"] - trace["random_skipped"] - random_count <= trace["send_aborted"], "1000-iteration actual stream differs from submitted/aborted ledger")
        else:
            require(trace["random_iterations"] == trace["random_draws"] == trace["random_skipped"] == 0, "receiver performed unasked random work")
        allocator_records = allocations[node]
        mapped = {tuple(item["tuple"]): tuple(item["mapped"]) for item in allocator_records if "tuple" in item}
        exhausted = collections.Counter(bytes.fromhex(item["payload"]) for item in allocator_records if isinstance(item.get("exhausted"), list))
        outgoing = [e for e in wan if e["packet_type"] == 0 and raw(e) and e["src"] == PUBLIC[node]]
        wan_by_payload = collections.defaultdict(list)
        for event in outgoing:
            wan_by_payload[event["payload"]].append(event)
        for event in local:
            translated = wan_by_payload[event["payload"]]
            if not translated:
                require(exhausted[event["payload"]] > 0, "LAN packet absent from WAN without an explicit exhausted allocation")
                exhausted[event["payload"]] -= 1
                continue
            actual = translated.pop(0)
            key = (PRIVATE[node], event["sport"], event["dst"], event["dport"])
            require(key in mapped and (actual["src"], actual["sport"]) == mapped[key], "kernel SNAT differs from exact physical four-tuple allocation")
            require(actual["dst"] == event["dst"] and actual["dport"] == event["dport"] and actual["ttl"] == event["ttl"] - 1, "NAT target/hop changed")
            kind = cell["topology"].split("-")[0 if node == "a" else 1]
            base = 42031 if node == "a" else 43031
            if kind == "random":
                require(base <= actual["sport"] < base + 16, "bounded pool silently escaped through kernel collision rewrite")
            if kind == "wide":
                require(1024 <= actual["sport"] <= 65534, "full pool source outside old interval")
        require(not any(wan_by_payload.values()), "WAN contains unaccounted/fabricated traversal sends")
        low = [e for e in requests if e["ttl"] < 8]
        other_in = [e for e in lan[other] if raw(e) and e["dst"] == PRIVATE[other]]
        require(not any(e["payload"] == q["payload"] for e in other_in for q in low), "low TTL passed pre-conntrack loss")
        if cell["case"] == "blocked":
            require(not any(e["payload"] == q["payload"] for e in other_in for q in requests), "blocked WAN delivered request")
        quic = [e for e in lan[node] if e["src"] == PRIVATE[node] and e["sport"] in bindings.values() and e["payload"] and e["payload"][0] & 0x40]
        require(quic and all(e["ttl"] == 64 for e in quic), "per-packet TTL contaminated QUIC")
        stop = STOP.search(log)
        require(stop and stop[1] == round_id and stop[2] == stable and int(stop[3]) <= 38250, "phase deadline/cleanup identity differs")
        success = "DATA_OK" in log
        successes.append(success)
        if success:
            winner = WINNER.search(log)
            require(winner and winner[1] == round_id and winner[2] == stable, "winner identity absent")
            local_id, actual_local = int(winner[9]), address(winner[10])
            remote = address(winner[3])
            require(local_id in bindings and bindings[local_id] == actual_local[1] and actual_local[0] == PRIVATE[node], "winner actual local binding differs")
            winner_receipts = [e for e in incoming if e["dport"] == actual_local[1] and (e["src"], e["sport"]) == remote]
            require(winner_receipts, "winner source/physical port absent from LAN")
            if winner[4] == "true":
                require(any(e["payload"][27] == 1 and any(q["sport"] == actual_local[1] and nonce(q) == nonce(e) for q in requests) for e in winner_receipts), "response winner has no same-socket request nonce")
            else:
                require(any(e["payload"][27] == 0 for e in winner_receipts), "receiver winner has no actual request")
            business = BINDING.search(log)
            require(business and business[1] == stable and business[2] == peers[peer_side] and int(business[4]) == local_id and business[5] == winner[10] and business[6] == winner[3], "selected original business path differs from physical winner")
            # The corrected handoff generations must prove both physical
            # owners were ready without resetting the original38s deadline.
            if cell in cells()[12:]:
                ready = re.search(r"^LEGACY_WINNER_READY round=([0-9a-f]{32}) stable_id=(\d+) remaining_ms=(\d+)$", log, re.M)
                require(ready and ready[1] == round_id and ready[2] == stable and 0 < int(ready[3]) <= 38005 - int(stop[3]), "winner readiness reset deadline or changed round/connection")
                require(re.search(rf"^LEGACY_WINNER_PATH path_id_retries=\d+ stable_id={stable}$", log, re.M), "exact winner path was not validated by the corrected handoff")
            data_line = next(line for line in log.splitlines() if line.startswith("DATA_OK "))
            selected_path = re.search(r"selected: Some\(PathId\((\d+)\)\)", data_line)
            require(selected_path and selected_path[1] == business[3], "business path ID differs from selected path")
            # QUIC path validation and authorized business actually leave the
            # winning descriptor, rather than primary or a forwarding socket.
            require(any(e["sport"] == actual_local[1] and (e["dst"], e["dport"]) == remote for e in quic), "no QUIC on winner physical source/destination")
            nonprimary.append(local_id != 0)
            if side == "server":
                require("outcome=confirmed_business learned=true score_delta=2" in log, "original authorized business did not own feedback")
        else:
            require("PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed" in log and stop[4] == "failed" and int(stop[3]) >= 37950, "arbitrary or early failure is not a complete bounded miss")
            require(re.search(rf"^(CONTROL_ALIVE_AFTER_UDP_FAILURE|EARLY_FAILURE_CONTROL_ALIVE) stable_id={stable}$", log, re.M) and "CONTROL_LIVENESS_FAILED" not in log and "learned=true" not in log, "miss lost original control or incorrectly learned")
            if behavior["PortsRandomNumber"]:
                require(trace["random_iterations"] == 1000, "bounded miss did not complete old random iterations")
        costs[side] = dict(trace, elapsed_ms=int(stop[3]), local_sockets=len(bindings),
                           allocated_tuples=allocator_records[-1]["tuples"],
                           allocation_collisions=allocator_records[-1]["collisions"],
                           allocation_exhausted=allocator_records[-1]["exhausted"])
    require(len(set(rounds)) == 1 and successes[0] == successes[1], "peer round or business confirmation disagrees")
    if cell["case"] == "blocked":
        require(not any(successes), "blocked WAN gained authorized data")
    row.update(mode=plan[0], index=plan[1], round=rounds[0],
               outcome="confirmed_business" if all(successes) else "bounded_miss",
               nonprimary_business=any(nonprimary), costs=costs)
    return row


def main():
    p = argparse.ArgumentParser()
    p.add_argument("evidence", type=pathlib.Path)
    a = p.parse_args()
    repo = pathlib.Path(__file__).resolve().parents[1]
    expected = cells()
    require(json.loads((a.evidence / "verification/cells.json").read_text()) == expected, "changed or incomplete cell denominator")
    actual = {directory.name for directory in (a.evidence / "canonical").iterdir() if directory.is_dir()}
    require(actual == {name(cell) for cell in expected}, "missing/extra canonical cells")
    runners = json.loads((a.evidence / "verification/runner.json").read_text())
    require([row["cell"] for row in runners] == [name(cell) for cell in expected] and all(row["exit"] == 0 for row in runners), "failed or omitted fixture cells")
    generations = json.loads((a.evidence / "verification/runtime-by-cell.json").read_text())
    require([row["cell"] for row in generations] == [name(cell) for cell in expected], "runtime provenance omitted or reordered cells")
    for index, (cell, provenance) in enumerate(zip(expected, generations)):
        generation = 1 if index < 12 else 4 if index < 18 else 5
        require(provenance["runtime_generation"] == generation and provenance["runtime_manifest"] == f"runtime-generation{generation}-sha256.json", "unexpected runtime generation")
        fixture = a.evidence / "canonical" / name(cell) / "fixture.sh"
        require(hashlib.sha256(fixture.read_bytes()).hexdigest() == provenance["fixture_sha256"], "actual retained fixture differs from per-cell provenance")
        require((a.evidence / "verification" / provenance["source_manifest"]).is_file(), "missing source-generation manifest")
        manifest = json.loads((a.evidence / "verification" / provenance["runtime_manifest"]).read_text())
        require(set(manifest) == {"udp_nat_probe", "udp_observation_probe", "keys/client.key", "keys/server.key", "keys/secret.bin"}, "incomplete binary or fixed-pair provenance")
    repaired = [row for row in runners if row.get("verification_repaired")]
    require(len(repaired) == 1 and repaired[0]["cell"] == name(expected[18]) and repaired[0]["original_harness_exit"] == 1 and repaired[0]["reexecution"] is False, "post-validator repair obscured or reran an existing trial")
    for file, digest in json.loads((a.evidence / "verification/source-sha256.json").read_text()).items():
        require(hashlib.sha256((repo / file).read_bytes()).hexdigest() == digest, "frozen runtime source changed: " + file)
    oracle = json.loads((repo / "tests/fixtures/legacy_nat_behaviors.json").read_text())
    rows = [verify(a.evidence / "canonical" / name(cell), cell, oracle) for cell in expected]
    print(json.dumps(dict(result="PASS", gate_cases=len(rows),
                         confirmed_business=sum(row["outcome"] == "confirmed_business" for row in rows),
                         bounded_misses=sum(row["outcome"] == "bounded_miss" for row in rows),
                         admission_rejections=sum(row["outcome"] == "admission_rejection" for row in rows),
                         nonprimary_business=sum(row.get("nonprimary_business", False) for row in rows),
                         rows=rows), indent=2))


if __name__ == "__main__":
    main()
