#!/usr/bin/env python3
"""Check the MIOPUNCH-156 host witness and the no-work cooldown contract."""

import argparse
import json
from pathlib import Path


COOLDOWN_MS = 8_000


def require(condition, message):
    if not condition:
        raise ValueError(message)


def select(rows, phase, result=None, resource=None):
    return [
        row for row in rows
        if row["phase"] == phase
        and (result is None or row["result"] == result)
        and (resource is None or row["detail"].get("resource") == resource)
    ]


def one(rows):
    require(len(rows) == 1, f"expected one event, got {rows}")
    return rows[0]


def read_log(path, phases):
    markers = {}
    windows = {}
    timeline = []
    finished = False
    for line in path.read_text().splitlines():
        if line.startswith("test result: ok. 1 passed;"):
            finished = True
        if not line.startswith("{"):
            continue
        row = json.loads(line)
        if row["event"] == "connection-timeline":
            timeline.append(row)
            continue
        if row["event"] == "contention-fixture":
            phase = row["phase"]
            require(phase not in markers, f"duplicate fixture marker {phase}")
            markers[phase] = row
            windows[phase] = len(timeline)
    require(finished and tuple(markers) == phases, f"incomplete fixture run in {path}")
    start = phases[0]
    for end in phases[1:]:
        before, after = markers[start], markers[end]
        require(before["originals"] == after["originals"], "the original connection changed")
        require(all(a < b for a, b in zip(before["businessSequences"], after["businessSequences"], strict=True)),
                f"original bidirectional traffic stalled between {start} and {end}")
        start = end
    intervals = {
        end: timeline[windows[start]:windows[end]]
        for start, end in zip(phases, phases[1:])
    }
    return markers, intervals, timeline


def reservations(rows):
    return select(rows, "probe-budget", "reserved", "enhanced_round")


def check_paired_round(rows, originals):
    held = reservations(rows)
    require(len(held) == 2 and {row["stableId"] for row in held} == set(originals),
            "one acquired enhanced permit per side is required")
    wire_id = held[0]["attemptId"]
    require(wire_id and all(row["attemptId"] == wire_id for row in held),
            "the two permits must belong to the same protected attempt")
    sides = {}
    for row in held:
        stable = row["stableId"]
        own = [r for r in rows if r["localAttemptId"] == row["localAttemptId"]]
        sampled = one(select(own, "observation", "succeeded"))
        packet_budget = one(select(own, "probe-budget", "reserved", "punch_datagrams"))
        first_send = one(select(own, "punch-first-send", "sent"))
        received = one(select(own, "punch-first-receipt", "matched"))
        packets = one(select(own, "punch-packets", "finished"))
        observations = sampled["detail"]["output"]["stunSamples"]
        actual = packets["detail"]["probes"] + packets["detail"]["responses"]
        limit = packet_budget["detail"]["requested"]
        require(row["detail"]["observationReserved"] == observations == 2,
                "the paired round must collect its two actual observations per side")
        require(0 < actual <= limit <= row["detail"]["peerLimit"], "peer datagram allowance exceeded or unused")
        require(row["monotonicMs"] <= sampled["monotonicMs"] <= first_send["monotonicMs"]
                <= packets["monotonicMs"], "expected observation before actual packet submission")
        require(received["monotonicMs"] <= packets["monotonicMs"], "matched receipt fell outside the punch")
        sides[stable] = {
            "localAttempt": row["localAttemptId"], "cooldownStartMs": row["monotonicMs"],
            "cooldownMs": row["detail"]["nextDueInMs"], "samples": observations,
            "firstSendMs": first_send["monotonicMs"], "firstMatchedMs": received["monotonicMs"],
            "packets": actual, "packetLimit": limit, "round": packets["detail"]["round"],
        }
    require(len({side["round"] for side in sides.values()}) == 1, "mismatched accepted rounds")
    return {"attempt": wire_id, "sides": sides}


