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


def cell(binary, output, case, backup=None):
    directory = output / (f"{case}-{backup}" if backup else case)
    directory.mkdir()
    members = directory / "members"
    names = [f"m161-{role}" for role in ("wan", "na", "nb", "a", "b")]
    relay_start = "relay" in case
    fast = "tcp" if case.endswith("tcp") else "udp"
    slow = "udp" if fast == "tcp" else "tcp"
    try:
        topology(names)
        if backup:
            for interface in ("wan-a", "wan-b"):
                run("ip", "netns", "exec", names[0], "tc", "qdisc", "add", "dev", interface,
                    "root", "handle", "1:", "prio")
                run("ip", "netns", "exec", names[0], "tc", "qdisc", "add", "dev", interface,
                    "parent", "1:3", "handle", "30:", "netem", "delay", "40ms")
                run("ip", "netns", "exec", names[0], "tc", "filter", "add", "dev", interface,
                    "parent", "1:", "protocol", "ip", "pref", "1", "flower", "ip_proto", "tcp",
                    "src_ip", PUBLIC[0] if interface == "wan-b" else PUBLIC[1], "flowid", "1:3")
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
        if relay_start or fast == "tcp":
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
                    *([f"MIO163_BACKUP={backup}"] if backup else []),
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

            if fast == "tcp" and not relay_start:
                wait_for(udp_started, 5)
                gate(fast, True)
            wait_for(lambda: (members / "first").exists())
            if relay_start:
                wait_for(udp_started, 5)
                wait_for(lambda: all("TCP_PUNCH_START" in (directory / f"{role}.log").read_text() for role in "ab"), 5)
                (members / "check-relay").touch()
                wait_for(lambda: (members / "relay-held").exists(), 3)
                gate(fast, True)
                (members / "release").touch()
                wait_for(lambda: (members / "fast").exists(), 7)
            gate(slow, True)
            (members / "release").touch()
            wait_for(lambda: (members / "both").exists(), 8)
            if backup:
                wait_for(lambda: all("MEMBER_DIRECT_RETAINED" in (directory / f"{role}.log").read_text()
                                    for role in "ab"), 5)
                (members / "backup-committed").touch()
                wait_for(lambda: all((members / f"backup-ready-{role}").exists() for role in "ab"), 5)
                (members / "backup-aging").touch()
                wait_for(lambda: (members / "backup-stale").exists())
                gate("udp", False)
                if backup == "blocked":
                    gate("tcp", False)
                (members / "backup-fault").touch()
            exits = [p.wait(timeout=15) for p in workers]
            assert exits == [0, 0], (directory, exits)
        for name in names:
            (directory / f"{name}-rules.txt").write_text(run("ip", "netns", "exec", name, "iptables-save", "-c"))
        events = {}
        backup_ends = []
        for role in "ab":
            lines = (directory / f"{role}.log").read_text().splitlines()
            events[role] = [line for line in lines
                            if line.startswith("MIO161 ")]
            assert any(line.startswith("MIO161 BOTH ") for line in events[role]), events
            if backup:
                ready, = [json.loads(line.removeprefix("MIO163 READY ")) for line in lines
                          if line.startswith("MIO163 READY ")]
                final, = [json.loads(line.removeprefix("MIO163 FINAL ")) for line in lines
                          if line.startswith("MIO163 FINAL ")]
                assert ready["path"] == final["path"]
                for field in ("connection", "stream", "peer"):
                    assert ready["transport"][field] == final["transport"][field]
                old, = [path for path in ready["transport"]["paths"] if path["id"] == ready["path"]]
                new, = [path for path in final["transport"]["paths"] if path["id"] == final["path"]]
                assert (old["local"], old["remote"]) == (new["local"], new["remote"])
                assert sum("TCP_PUNCH_START" in line for line in lines) == 1
                assert sum("TCP_OBSERVED" in line for line in lines) == 1
                backup_ends.append(final["transport"]["stream"])
                events[role].extend(line for line in lines if line.startswith("MIO163 "))
        if backup:
            assert backup_ends[0] == backup_ends[1]
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
    parser.add_argument("--backup", choices=("positive", "blocked"))
    args = parser.parse_args()
    args.evidence.mkdir()
    for case in args.case or ("parallel-tcp", "parallel-udp", "parallel-relay-tcp", "parallel-relay-udp"):
        if args.backup:
            assert case == "parallel-relay-udp", "backup checks reuse the UDP-first relay cell"
        cell(args.test_binary.resolve(strict=True), args.evidence.resolve(), case, args.backup)


if __name__ == "__main__":
    main()
