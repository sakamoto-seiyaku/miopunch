#!/usr/bin/env python3
"""Bounded Ethernet IPv4 pcap recorder using only the owned NET_RAW capability."""
import json
import signal
import socket
import struct
import sys
import time
from pathlib import Path


def main():
    interface, destination = sys.argv[1:]
    running = True

    def stop(*_):
        nonlocal running
        running = False

    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    captured = received = dropped = 0
    started = time.monotonic()
    with socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003)) as source:
        source.bind((interface, 0))
        source.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 8 * 1024 * 1024)
        source.settimeout(0.05)
        with Path(destination).open("wb") as output:
            output.write(struct.pack("<IHHIIII", 0xa1b2c3d4, 2, 4, 0, 0, 256, 1))
            output.flush()
            print(json.dumps({"event": "capture-ready", "interface": interface}), flush=True)
            while running and captured < 10000 and time.monotonic()-started < 120:
                try:
                    packet = source.recv(65535)
                except socket.timeout:
                    continue
                if len(packet) < 14 or packet[12:14] != b"\x08\x00":
                    continue
                now = time.time_ns()
                original = len(packet)
                output.write(struct.pack("<IIII", now // 1000000000, (now % 1000000000) // 1000, min(original, 256), original))
                output.write(packet[:256])
                captured += 1
            output.flush()
        # PACKET_STATISTICS returns/reset actual kernel packet and drop counts.
        received, dropped = struct.unpack("II", source.getsockopt(263, 6, 8))
    print(json.dumps({"event": "capture-complete", "interface": interface, "captured": captured,
        "kernelReceived": received, "kernelDropped": dropped, "packetLimitReached": captured == 10000,
        "snaplen": 256, "kernelCountScope": "all Ethernet frames including filtered non-IPv4 frames",
        "timestampScope": "userspace receive wall clock; no hardware timestamp",
        "seconds": time.monotonic()-started}), flush=True)


if __name__ == "__main__":
    main()
