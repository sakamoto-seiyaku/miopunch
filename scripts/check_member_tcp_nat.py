#!/usr/bin/env python3
"""Exercise NativeClient TCP first contact through two real NAT gateways.

Run in a disposable mount/user/network namespace with an empty /run/netns.
Pass an already built native_service_streams test executable.
"""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import subprocess
import time

from check_member_fusion_nat import topology, stop, run, PUBLIC, restricted_stream_results

TEST = "native_service_tcp_nat::restricted_contact_delivers_the_first_native_reply_through_tcp"
REPO = Path(__file__).resolve().parents[1]


def ipv6_topology(names):
    wan, gateway_a, gateway_b, node_a, node_b = names
    for namespace, interface, address in [
        (wan, "lo", "fd27:0::1/128"),
        (wan, "wan-a", "fd27:a::1/64"), (gateway_a, "wan0", "fd27:a::2/64"),
        (wan, "wan-b", "fd27:b::1/64"), (gateway_b, "wan0", "fd27:b::2/64"),
        (gateway_a, "lan0", "fd27:1::1/64"), (node_a, "eth0", "fd27:1::2/64"),
        (gateway_b, "lan0", "fd27:2::1/64"), (node_b, "eth0", "fd27:2::2/64"),
    ]:
        run("ip", "-n", namespace, "-6", "addr", "add", address, "dev", interface, "nodad")
    for namespace, destination, gateway in [
        (wan, "fd27:1::/64", "fd27:a::2"), (wan, "fd27:2::/64", "fd27:b::2"),
        (gateway_a, "default", "fd27:a::1"), (gateway_b, "default", "fd27:b::1"),
        (node_a, "default", "fd27:1::1"), (node_b, "default", "fd27:2::1"),
    ]:
        run("ip", "-n", namespace, "-6", "route", "add", destination, "via", gateway)
    for namespace in names[:3]:
        run("ip", "netns", "exec", namespace, "sysctl", "-qw", "net.ipv6.conf.all.forwarding=1")
    for gateway in (gateway_a, gateway_b):
        run("ip", "netns", "exec", gateway, "ip6tables", "-P", "FORWARD", "DROP")
        run("ip", "netns", "exec", gateway, "ip6tables", "-A", "FORWARD", "-i", "lan0", "-o", "wan0", "-j", "ACCEPT")
        run("ip", "netns", "exec", gateway, "ip6tables", "-A", "FORWARD", "-i", "wan0", "-o", "lan0",
            "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT")


