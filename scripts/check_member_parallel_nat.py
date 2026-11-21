#!/usr/bin/env python3
"""Check first business and late TCP/UDP completion behind two real gateways."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import subprocess
import time

from check_member_fusion_nat import topology, rules, run, stop, PUBLIC, REPO

TEST = "native_service_parallel::parallel_paths_through_translated_native_clients"


def cell(binary, output, case):
    directory = output / case
    directory.mkdir()
    members = directory / "members"
    names = [f"m161-{role}" for role in ("wan", "na", "nb", "a", "b")]
    fast = "tcp" if case.endswith("tcp") else "udp"
    slow = "udp" if fast == "tcp" else "tcp"
    try:
        topology(names)
        run("ip", "netns", "exec", names[0], "sysctl", "-qw", "net.ipv4.ip_forward=1")
        rules(names, "restricted-positive", 0)
        for gateway in names[1:3]:
            run("ip", "netns", "exec", gateway, "iptables", "-A", "INPUT",
                "-i", "wan0", "-p", "tcp", "--syn", "-j", "DROP")

        def gate(protocol, enable):
            for side in (0, 1):
                run("ip", "netns", "exec", names[0], "iptables",
                    "-D" if enable else "-I", "FORWARD",
                    *([] if enable else ["1"]), "-p", protocol,
                    "-s", PUBLIC[side], "-d", PUBLIC[1-side], "-j", "DROP")

        gate(slow, False)
        if "relay" in case or fast == "tcp":
            gate(fast, False)
        # Count real enhanced UDP probes before the gates, without changing forwarding.
        run("ip", "netns", "exec", names[0], "iptables", "-I", "FORWARD", "1",
            "-p", "udp", "-m", "string", "--algo", "bm", "--from", "28", "--to", "39",
            "--hex-string", "|004d494f50554e43483633|")
        with ExitStack() as stack:
            processes = []

            def process(role, command):
                log = stack.enter_context((directory / f"{role}.log").open("w"))
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                processes.append(proc)
                stack.callback(stop, proc)
                return proc

            process("observer", ["ip", "netns", "exec", names[0], "python3", "-u",
                    str(REPO / "scripts/tcp_stun_observer.py")])

            def worker(role):
                index = {"setup": 0, "a": 3, "b": 4}[role]
                return process(role, ["ip", "netns", "exec", names[index], "env",
                    f"MIO150_WORKER={role}", f"MIO150_ROOT={members}",
                    f"MIO150_CASE={case}", "MIO150_VISITOR=a",
                    str(binary), TEST, "--exact", "--ignored", "--nocapture"])

            def wait_for(predicate, seconds=20):
                deadline = time.monotonic() + seconds
                while not predicate():
                    assert all(p.poll() is None for p in processes), directory
                    assert time.monotonic() < deadline, directory
                    time.sleep(0.02)

            worker("setup")
            wait_for(lambda: "MIO150 LAB_READY" in (directory / "setup.log").read_text())
            workers = [worker("b"), worker("a")]
            wait_for(lambda: all((members / f"port-{role}").exists() for role in "ab"))
            (members / "start").touch()
            def udp_started():
                return all((directory / f"{role}.log").read_text().count("STUN_UDP_SEND") >= 2
                           for role in "ab")

            if fast == "tcp" and "relay" not in case:
                wait_for(udp_started, 5)
                gate(fast, True)
            wait_for(lambda: (members / "first").exists())
            if "relay" in case:
                if fast == "tcp":
                    wait_for(udp_started, 5)
                gate(fast, True)
                (members / "release").touch()
                wait_for(lambda: (members / "fast").exists(), 7)
            gate(slow, True)
            (members / "release").touch()
            wait_for(lambda: (members / "both").exists(), 8)
            exits = [p.wait(timeout=15) for p in workers]
            assert exits == [0, 0], (directory, exits)
        for name in names:
            (directory / f"{name}-rules.txt").write_text(run("ip", "netns", "exec", name, "iptables-save", "-c"))
        events = {}
        for role in "ab":
            events[role] = [line for line in (directory / f"{role}.log").read_text().splitlines()
                            if line.startswith("MIO161 ")]
            assert any(line.startswith("MIO161 BOTH ") for line in events[role]), events
        (directory / "result.json").write_text(json.dumps(events, indent=2) + "\n")
        print(case, "passed", flush=True)
    finally:
        for name in names:
            snapshot = subprocess.run(["ip", "netns", "exec", name, "iptables-save", "-c"],
                                      capture_output=True, text=True)
            if snapshot.returncode == 0:
                (directory / f"{name}-rules.txt").write_text(snapshot.stdout)
            subprocess.run(["ip", "netns", "del", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_binary", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--case", action="append", choices=("parallel-tcp", "parallel-udp", "parallel-relay-tcp", "parallel-relay-udp"))
    args = parser.parse_args()
    args.evidence.mkdir()
    for case in args.case or ("parallel-tcp", "parallel-udp", "parallel-relay-tcp", "parallel-relay-udp"):
        cell(args.test_binary.resolve(strict=True), args.evidence.resolve(), case)


if __name__ == "__main__":
    main()
