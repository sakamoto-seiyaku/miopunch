#!/usr/bin/env python3
"""Routes/rules/capture only inside a container named by the owned controller."""
import ipaddress
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def run(*argv):
    return subprocess.check_output(["rtk", "proxy", *argv], text=True, stderr=subprocess.STDOUT)


def main():
    role = sys.argv[1]
    action = sys.argv[2] if len(sys.argv) > 2 else "setup"
    graph = json.loads(Path("/topology.json").read_text())
    settings = graph["roles"][role]
    addresses = json.loads(run("ip", "-j", "address"))
    interfaces = {}
    for link, address in settings["links"].items():
        matches = [item["ifname"] for item in addresses if any(entry.get("local") == address for entry in item["addr_info"])]
        if len(matches) != 1:
            raise RuntimeError("owned interface/address is absent or ambiguous")
        interfaces[link] = matches[0]
    if action.startswith("capture-"):
        pid_path = Path("/tmp/owned-captures.json")
        if action == "capture-start":
            pids = []
            for link, interface in interfaces.items():
                with Path(f"/evidence/capture-{role}-{link}.log").open("w") as log:
                    child = subprocess.Popen(["rtk", "proxy", "python3", "/repo/scripts/rtc_cross_network_capture.py",
                        interface, f"/evidence/{role}-{link}.pcap"],
                        stdout=log, stderr=log, start_new_session=True)
                    pids.append(child.pid)
            pid_path.write_text(json.dumps(pids))
            time.sleep(0.1)
            for link in interfaces:
                content = Path(f"/evidence/capture-{role}-{link}.log").read_text()
                if '"event": "capture-ready"' not in content:
                    raise RuntimeError("owned packet capture did not become ready")
        else:
            for pid in json.loads(pid_path.read_text()) if pid_path.exists() else []:
                try:
                    os.killpg(pid, signal.SIGINT)
                except ProcessLookupError:
                    pass
            time.sleep(0.15)
        return
    if action == "snapshot":
        print(json.dumps({"netNamespace": os.readlink("/proc/self/ns/net"), "addresses": addresses,
            "routes": json.loads(run("ip", "-j", "route")), "ipv6Routes": json.loads(run("ip", "-6", "-j", "route")),
            "filterRules": run("iptables-save", "-c"), "ipv6Rules": run("ip6tables-save", "-c"),
            "udpConntrack": run("conntrack", "-L", "-p", "udp") if role.startswith("nat-") else None,
            "capabilities": [line for line in Path("/proc/self/status").read_text().splitlines() if line.startswith("Cap")]}))
        return
    for _ in json.loads(run("ip", "-j", "route", "show", "default")):
        run("ip", "route", "del", "default")
    if settings["default"]:
        run("ip", "route", "add", "default", "via", settings["default"])
    if role == "browser":
        os.chmod("/root", 0o755)  # Only this owned container; UID1000 can read its private NSS mount.
    for chain in ("INPUT", "OUTPUT", "FORWARD"):
        run("ip6tables", "-P", chain, "DROP")
    table = lambda *args: run("iptables", *args)
    table("-P", "INPUT", "DROP")
    table("-P", "OUTPUT", "DROP")
    table("-P", "FORWARD", "DROP")
    table("-A", "INPUT", "-i", "lo", "-j", "ACCEPT")
    table("-A", "OUTPUT", "-o", "lo", "-j", "ACCEPT")
    table("-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT")
    table("-A", "OUTPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT")
    service = graph["service"]
    ports = ["8883", "8084"] + (["8443"] if graph["mode"] in ("prepare", "relay", "prerequisites") else [])
    if role in ("browser", "native"):
        peer = graph["roles"]["nat-b" if role == "browser" else "nat-a"]["links"]["wan-b" if role == "browser" else "wan-a"]
        for chain in ("INPUT", "OUTPUT"):
            table("-A", chain, "-s" if chain == "INPUT" else "-d", peer, "-p", "udp", "-j", "ACCEPT")
        table("-A", "OUTPUT", "-d", service, "-p", "udp", "--dport", "3478", "-j", "ACCEPT")
        for port in ports:
            table("-A", "OUTPUT", "-d", service, "-p", "tcp", "--dport", port, "-j", "ACCEPT")
    elif role.startswith("nat-"):
        a = role == "nat-a"
        lan, wan = ("lan-a", "wan-a") if a else ("lan-b", "wan-b")
        private = graph["roles"]["browser" if a else "native"]["links"][lan]
        outer = settings["links"][wan]
        peer = graph["roles"]["nat-b" if a else "nat-a"]["links"]["wan-b" if a else "wan-a"]
        remote_lan = graph["networks"]["lan-b" if a else "lan-a"]["subnet"]
        table("-A", "FORWARD", "-d", remote_lan, "-j", "DROP")
        table("-A", "FORWARD", "-i", interfaces[wan], "-o", interfaces[lan], "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT")
        table("-A", "FORWARD", "-i", interfaces[wan], "-o", interfaces[lan], "-s", peer, "-d", private, "-p", "udp", "-j", "ACCEPT")
        table("-A", "FORWARD", "-i", interfaces[lan], "-o", interfaces[wan], "-s", private, "-d", peer, "-p", "udp", "-j", "ACCEPT")
        table("-A", "FORWARD", "-i", interfaces[lan], "-o", interfaces[wan], "-s", private, "-d", service, "-p", "udp", "--dport", "3478", "-j", "ACCEPT")
        for port in ports:
            table("-A", "FORWARD", "-i", interfaces[lan], "-o", interfaces[wan], "-s", private, "-d", service, "-p", "tcp", "--dport", port, "-j", "ACCEPT")
        table("-t", "nat", "-A", "POSTROUTING", "-o", interfaces[wan], "-s", private, "-j", "SNAT", "--to-source", outer)
        table("-t", "nat", "-A", "PREROUTING", "-i", interfaces[wan], "-d", outer, "-p", "udp", "-j", "DNAT", "--to-destination", private)
    else:
        run("ip", "address", "add", service+"/32", "dev", "lo")
        for lan in ("lan-a", "lan-b"):
            table("-A", "FORWARD", "-d", graph["networks"][lan]["subnet"], "-j", "DROP")
        for first, second in (("a", "b"), ("b", "a")):
            source = graph["roles"]["nat-"+first]["links"]["wan-"+first]
            destination = graph["roles"]["nat-"+second]["links"]["wan-"+second]
            table("-A", "FORWARD", "-i", interfaces["wan-"+first], "-o", interfaces["wan-"+second], "-s", source, "-d", destination,
                "-p", "udp", "-m", "comment", "--comment", "owned-peer-udp", "-j", "DROP" if graph["mode"] == "blocked" else "ACCEPT")
            table("-A", "INPUT", "-s", source, "-d", service, "-p", "udp", "--dport", "3478", "-j", "ACCEPT")
            for port in ports:
                table("-A", "INPUT", "-s", source, "-d", service, "-p", "tcp", "--dport", port, "-j", "ACCEPT")


if __name__ == "__main__":
    main()
