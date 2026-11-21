#!/usr/bin/env python3
"""Exercise UDP-started NativeClient recovery through two disposable NAT gateways."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import subprocess
import time
from urllib.parse import urlparse

from check_member_fusion_nat import topology, rules, run, stop, PUBLIC, REPO

TEST = "native_service_parallel::parallel_paths_through_translated_native_clients"


def check(binary, directory, mode, failed_first):
    directory.mkdir()
    members = directory / "members"
    names = [f"m164-{role}" for role in ("wan", "na", "nb", "a", "b")]
    try:
        topology(names)
        run("ip", "netns", "exec", names[0], "sysctl", "-qw", "net.ipv4.ip_forward=1")
        rules(names, "restricted-positive", 0)
        for gateway in names[1:3]:
            run("ip", "netns", "exec", gateway, "iptables", "-A", "INPUT",
                "-i", "wan0", "-p", "tcp", "--syn", "-j", "DROP")

        def gate(protocol, enabled):
            for side in (0, 1):
                run("ip", "netns", "exec", names[0], "iptables", "-D" if enabled else "-I", "FORWARD",
                    *([] if enabled else ["1"]), "-p", protocol, "-s", PUBLIC[side],
                    "-d", PUBLIC[1-side], "-j", "DROP")

        def gate_initial_tcp(enabled):
            # Fail the initial local dial before either gateway records peer
            # TCP state. The later attempt still traverses both real NATs.
            for side, name in enumerate(names[3:]):
                run("ip", "netns", "exec", name, "iptables",
                    "-D" if enabled else "-I", "OUTPUT",
                    "-p", "tcp", "-d", PUBLIC[1-side], "-j", "DROP")

        if failed_first:
            gate_initial_tcp(False)
        with ExitStack() as stack:
            processes = []

            def process(role, command):
                log = stack.enter_context((directory / f"{role}.log").open("w"))
                child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                processes.append(child)
                stack.callback(stop, child)
                return child

            def worker(role):
                index = {"setup": 0, "a": 3, "b": 4}[role]
                return process(role, ["ip", "netns", "exec", names[index], "env",
                    f"MIO150_WORKER={role}", f"MIO150_ROOT={members}",
                    "MIO150_CASE=parallel-udp", "MIO150_VISITOR=a", f"MIO164_RECOVERY={mode}",
                    str(binary), TEST, "--exact", "--ignored", "--nocapture"])

            def wait_for(signal, seconds=20):
                until = time.monotonic() + seconds
                while not signal():
                    assert all(child.poll() is None for child in processes), directory
                    assert time.monotonic() < until, directory
                    time.sleep(0.02)

            def observer():
                process("observer", ["ip", "netns", "exec", names[0], "python3", "-u",
                        str(REPO / "scripts/tcp_stun_observer.py")])
                wait_for(lambda: "STUN_TCP_READY" in (directory / "observer.log").read_text())

            if failed_first:
                observer()
            worker("setup")
            wait_for(lambda: "MIO150 LAB_READY" in (directory / "setup.log").read_text())
            config = json.loads((members / "consumer" / "resources.json").read_text())
            relay_port = next(urlparse(r["url"]).port for r in config["defaults"]["resources"]
                              if "DataRelay" in r["uses"])
            relay_rule = ["INPUT", "-p", "tcp", "--dport", str(relay_port), "-j", "DROP"]
            run("ip", "netns", "exec", names[0], "iptables", "-I", *relay_rule)
            peers = [worker("b"), worker("a")]
            wait_for(lambda: all((members / f"port-{role}").exists() for role in "ab"))
            (members / "start").touch()
            wait_for(lambda: (members / "first").exists())
            if mode == "relay":
                run("ip", "netns", "exec", names[0], "iptables", "-D", *relay_rule)
            wait_for(lambda: (members / "udp-ready").exists())
            if not failed_first:
                observer()
            else:
                for side, name in enumerate(names[3:]):
                    sockets = run("ip", "netns", "exec", name, "ss", "-tanpo")
                    (directory / f"sockets-before-{side}.txt").write_text(sockets)
                    assert not any("SYN-SENT" in line and PUBLIC[1-side] in line
                                   for line in sockets.splitlines()), sockets
            gate("udp", False)
            if failed_first:
                gate_initial_tcp(True)
            (members / "fault").touch()
            if mode == "relay":
                wait_for(lambda: (members / "relay").exists(), 5)
            wait_for(lambda: (members / "tcp-used").exists(), 15)
            wait_for(lambda: all("MEMBER_DIRECT_RETAINED" in (directory / f"{role}.log").read_text()
                                for role in "ab"), 3)
            if mode == "relay":
                run("ip", "netns", "exec", names[0], "iptables", "-I", *relay_rule)
            (members / "tcp-committed").touch()
            assert [peer.wait(timeout=6) for peer in peers] == [0, 0]
        for role in "ab":
            lines = (directory / f"{role}.log").read_text().splitlines()
            records = [line for line in lines if line.startswith("MIO164 ")]
            (directory / f"{role}-result.json").write_text(json.dumps(records, indent=2) + "\n")
        consumer_lines = (directory / "a.log").read_text().splitlines()
        first, = [json.loads(line.removeprefix("MIO164 FIRST ")) for line in consumer_lines
                  if line.startswith("MIO164 FIRST ")]
        delivered, = [json.loads(line.removeprefix("MIO164 SERVICE_PROOF ")) for line in consumer_lines
                      if line.startswith("MIO164 SERVICE_PROOF ")]
        peer_lines = (directory / "b.log").read_text().splitlines()
        peer, = [json.loads(line.removeprefix("MIO164 PEER ")) for line in peer_lines
                 if line.startswith("MIO164 PEER ")]
        assert first["connection"] == delivered["connection"]
        assert first["stream"] == delivered["stream"] == peer["stream"]
        assert any(path["remote"].get("Custom") and path["rx_stream_frames"] > 0
                   for path in peer["paths"]), peer
        if mode == "signaling":
            assert any('"source":"member_fusion_repair"' in line and '"result":"succeeded"' in line
                       for line in consumer_lines)
        print(f"UDP-started TCP recovery passed via {mode}", flush=True)
    finally:
        for name in names:
            snapshot = subprocess.run(["ip", "netns", "exec", name, "iptables-save", "-c"],
                                      capture_output=True, text=True)
            if snapshot.returncode == 0:
                (directory / f"{name}-rules.txt").write_text(snapshot.stdout)
            subprocess.run(["ip", "netns", "del", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_binary", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--mode", choices=("relay", "signaling"), default="relay")
    parser.add_argument("--failed-first", action="store_true")
    args = parser.parse_args()
    check(args.test_binary.resolve(strict=True), args.evidence.resolve(), args.mode, args.failed_first)
