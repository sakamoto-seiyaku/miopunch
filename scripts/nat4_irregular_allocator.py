#!/usr/bin/env python3
"""Allocate real kernel SNAT tuples before POSTROUTING, never emulate packets.

NFQUEUE pauses each first peer datagram while an exact destination rule is
installed. Conntrack then supplies APDF reverse-tuple filtering. SIGUSR1 resets
only disposable preflight allocations; the selector never reads fixture seeds.
"""
import argparse
import json
import random
import signal
import struct
import subprocess


def allocation(kind, base, step, seed):
    if kind in ("random", "wide"):
        ports = list(range(base, base + 16)) if kind == "random" else list(range(1024, 65535))
        random.Random(seed).shuffle(ports)
        return ports
    return [base + index * step for index in range(256)]


def main():
    from netfilterqueue import NetfilterQueue
    parser = argparse.ArgumentParser()
    for name in ("kind", "private", "public", "peer", "seed"):
        parser.add_argument("--" + name, required=True)
    for name in ("base", "step", "source"):
        parser.add_argument("--" + name, type=int, required=True)
    args = parser.parse_args()
    ports = allocation(args.kind, args.base, args.step, args.seed)
    targets = {}

    def reset(*_):
        subprocess.run(["iptables", "-t", "nat", "-F", "MIO59_PEER"], check=True)
        targets.clear()
        print("ALLOCATOR_RESET", flush=True)

    def receive(packet):
        data = packet.get_payload()
        ihl = (data[0] & 15) * 4
        destination = ".".join(str(octet) for octet in data[16:20])
        sport, dport = struct.unpack("!HH", data[ihl:ihl + 4])
        target = (destination, dport)
        if target not in targets:
            index = 4 + len(targets)
            if index >= len(ports):
                packet.drop()
                return
            mapped = ports[index]
            subprocess.run(["iptables", "-t", "nat", "-A", "MIO59_PEER",
                            "-s", args.private, "-p", "udp", "--sport", str(sport),
                            "-d", destination, "--dport", str(dport), "-j", "SNAT",
                            "--to-source", f"{args.public}:{mapped}"], check=True)
            targets[target] = mapped
            print(json.dumps({"allocation": index, "destination": f"{destination}:{dport}",
                              "source": f"{args.public}:{mapped}"}), flush=True)
        packet.accept()

    signal.signal(signal.SIGUSR1, reset)
    def stop(*_):
        raise SystemExit(0)
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    queue = NetfilterQueue()
    queue.bind(59, receive)
    print("ALLOCATOR_READY", flush=True)
    try:
        queue.run()
    finally:
        queue.unbind()


if __name__ == "__main__":
    main()