def same_original(path):
    phases = ("crossed-start", "crossed-end")
    markers, intervals, _ = read_log(path, phases)
    start, end = (markers[phase] for phase in phases)
    (left, right), = start["originals"]
    rows = intervals[phases[-1]]
    outgoing = [r for r in rows if r["source"] == "maintenance_improvement"]
    starts = select(outgoing, "hello", "started")
    require(len(starts) == 2 and {r["stableId"] for r in starts} == {left, right},
            "both maintained sides must send Hello on the same original")
    yielded = one(select(outgoing, "hello", "cancelled"))
    handoff = one(select(rows, "hello-arbitration", "handed-off"))
    require(yielded["stableId"] == handoff["stableId"] == right,
            "the same-original acceptor must release its outgoing Hello")
    loser = [r for r in rows if r["localAttemptId"] == yielded["localAttemptId"]]
    require(not (reservations(loser) or select(loser, "observation") or select(loser, "punch-first-send")),
            "the superseded Hello consumed enhanced work")
    work = check_paired_round(rows, [left, right])
    require(work["sides"][right]["localAttempt"] == handoff["localAttemptId"],
            "the handed-off responder must own its subsequent work")
    require(start["stunRequests"] == [0, 0] and end["stunRequests"] == [2, 2], "STUN witness differs")
    return {
        "original": [left, right], "yieldedLocalAttempt": yielded["localAttemptId"],
        "yieldedPackets": 0, "yieldedSamples": 0, "handoffMs": handoff["monotonicMs"],
        "businessSequences": end["businessSequences"], "work": work,
    }


def cooldowns(previous, retry, originals):
    by_original = {}
    for row in select(retry, "probe-budget", "deferred", "enhanced_round"):
        stable = row["stableId"]
        require(stable in originals and stable not in by_original, "retry must use the same pair")
        detail = row["detail"]
        require(not detail["occupied"], "still occupied is not a released permit's cooldown")
        acquired = previous[stable]
        elapsed_ms = row["monotonicMs"] - acquired["cooldownStartMs"]
        remaining_ms = detail["nextDueInMs"]
        require(0 < remaining_ms < COOLDOWN_MS
                and abs(elapsed_ms + remaining_ms - acquired["cooldownMs"]) <= 3,
                "unrelated defer cannot stand in for the previous round's clock")
        by_original[stable] = remaining_ms
    return by_original


def check_unspent_handoff(early_timeline, unspent_cooldowns):
    """Shared acceptance for the same crossed operation on the fused entry."""
    sampled = {
        row["attemptId"] for row in select(early_timeline, "observation", "succeeded")
        if row["attemptId"] and row["detail"]["output"]["samples"] > 0
    }
    sent = {
        row["attemptId"] for row in select(early_timeline, "punch-first-send", "sent")
        if row["attemptId"]
    }
    failures = []
    if not sampled & sent:
        failures.append("neither crossed attempt nor the early follow-up reached observation and a real send")
    if unspent_cooldowns:
        failures.append("released zero-work permits held a full failure cooldown")
    return failures


