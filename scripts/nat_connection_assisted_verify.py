#!/usr/bin/env python3
"""Independent packet validation of the two declared assisted-address corrections.

These blocked functional checks add no connection-probability samples. They
retain the earlier fixed comparison results under their actual runtime versions.
"""
import argparse
import json
import pathlib
import re

from nat_connection_verify import profile, sha
from legacy_nat_multi_socket_verify import DATAGRAM, LISTENER, STOP, TRACE, EvidencePath, address
from legacy_nat_scheduler_verify import BEHAVIOR, PRIVATE, PUBLIC, raw, target
from legacy_nat_verify import SAMPLE, require


def verify(base):
    v = base / "verification"
    plan_path = v / "assisted-functional-plan.json"
    plan = json.loads(plan_path.read_text())
    require(sha(plan_path) == (v / "assisted-functional-plan.sha256").read_text().strip(), "functional declaration changed")
    require(plan["probability_samples_added"] == 0 and len(plan["cells"]) == 2, "functional denominator changed")
    # The same immutable /keys mount and role identities were inherited from
    # the original frozen comparison plan, not selected from the new outcomes.
    pair = json.loads((v / "frozen-plan.json").read_text())["pair"]
    rows = json.loads((v / "assisted-functional-runner.json").read_text())
    require([r["cell"] for r in rows] == [c["id"] for c in plan["cells"]] and all(r["exit"] == 0 for r in rows), "functional runner missing/failed")
    results = []
    for cell in plan["cells"]:
        directory = EvidencePath(base / "assisted-functional" / cell["id"])
        require(json.loads((directory / "cell.json").read_text()) == cell, "functional inputs changed")
        wan, lan, counts, allocations = profile(directory, cell)
        result = dict(cell=cell["id"], generation=3, outcome="complete_bounded_miss", captures=counts, nodes={})
        round_ids = []
        for side in ["client", "server"]:
            log = (directory / f"{side}.log").read_text()
            node = cell["dialer"] if side == "client" else ("b" if cell["dialer"] == "a" else "a")
            other = "b" if node == "a" else "a"
            peer_side = "server" if side == "client" else "client"
            ids = set(re.findall(r"stable_id=(\d+)", log))
            require(len(ids) == 1, "original connection continuity changed")
            stable = next(iter(ids))
            require(f"NODE_SESSION local={pair[side]} peer={pair[peer_side]} stable_id={stable}" in log, "original TLS identities changed")
            require(f"EARLY_FAILURE_CONTROL_ALIVE stable_id={stable}" in log and "PATH_UNAVAILABLE" in log and "DATA_OK" not in log and "learned=true" not in log, "miss lost control/learned business")
            selected = re.findall(r"^LEGACY_SELECTED (.+)$", log, re.M)
            require(len(selected) == 1 and "execution=Ready" in selected[0], "basic fallback disabled or repeated recommendation")
            no_s = cell["no_observers"]
            expected_behaviors = [("Sender", "0", "0" if no_s else "200", "0", "0", "0"), ("Receiver", "0" if no_s else "7", "0", "0", "0", "0")]
            require(BEHAVIOR.findall(selected[0]) == expected_behaviors, "old default TTL/delay/role behavior changed")
            require(f"mode=0 index={6 if no_s else 0} " in selected[0] and f"fallback={str(no_s).lower()}" in selected[0], "fallback classification differs")
            candidates = re.findall(r"^LEGACY_CANDIDATES round=([0-9a-f]{32}) stable_id=(\d+) assisted=\[(.*?)\] mapped=\[(.*?)\]$", log, re.M)
            require(len(candidates) == 1 and candidates[0][1] == stable, "protected candidates missing")
            round_id, _, assisted, mapped = candidates[0]
            round_ids.append(round_id)
            require(f"round={round_id} " in selected[0], "candidate round differs")
            # Old assisted input is the peer's actual LAN interface and primary
            # port, independent of whether its reflected S candidates exist.
            peer_log = (directory / f"{peer_side}.log").read_text()
            peer_bindings = LISTENER.findall(peer_log)
            require(len(peer_bindings) == 1 and peer_bindings[0][2] == "0" and peer_bindings[0][4] == "true", "physical primary ownership changed")
            peer_port = address(peer_bindings[0][3])[1]
            assisted_expected = f"{PRIVATE[other]}:{peer_port}"
            require(assisted == assisted_expected, "actual local assisted candidate missing/wrong")
            samples = SAMPLE.findall(log)
            require(len(samples) == (0 if no_s else 4), "S resource absence/presence differs")
            if no_s:
                require(mapped == "" and "LEGACY_FEATURE feature=None" in log, "no-S fallback contains invented mapped observations")
            else:
                require([int(s[0]) for s in samples] == [0, 1, 2, 3], "ordered S observations differ")
                require(mapped.split(", ") == [f"{PUBLIC[other]}:{peer_port}"] * 4, "mapped candidate order/duplicates changed")
                for _, destination, observed in samples:
                    src, sport = address(observed)
                    dst, dport = address(destination)
                    require(any((e["src"], e["sport"], e["dst"], e["dport"]) == (src, sport, dst, dport) for e in wan), "real S mapped wire tuple missing")
            local = LISTENER.findall(log)
            require(len(local) == 1 and local[0][0:3] == (round_id, stable, "0") and local[0][4] == "true", "temporary socket/foreign round changed")
            local_port = address(local[0][3])[1]
            behavior = expected_behaviors[0 if side == "server" else 1]
            explicit = ([assisted_expected] + ([] if no_s else [f"{PUBLIC[other]}:{peer_port}"])) if side == "server" else ([] if no_s else [f"{PUBLIC[other]}:{peer_port}"])
            datagrams = DATAGRAM.findall(log)
            require(all(d[1:] == (behavior[1], "false", "false", "0") for d in datagrams), "probe TTL/response/socket changed")
            if explicit:
                require([d[0] for d in datagrams] == explicit * (len(datagrams) // len(explicit)), "assisted-before-mapped sequence changed")
            else:
                require(not datagrams, "receiver with no mapped candidates unexpectedly sent probes")
            outgoing = [e for e in lan[node] if raw(e) and e["src"] == PRIVATE[node] and e["payload"][11:27].hex() == round_id and e["payload"][27] == 0]
            require([target(e) for e in outgoing] == [d[0] for d in datagrams], "LAN wire assisted order differs from submissions")
            require(all(e["sport"] == local_port and e["ttl"] == (int(behavior[1]) or 64) for e in outgoing), "actual physical socket/default TTL differs")
            starts = re.findall(rf"^LEGACY_RECV_START round={round_id} .* phase=(.+)$", log, re.M)
            total = 5000 if no_s else 5200
            require(len(starts) == 1 and f"total_budget_ms: {total}, read_budget_ms: {total - int(behavior[2])}" in starts[0], "basic absolute deadline differs")
            stops = STOP.findall(log)
            require(len(stops) == 1 and stops[0][:2] == (round_id, stable) and stops[0][3] == "failed" and total - 15 <= int(stops[0][2]) <= total + 150, "early/unbounded phase stop")
            traces = TRACE.findall(log)
            require(len(traces) == 1, "execution ledger missing")
            trace = dict(item.split("=", 1) for item in traces[0][2].split())
            require(int(trace["probes"]) == len(outgoing) and trace["responses"] == "0" and all(trace[k] == "0" for k in ["listener_attempted", "listener_bound", "listener_failed", "random_iterations", "send_errors", "send_aborted"]), "partial/extra/leaked execution")
            require(not explicit or len(outgoing) >= len(explicit) * 20, "ordinary 200ms repeats missing")
            cpu = json.loads((directory / f"{side}-cpu.json").read_text())
            require(cpu["collector"] == "namespace-local wait4" and cpu["scope"] == "actual native node process" and cpu["executable_sha256"] == plan["runtime"]["udp_nat_probe"] and cpu["exit"] == 0 and not cpu["timed_out"], "wrong node accounting/runtime")
            require(cpu["user_cpu_seconds"] > 0 and cpu["system_cpu_seconds"] >= 0 and cpu["wall_seconds"] >= total / 1000 and cpu["exec_samples"] > 0 and cpu["sampled_exec_vm_hwm_kib"] > 0 and 0 < cpu["sampled_maximum_fd"] < 20, "node costs/cleanup bound missing")
            product = [e for e in lan[node] if e["src"] == PRIVATE[node] and not e["payload"].startswith(b"PREFLIGHT")]
            qad = [e for e in product if e["dst"] == "100.64.27.1" and 46761 <= e["dport"] <= 46764]
            require(not qad if no_s else bool(qad), "actual no-S resource absence differs")
            result["nodes"][side] = dict(cpu=cpu, assisted=assisted, mapped=mapped, probes=len(outgoing), phase_ms=int(stops[0][2]), udp_datagrams=len(product), udp_payload_bytes=sum(len(e["payload"]) for e in product), qad_datagrams=len(qad))
        require(len(set(round_ids)) == 1, "protected peer rounds disagree")
        results.append(result)
    return dict(result="PASS", probability_samples_added=0, final_generation=3, functional_checked=2, cells=results)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    args = parser.parse_args()
    result = verify(args.evidence)
    (args.evidence / "verification/assisted-functional-strict.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "cells"}))


if __name__ == "__main__":
    main()
