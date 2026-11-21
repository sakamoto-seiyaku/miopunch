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


def cell(binary, output, case, visitor):
    directory = output / f"{case}-{'ab'[visitor]}"
    directory.mkdir()
    names = [f"m159-{role}" for role in ("wan", "na", "nb", "a", "b")]
    try:
        topology(names)
        run("ip", "netns", "exec", names[0], "sysctl", "-qw", "net.ipv4.ip_forward=1")
        for gateway in names[1:3]:
            # Leave forwarding governed by real conntrack. No static DNAT.
            run("ip", "netns", "exec", gateway, "iptables", "-A", "INPUT",
                "-i", "wan0", "-p", "tcp", "--syn", "-j", "DROP")
        for side in (0, 1):
            run("ip", "netns", "exec", names[0], "iptables", "-A", "FORWARD",
                "-p", "udp", "-s", PUBLIC[side], "-d", PUBLIC[1-side], "-j", "DROP")
            run("ip", "netns", "exec", names[0], "iptables", "-A", "FORWARD",
                "-p", "tcp", "-s", PUBLIC[side], "-d", PUBLIC[1-side], "-j",
                "DROP" if case == "blocked" else "ACCEPT")
        with ExitStack() as stack:
            def process(role, command):
                log = stack.enter_context((directory / f"{role}.log").open("w"))
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                stack.callback(stop, proc)
                return proc

            observer = process("observer", ["ip", "netns", "exec", names[0], "python3",
                                           "-u", str(REPO / "scripts/tcp_stun_observer.py")])

            def worker(role):
                index = {"setup": 0, "a": 3, "b": 4}[role]
                return process(role, ["ip", "netns", "exec", names[index], "env",
                    f"MIO150_WORKER={role}", f"MIO150_ROOT={directory / 'members'}",
                    f"MIO150_CASE={case}", f"MIO150_VISITOR={'ab'[visitor]}",
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
            (directory / "members" / "start").touch()
            exits = [p.wait(timeout=max(1, deadline - time.monotonic())) for p in workers]
            assert exits == [0, 0], (directory, exits)
            initials, = [line for line in (directory / "setup.log").read_text().splitlines()
                         if line.startswith("MIO155 MQTT_INITIALS ")]
            dials = int(initials.rsplit(" ", 1)[1])
            assert 1 <= dials <= (2 if case == "simultaneous" else 1)
        for name, label in zip(names, ("wan", "gateway-a", "gateway-b", "a", "b")):
            (directory / f"{label}-rules.txt").write_text(
                run("ip", "netns", "exec", name, "iptables-save", "-c"))
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
        for side, role in enumerate("ab"):
            lines = (directory / f"{role}.log").read_text().splitlines()
            observed = [line for line in lines if line.startswith("TCP_OBSERVED")]
            punched = any("TCP_PUNCH_START" in line for line in lines)
            if case in ("disabled", "observer-denied"):
                assert not observed and not punched, lines
            else:
                assert len(observed) == 1
                assert f"listener=0.0.0.0:{ports[side].read_text()}" in observed[0]
                assert f"candidate={PUBLIC[side]}:" in observed[0]
                assert punched == (case != "target-denied")
        observations = (directory / "observer.log").read_text()
        assert observations.count("STUN_TCP_OBSERVED") == (0 if case in ("disabled", "observer-denied") else 2)
        if case in ("disabled", "observer-denied", "target-denied"):
            for side in (0, 1):
                rules = (directory / "wan-rules.txt").read_text().splitlines()
                assert all(line.startswith("[0:0]") for line in rules
                           if f"-s {PUBLIC[side]}/32 -d {PUBLIC[1-side]}/32 -p tcp" in line), rules
        assert elapsed < 10_000
        result = {"case": case, "visitor": "ab"[visitor], "success": case == "positive", "ends": ends}
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
    args = parser.parse_args()
    binary = args.test_binary.resolve(strict=True)
    output = args.evidence.resolve()
    output.mkdir()
    results = []
    for case in args.case or ("positive", "blocked", "disabled", "observer-denied", "target-denied", "simultaneous"):
        for visitor in (0, 1):
            result = cell(binary, output, case, visitor)
            results.append(result)
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            print(f"{case} visitor={'ab'[visitor]} success={result['success']}", flush=True)


if __name__ == "__main__":
    main()