def independent(path):
    phases = ("crossed-start", "crossed-end", "early-retry-end", "probe-end", "probe-retry-end")
    markers, rows, timeline = read_log(path, phases)
    first, second = markers[phases[0]]["originals"]
    require(len(set(first + second)) == 4, "two distinct baseline originals are required")
    crossed = rows["crossed-end"]
    winners = {}
    for original in (first, second):
        round_events = [r for r in crossed if r["stableId"] in original]
        held = one(reservations(round_events))
        blocked = one(select(round_events, "probe-budget", "deferred", "enhanced_round"))
        require(held["attemptId"] and held["attemptId"] == blocked["attemptId"]
                and held["stableId"] != blocked["stableId"], "independent attempt did not reach both sides")
        require(blocked["detail"]["occupied"], "defer must witness active contention")
        old = [r for r in timeline if r["localAttemptId"] == held["localAttemptId"]]
        one(select(old, "punch-round-agreement", "failed"))
        winners[held["stableId"]] = {
            "attempt": held["attemptId"], "control": held["controlStableId"],
            "localAttempt": held["localAttemptId"], "cooldownStartMs": held["monotonicMs"],
            "cooldownMs": held["detail"]["nextDueInMs"], "blockedStableId": blocked["stableId"],
            "blockedAtMs": blocked["monotonicMs"], "samples": 0, "packets": 0,
        }
    left = next(side for side in winners if side in (first[0], second[0]))
    right = next(side for side in winners if side in (first[1], second[1]))
    require(winners[left]["attempt"] != winners[right]["attempt"], "same wire attempt cannot fill both nodes")
    require(max(winners[side]["cooldownStartMs"] for side in (left, right)) <
            min(winners[side]["blockedAtMs"] for side in (left, right)), "reservations did not overlap")
    require(all(winners[side]["cooldownMs"] == COOLDOWN_MS for side in (left, right)),
            "both nodes must observe the old full cooldown begin")
    early = rows["early-retry-end"]
    no_work = crossed + early
    require(not (select(no_work, "observation") or select(no_work, "punch-first-send")
                 or select(no_work, "punch-packets")), "work occurred during the zero-work collision")
    require(all(markers[phase]["stunRequests"] == [0, 0] for phase in phases[:3]),
            "the two actual STUN observers saw traffic during the dry rounds")
    by_retry_original = {first[0]: winners[left], first[1]: winners[right]}
    dry_cooldowns = cooldowns(by_retry_original, early, first)
    require(set(dry_cooldowns) == set(first), "both zero-work cooldowns must be independently witnessed")
    require(len(markers["crossed-end"]["outcomes"]) == 2 and
            all("local enhanced-round budget deferred" in cause for cause in markers["crossed-end"]["outcomes"]),
            "both crossed repairs must return their real budget deferral")

    work = check_paired_round(rows["probe-end"], first)
    deadline = one(select(rows["probe-end"], "repair", "started"))
    settled = one(select(rows["probe-end"], "repair", "failed"))
    require(2_990 <= deadline["detail"]["input"]["budgetMs"] <= 3_000
            and settled["durationMs"] <= 3_100
            and settled["detail"]["output"]["originalOpen"], "the original three-second repair envelope changed")
    require(all(side["firstSendMs"] - deadline["monotonicMs"] <= 3_000 for side in work["sides"].values()),
            "a paired probe began sending after the original deadline")
    require(all(work["sides"][stable]["cooldownStartMs"] >= previous["cooldownStartMs"] + COOLDOWN_MS
                for stable, previous in by_retry_original.items()), "the control must run after old cooldown")
    spent_cooldowns = cooldowns(work["sides"], rows["probe-retry-end"], first)
    require(set(spent_cooldowns) == set(first), "worked attempts must still count against both local budgets")
    require(all(markers[phase]["stunRequests"] == [2, 2] for phase in phases[3:]),
            "only the substantive round should reach the STUN observers")
    return {
        "originals": [first, second], "crossed": winners, "dryCooldownRemainingMs": dry_cooldowns,
        "work": work, "workDeadlineMs": deadline["detail"]["input"]["budgetMs"],
        "workSettledMs": settled["durationMs"], "workedCooldownRemainingMs": spent_cooldowns,
        "businessSequences": {phase: markers[phase]["businessSequences"] for phase in phases},
        "repairOutcomes": {phase: markers[phase]["outcomes"] for phase in phases[1:]},
        "acceptanceFailures": check_unspent_handoff(no_work, dry_cooldowns),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("same_original", type=Path)
    parser.add_argument("independent", type=Path)
    parser.add_argument("--expect-baseline-red", action="store_true",
                        help="succeed only after reproducing the legacy red and its working control")
    args = parser.parse_args()
    try:
        result = {"sameOriginal": same_original(args.same_original), "independent": independent(args.independent)}
    except (ValueError, KeyError) as error:
        parser.exit(2, f"incomplete witness: {error}\n")
    failures = result["independent"]["acceptanceFailures"]
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return int(bool(failures) != args.expect_baseline_red)


if __name__ == "__main__":
    raise SystemExit(main())
