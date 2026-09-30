#!/usr/bin/env python3
"""Allocate four-tuples through NFQUEUE marks and real kernel nftables SNAT.

Every physical source participates. A bounded pool explicitly drops exhausted
destination collision slots; it never lets Linux silently choose an outside
port. The full pool spans 1024..65534. Fixture seeds never reach the product.
"""
import argparse
import json
import random
import signal
import struct
import subprocess


class Allocator:
    def __init__(self, kind, base, step, primary, seed):
        self.kind, self.base, self.step, self.primary = kind, base, step, primary
        self.rng = random.Random(seed)
        self.targets = {}
        self.occupied = {}
        self.collisions = 0
        self.exhausted = 0

    def allocate(self, key):
        if key in self.targets:
            return self.targets[key], False
        src, sport, dst, dport = key
        occupied = self.occupied.setdefault((dst, dport), set())
        if self.kind == "eim":
            mapped = self.base if sport == self.primary else sport
            candidates = [mapped]
        elif self.kind in ("random", "wide"):
            low, size = (self.base, 16) if self.kind == "random" else (1024, 64511)
            first = self.rng.randrange(size)
            candidates = (low + (first + offset) % size for offset in range(size))
        else:
            first = self.base + (4 + len(self.targets)) * self.step
            candidates = (1024 + (first - 1024 + offset) % 64511 for offset in range(64511))
        for mapped in candidates:
            if mapped not in occupied:
                occupied.add(mapped)
                self.targets[key] = mapped
                return mapped, True
            self.collisions += 1
        self.exhausted += 1
        return None, False


def main():
    from netfilterqueue import NetfilterQueue
    p = argparse.ArgumentParser()
    for name in ("kind", "private", "public", "peer", "seed"):
        p.add_argument("--" + name, required=True)
    for name in ("base", "step", "source"):
        p.add_argument("--" + name, type=int, required=True)
    a = p.parse_args()
    allocator = Allocator(a.kind, a.base, a.step, a.source, a.seed)
    # One typed map is installed before traffic. Per-packet callbacks only set
    # a mark; nftables and conntrack perform the actual translation/reversal.
    elements = ", ".join(f"{port} : {port}" for port in range(1024, 65535))
    nft = f"""table ip mio61 {{
map ports {{ type mark : inet_service; elements = {{ {elements} }}
}}
chain post {{ type nat hook postrouting priority 99;
ip saddr {a.private} ip daddr {a.peer} ip protocol udp meta mark > 0 snat ip to {a.public}:meta mark map @ports
}}
}}
"""
    subprocess.run(["nft", "-f", "-"], input=nft, text=True, check=True)

    def reset(*_):
        nonlocal allocator
        allocator = Allocator(a.kind, a.base, a.step, a.source, a.seed)
        print("ALLOCATOR_RESET", flush=True)

    def receive(packet):
        data = packet.get_payload()
        ihl = (data[0] & 15) * 4
        src = ".".join(map(str, data[12:16]))
        dst = ".".join(map(str, data[16:20]))
        sport, dport = struct.unpack("!HH", data[ihl:ihl + 4])
        key = (src, sport, dst, dport)
        mapped, fresh = allocator.allocate(key)
        if mapped is None:
            # This is a documented capacity/collision cost, never a fabricated
            # successful mapping or silent MASQUERADE fallback.
            print(json.dumps({"exhausted": list(key), "collisions": allocator.collisions,
                              "payload": data[ihl + 8:].hex()}), flush=True)
            packet.drop()
            return
        if fresh:
            print(json.dumps({"tuple": list(key), "mapped": [a.public, mapped],
                              "allocation": len(allocator.targets), "collisions": allocator.collisions}), flush=True)
        packet.set_mark(mapped)
        packet.accept()

    def stop(*_):
        raise SystemExit(0)
    signal.signal(signal.SIGUSR1, reset)
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    queue = NetfilterQueue()
    queue.bind(61, receive, max_len=65535)
    print("ALLOCATOR_READY", flush=True)
    try:
        queue.run()
    finally:
        queue.unbind()
        print(json.dumps({"summary": True, "tuples": len(allocator.targets),
                          "collisions": allocator.collisions, "exhausted": allocator.exhausted}), flush=True)


if __name__ == "__main__":
    main()
