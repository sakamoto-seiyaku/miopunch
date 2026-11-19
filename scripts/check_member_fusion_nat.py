#!/usr/bin/env python3
"""Run native member streams in the retained two-gateway NAT lab.

Invoke in a disposable mount/user/network namespace with an empty /run/netns.
Pass the already built resources test executable and a new evidence directory.
"""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import subprocess
import time


REPO = Path(__file__).resolve().parents[1]
TEST = "member_fusion::udp::nat_lab::enhanced_udp_through_translated_members"
RESTRICTED_TEST = "native_service_nat::restricted_contact_delivers_the_first_native_reply_through_enhanced_udp"
WAN = "100.64.27.1"
PUBLIC = ("100.64.27.2", "100.64.27.6")
PRIVATE = ("10.27.1.2", "10.27.2.2")
PRIMARY = (42027, 43027)
OBSERVERS = (46761, 46762)
RANDOM_PROFILES = ((42040, 42080), (43028, 43030))


def run(*args):
    return subprocess.run(args, check=True, text=True, stdout=subprocess.PIPE).stdout


def topology(names):
    source = (REPO / "scripts/nat4_irregular_lab.sh").read_text()
    # Keep the same WAN, gateways, private interfaces, routes and baseline firewall.
    body = source[source.index('for ns in "${names[@]}"; do\n  ip netns add'):
                  source.index("# Each gateway independently maps")]
    run("bash", "-ec", 'names=("$@"); wan=$1; nat_a=$2; nat_b=$3; node_a=$4; node_b=$5; case_name=fusion\n' + body,
        "member-fusion-nat", *names)


def rules(names, case, visitor, primary=PRIMARY):
    wan, *gateways, _, _ = names
    random = case.startswith("random")
    blocked = case.endswith("blocked") or case == "random-cancel"
    # For this role table the random phase always has B sending to A's listeners.
    sender = 1 if random else 1 - visitor
    receiver = 1 - sender
    for side, gateway in enumerate(gateways):
        def iptables(*args):
            return run("ip", "netns", "exec", gateway, "iptables", *args)

        iptables("-I", "FORWARD", "1", "-i", "wan0", "-o", "lan0", "-p", "udp",
                 "-d", PRIVATE[side], "-j", "ACCEPT")
        iptables("-t", "raw", "-A", "PREROUTING", "-i", "wan0", "-p", "udp",
                 "-m", "ttl", "--ttl-lt", "8", "-j", "DROP")
        iptables("-t", "nat", "-A", "PREROUTING", "-i", "wan0", "-p", "udp",
                 "-d", PUBLIC[side], "-j", "DNAT", "--to-destination", PRIVATE[side])
        iptables("-t", "nat", "-I", "POSTROUTING", "1", "-o", "wan0", "-p", "udp",
                 "-s", PRIVATE[side], "-j", "SNAT", "--to-source", PUBLIC[side])
        if random:
            for destination, mapped in zip(OBSERVERS, RANDOM_PROFILES[side]):
                iptables("-t", "nat", "-I", "POSTROUTING", "1", "-o", "wan0", "-p", "udp",
                         "-s", PRIVATE[side], "--sport", str(PRIMARY[side]), "-d", WAN,
                         "--dport", str(destination), "-j", "SNAT",
                         "--to-source", f"{PUBLIC[side]}:{mapped}")
        elif side == sender:
            # Preserve the real reply tuple through conntrack, with a peer-only port shift
            # on the sender so one end must use the received source instead of the observations.
            iptables("-t", "nat", "-I", "POSTROUTING", "1", "-o", "wan0", "-p", "udp",
                         "-s", PRIVATE[side], "--sport", str(primary[side]), "-d", PUBLIC[receiver],
                         "-j", "SNAT", "--to-source", f"{PUBLIC[side]}:{primary[side] + 10}")

    def wan_rule(side, *args):
        return run("ip", "netns", "exec", wan, "iptables", "-A", "FORWARD", "-p", "udp",
                   "-s", PUBLIC[side], "-d", PUBLIC[1 - side], *args)

    if case.startswith("restricted-"):
        # The lab permits regular peer QUIC only after seeing the separate
        # traversal protocol in that direction. STUN and MQTT are unaffected.
        for side in (0, 1):
            if not blocked:
                marker = ("-m", "recent", "--name", f"m155-{side}")
                wan_rule(side, "-m", "string", "--algo", "bm", "--from", "28", "--to", "39",
                         "--hex-string", "|004d494f50554e43483633|", *marker, "--set", "-j", "ACCEPT")
                wan_rule(side, *marker, "--rcheck", "-j", "ACCEPT")
            wan_rule(side, "-j", "DROP")
    elif case == "validation-blocked":
        # Allow the 76-byte authenticated UDP message (104 including UDP/IPv4).
        # The same one-way fault then blocks padded QUIC validation on the return.
        wan_rule(receiver, "-m", "length", "!", "--length", "104", "-j", "DROP")
    elif blocked:
        wan_rule(receiver, "-j", "DROP")
    if random:
        for port in (PRIMARY[receiver], *RANDOM_PROFILES[receiver]):
            wan_rule(sender, "--dport", str(port), "-j", "DROP")
    for side in (0, 1):
        wan_rule(side, "-j", "ACCEPT")