def cell(binary, output, case, visitor, ipv6=False):
    directory = output / f"{case}-{'ab'[visitor]}"
    directory.mkdir()
    names = [f"m159-{role}" for role in ("wan", "na", "nb", "a", "b")]
    try:
        topology(names)
        if ipv6:
            ipv6_topology(names)
        public = ("fd27:1::2", "fd27:2::2") if ipv6 else PUBLIC
        firewall = "ip6tables" if ipv6 else "iptables"
        run("ip", "netns", "exec", names[0], "sysctl", "-qw", "net.ipv4.ip_forward=1")
        for gateway in names[1:3]:
            # Leave forwarding governed by real conntrack. No static DNAT.
            run("ip", "netns", "exec", gateway, "iptables", "-A", "INPUT",
                "-i", "wan0", "-p", "tcp", "--syn", "-j", "DROP")
        for side in (0, 1):
            run("ip", "netns", "exec", names[0], firewall, "-A", "FORWARD",
                "-p", "udp", "-s", public[side], "-d", public[1-side], "-j", "DROP")
            run("ip", "netns", "exec", names[0], firewall, "-A", "FORWARD",
                "-p", "tcp", "-s", public[side], "-d", public[1-side], "-j",
                "DROP" if case == "blocked" else "ACCEPT")
        with ExitStack() as stack:
            def process(role, command):
                log = stack.enter_context((directory / f"{role}.log").open("w"))
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                stack.callback(stop, proc)
                return proc

            observer = process("observer", ["ip", "netns", "exec", names[0], "python3",
                                           "-u", str(REPO / "scripts/tcp_stun_observer.py"), "--host", "fd27:0::1" if ipv6 else "100.64.27.1"])

            def worker(role):
                index = {"setup": 0, "a": 3, "b": 4}[role]
                return process(role, ["ip", "netns", "exec", names[index], "env",
                    f"MIO150_WORKER={role}", f"MIO150_ROOT={directory / 'members'}",
                    f"MIO150_CASE={case}", f"MIO150_VISITOR={'ab'[visitor]}", *(["MIO203_TCP_IPV6=1"] if ipv6 else []),
                    str(binary), TEST, "--exact", "--ignored", "--nocapture"])

            setup = worker("setup")
            deadline = time.monotonic() + 35
            while "MIO150 LAB_READY" not in (directory / "setup.log").read_text():
                assert setup.poll() is None and observer.poll() is None and time.monotonic() < deadline, directory
                time.sleep(0.05)
            workers = [worker("b")]
            if case == "simultaneous":
                deadline = time.monotonic() + 15
                while not (directory / "members" / "contact-b").exists():
                    assert workers[0].poll() is None and setup.poll() is None and time.monotonic() < deadline, directory
                    time.sleep(0.02)
            workers.append(worker("a"))
            ports = [directory / "members" / f"port-{role}" for role in "ab"]
            deadline = time.monotonic() + 35
            while any(not port.exists() or not port.stat().st_size for port in ports):
                assert all(p.poll() is None for p in workers) and time.monotonic() < deadline, directory
                time.sleep(0.02)
            if case in ("positive", "blocked"):
                for side in (0, 1):
                    target = public[1 - side]
                    port = int(ports[1 - side].read_text())
                    probe = "import socket; s=socket.socket(socket.AF_INET6 if ':' in %r else socket.AF_INET); s.settimeout(0.3); result=s.connect_ex((%r,%d)); assert result != 0, result" % (target, target, port)
                    run("ip", "netns", "exec", names[3 + side], "python3", "-c", probe)
                print("direct inbound negative control passed in both directions", flush=True)
            (directory / "members" / "start").touch()
            exits = [p.wait(timeout=max(1, deadline - time.monotonic())) for p in workers]
            assert exits == [0, 0], (directory, exits)
            initials, = [line for line in (directory / "setup.log").read_text().splitlines()
                         if line.startswith("MIO155 MQTT_INITIALS ")]
            dials = int(initials.rsplit(" ", 1)[1])
            if case == "simultaneous":
                assert 1 <= dials <= 2
        for name, label in zip(names, ("wan", "gateway-a", "gateway-b", "a", "b")):
            (directory / f"{label}-rules.txt").write_text(
                run("ip", "netns", "exec", name, "ip6tables-save" if ipv6 else "iptables-save", "-c"))
            (directory / f"{label}-sockets.txt").write_text(
                run("ip", "netns", "exec", name, "ss", "-tan"))
        if case == "simultaneous":
            ends = []
            for role in "ab":
                prefix = "MIO159 MUTUAL_ORIGINAL "
                records = [json.loads(line[len(prefix):])
                           for line in (directory / f"{role}.log").read_text().splitlines()
                           if line.startswith(prefix)]
                transports, = records
                assert len(transports) == 2
                ends.append(transports)
            assert sorted(t["stream"] for t in ends[0]) == sorted(t["stream"] for t in ends[1])
            result = {"case": case, "success": True, "ends": ends}
            (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            return result
        ends, elapsed = restricted_stream_results(directory, visitor, case == "positive")
        if case == "positive":
            assert dials == 1
        else:
            controls = []
            members = []
            attempts = []
            for role in "ab":
                lines = (directory / f"{role}.log").read_text().splitlines()
                controls.append([json.loads(line.removeprefix("NATIVE_CONTROL_IDENTITY "))
                                 for line in lines if line.startswith("NATIVE_CONTROL_IDENTITY ")])
                starts = [json.loads(line) for line in lines if line.startswith('{"attemptId":')]
                member, = [event for event in starts if event["phase"] == "member-admission" and event["result"] == "started"]
                members.append(member["detail"]["input"]["peer"])
                attempts.append(member["attemptId"])
            assert attempts[0] and attempts[0] == attempts[1]
            management = [record for record in controls[visitor]
                          if record["alpn"] == "miopunch/device-removal/1" and record["side"] == "Client"]
            peer_controls = [record for record in controls[1 - visitor]
                        if record["alpn"] == "miopunch/device-removal/1" and record["side"] == "Server"]
            assert len(management) == len(peer_controls)
            assert all(record["local"] == members[1 - visitor] and record["peer"] == members[visitor]
                       for record in management)
            assert all(record["local"] == members[visitor] and record["peer"] == members[1 - visitor]
                       for record in peer_controls)
            assert dials == 1 + len(management), (dials, management, peer_controls)
        for side, role in enumerate("ab"):
            lines = (directory / f"{role}.log").read_text().splitlines()
            observed = [line for line in lines if line.startswith("TCP_OBSERVED")]
            punched = any("TCP_PUNCH_START" in line for line in lines)
            if case in ("disabled", "observer-denied"):
                assert not observed and not punched, lines
            else:
                assert len(observed) == 1
                assert f"listener={'[::]' if ipv6 else '0.0.0.0'}:{ports[side].read_text()}" in observed[0]
                assert f"candidate={'[' + public[side] + ']' if ipv6 else public[side]}:" in observed[0]
                assert punched == (case != "target-denied")
        observations = (directory / "observer.log").read_text()
        assert observations.count("STUN_TCP_OBSERVED") == (0 if case in ("disabled", "observer-denied") else 2)
        if case in ("disabled", "observer-denied", "target-denied"):
            for side in (0, 1):
                rules = (directory / "wan-rules.txt").read_text().splitlines()
                assert all(line.startswith("[0:0]") for line in rules
                           if f"-s {public[side]}/{128 if ipv6 else 32} -d {public[1-side]}/{128 if ipv6 else 32} -p tcp" in line), rules
        assert elapsed < 10_000
        if case == "positive":
            for role, port in zip("ab", ports):
                lines = (directory / f"{role}.log").read_text().splitlines()
                assert any(line.startswith("TCP_DIAL_START") and f":{port.read_text()} target=" in line for line in lines), lines
                assert any(line.startswith("TCP_PHYSICAL_RELEASED") for line in lines), lines
        result = {"case": case, "ipv6": ipv6, "visitor": "ab"[visitor], "success": case == "positive", "ends": ends}
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        return result
    finally:
        for name in names:
            subprocess.run(["ip", "netns", "del", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_binary", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--case", action="append", choices=("positive", "blocked", "disabled", "observer-denied", "target-denied", "simultaneous"))
    parser.add_argument("--ipv6", action="store_true", help="routed IPv6 with stateful inbound filtering on both gateways")
    args = parser.parse_args()
    binary = args.test_binary.resolve(strict=True)
    output = args.evidence.resolve()
    output.mkdir()
    results = []
    for case in args.case or ("positive", "blocked", "disabled", "observer-denied", "target-denied", "simultaneous"):
        for visitor in (0, 1):
            result = cell(binary, output, case, visitor, args.ipv6)
            results.append(result)
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            print(f"{case} visitor={'ab'[visitor]} success={result['success']}", flush=True)


if __name__ == "__main__":
    main()