def worker(stack, names, binary, directory, case, visitor, role):
    index = {"setup": 0, "a": 3, "b": 4}[role]
    log = stack.enter_context((directory / f"{role}.log").open("w"))
    process = subprocess.Popen(
        ["ip", "netns", "exec", names[index], "env", f"MIO150_WORKER={role}",
         f"MIO150_ROOT={directory / 'members'}", f"MIO150_CASE={case}",
         f"MIO150_VISITOR={'ab'[visitor]}", str(binary),
         RESTRICTED_TEST if case.startswith("restricted-") else TEST, "--exact", "--ignored", "--nocapture"],
        stdout=log, stderr=subprocess.STDOUT)
    stack.callback(stop, process)
    return process


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def read_result(log):
    lines = [line[len("MIO150 RESULT "):] for line in log.read_text().splitlines()
             if line.startswith("MIO150 RESULT ")]
    assert len(lines) == 1, log
    return json.loads(lines[0])


def restricted_result(directory, case, visitor, trial, stats):
    assert "MIO155 MQTT_INITIALS 1" in (directory / "setup.log").read_text()
    success = not case.endswith("blocked")
    ends = []
    for role, side in zip("ab", range(2)):
        log = (directory / f"{role}.log").read_text().splitlines()
        events = [json.loads(line) for line in log if line.startswith('{"attemptId":')]
        admission, = [event for event in events if event["phase"] == "member-admission"
                      and event["result"] == "succeeded"]
        if side == visitor:
            identity, = [event for event in events if event["phase"] == "quic-identity"]
            assert identity["stableId"] == admission["stableId"]
            reply, = [line for line in log if line.startswith("MIO155 FIRST_REPLY ")]
            assert f"success={str(success).lower()}" in reply
            elapsed = int(reply.rsplit("=", 1)[1])
        label = f"MIO155 {'CONSUMER' if side == visitor else 'PROVIDER'}_ORIGINAL "
        streams = [json.loads(line[len(label):]) for line in log if line.startswith(label)]
        assert len(streams) == int(success)
        if success:
            stream, = streams
            assert stream["connection"] == admission["stableId"]
            ends.append({"side": role, "attempt": admission["attemptId"], "transport": stream})
    if success:
        a, b = ends
        assert a["attempt"] == b["attempt"]
        assert a["transport"]["stream"] == b["transport"]["stream"]
        assert a["transport"]["enhanced_udp"]["round"] == b["transport"]["enhanced_udp"]["round"]
        # Each directions' WAN rule saw the separate raw protocol and allowed
        # follow-up QUIC on the learned mapping. A drop alone proves neither.
        for side in (0, 1):
            marker = [line.split() for line in stats.splitlines() if f"name: m155-{side}" in line]
            assert len(marker) == 2 and all(int(rule[0]) > 0 for rule in marker), stats
    else:
        assert any(rule[2] == "DROP" and int(rule[0]) > 0
                   for line in stats.splitlines() if (rule := line.split()) and rule[0].isdigit()), stats
    result = {"case": case, "visit": "ab"[visitor], "trial": trial,
              "success": success, "elapsed_ms": elapsed, "ends": ends}
    (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def cell(binary, output, case, visitor, trial):
    directory = output / f"{case}-{'ab'[visitor]}-{trial}"
    directory.mkdir()
    names = [f"m150-{role}" for role in ("wan", "na", "nb", "a", "b")]
    try:
        topology(names)
        restricted = case.startswith("restricted-")
        if not restricted:
            rules(names, case, visitor)
        with ExitStack() as stack:
            setup = worker(stack, names, binary, directory, case, visitor, "setup")
            ready = directory / "setup.log"
            deadline = time.monotonic() + 35
            while "MIO150 LAB_READY" not in ready.read_text():
                assert setup.poll() is None and time.monotonic() < deadline, ready
                time.sleep(0.05)
            processes = [worker(stack, names, binary, directory, case, visitor, role) for role in "ba"]
            deadline = time.monotonic() + 72
            if restricted:
                members = directory / "members"
                ports = [members / f"port-{role}" for role in "ab"]
                while any(not port.exists() or not port.stat().st_size for port in ports):
                    assert all(process.poll() is None for process in processes) and time.monotonic() < deadline
                    time.sleep(0.02)
                rules(names, case, visitor, tuple(int(port.read_text()) for port in ports))
                (members / "start").touch()
            exits = [process.wait(timeout=max(1, deadline - time.monotonic())) for process in processes]
            assert exits == [0, 0], (directory, exits)
        stats = run("ip", "netns", "exec", names[0], "iptables", "-nvxL", "FORWARD")
        (directory / "wan-forward.txt").write_text(stats)
        for role, name in zip("ab", names[1:3]):
            (directory / f"gateway-{role}.txt").write_text(
                run("ip", "netns", "exec", name, "iptables", "-t", "nat", "-nvxL"))
        if restricted:
            return restricted_result(directory, case, visitor, trial, stats)
        a, b = (read_result(directory / f"{role}.log") for role in "ab")
        for field in ("round", "attempt", "stream", "mode", "index", "success"):
            assert a[field] == b[field], (field, a, b)
        assert a["frp_role"] != b["frp_role"]
        assert "Sender" in (a["frp_role"], b["frp_role"])
        for side, evidence in enumerate((a, b)):
            assert evidence["visitor"] == (side == visitor)
            assert len(evidence["samples"]) == 2
            assert evidence["probes"] + evidence["responses"] <= evidence["packet_limit"]
            assert evidence["released"] + evidence["retained"] == evidence["listener_bound"]
        if case.endswith("blocked"):
            assert not a["success"] and not b["success"]
            assert a["relay_after"] and b["relay_after"]
            assert any(line.split()[0].isdigit() and int(line.split()[0]) > 0 and
                       line.split()[2] == "DROP" for line in stats.splitlines() if line.strip())
        if case == "validation-blocked":
            assert a["match"] and b["match"] and a["winner"] and b["winner"]
            assert a["ack_ms"] is None and b["ack_ms"] is None
        if case in ("fixed", "fixed-auto"):
            assert a["success"]
            sender = 1 - visitor
            receiver = (a, b)[visitor]
            assert receiver["match"]["remote"] == f"{PUBLIC[sender]}:{PRIMARY[sender] + 10}"
            assert receiver["winner"]["remote"] == receiver["match"]["remote"]
            assert receiver["winner"]["remote"] not in (a, b)[sender]["samples"]
        if case == "fixed-auto":
            assert a["automatic_business_verified"] and b["automatic_business_verified"]
            assert not a["learned"] and not b["learned"]
        if case.startswith("random"):
            assert a["mode"] == 4 and a["listener_attempted"] == 256
            assert a["retained"] == int(a["success"])
        if case == "random-cancel":
            assert not a["success"] and a["relay_after"] and b["relay_after"]
            assert a["released"] == 256
            assert "MIO150 SLOT_REUSED" in (directory / f"{'ab'[visitor]}.log").read_text()
        result = {"case": case, "visit": "ab"[visitor], "trial": trial, "ends": [a, b]}
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        return result
    finally:
        for name in names:
            subprocess.run(["ip", "netns", "del", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_binary", type=Path)
    parser.add_argument("evidence", type=Path)
    cases = ("fixed", "fixed-blocked", "validation-blocked", "random", "random-blocked", "random-cancel")
    parser.add_argument("--case", action="append", choices=(*cases, "fixed-auto", "restricted-fixed", "restricted-fixed-blocked"))
    parser.add_argument("--random-trials", type=int, default=2)
    args = parser.parse_args()
    binary = args.test_binary.resolve(strict=True)
    output = args.evidence.resolve()
    output.mkdir()
    cases = args.case or cases
    assert args.random_trials > 0
    summary = []
    for case in cases:
        for visitor in (0, 1):
            for trial in range(1, (args.random_trials if case == "random" else 1) + 1):
                result = cell(binary, output, case, visitor, trial)
                summary.append(result)
                (output / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
                success = result["success"] if "success" in result else result["ends"][0]["success"]
                print(f"{case} visitor={result['visit']} trial={trial} success={success}", flush=True)


if __name__ == "__main__":
    main()
