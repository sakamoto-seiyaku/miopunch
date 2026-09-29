#!/usr/bin/env python3
"""Read this lab's Linux SLL2 pcap to time QAD results and first peer probes."""

import glob
import ipaddress
import json
import struct
import sys


def packets(path):
    with open(path, "rb") as file:
        head = file.read(24)
        assert head[:4] == bytes.fromhex("d4c3b2a1")
        assert struct.unpack("<I", head[20:24])[0] == 276
        while header := file.read(16):
            sec, usec, caplen, _ = struct.unpack("<IIII", header)
            frame = file.read(caplen)
            if frame[:2] != b"\x08\x00":
                continue
            ip = frame[20:]
            if len(ip) < 28 or ip[0] >> 4 != 4 or ip[9] != 17:
                continue
            ihl = (ip[0] & 15) * 4
            src = str(ipaddress.IPv4Address(ip[12:16]))
            dst = str(ipaddress.IPv4Address(ip[16:20]))
            sport, dport, udp_len = struct.unpack("!HHH", ip[ihl:ihl + 6])
            yield sec + usec / 1_000_000, src, dst, sport, dport, udp_len - 8


def main(root):
    rows = []
    for path in glob.glob(root + "/*/wan.pcap"):
        events = list(packets(path))
        record = {"trial": path.split("/")[-2]}
        for public in ("100.64.27.2", "100.64.27.6"):
            peer = "100.64.27.6" if public.endswith(".2") else "100.64.27.2"
            primes = [time for time, src, dst, _, _, length in events
                      if src == public and dst == peer and length == 11]
            qad = [time for time, src, dst, sport, _, _ in events
                   if src == "100.64.27.1" and dst == public and sport == 46764]
            if not primes or not qad:
                raise SystemExit(f"missing QAD/probe packet in {path} for {public}")
            before = [time for time in qad if time < min(primes)]
            if not before:
                raise SystemExit(f"no QAD result before peer probe in {path} for {public}")
            record["age_" + public] = round((min(primes) - max(before)) * 1000)
            record["first_probe_" + public] = min(primes)
        rows.append(record)
    print(json.dumps({"trials": len(rows), "max_age_ms": max(row[key] for row in rows for key in row if key.startswith("age_")),
                      "min_age_ms": min(row[key] for row in rows for key in row if key.startswith("age_")),
                      "rows": sorted(rows, key=lambda row: row["trial"])}, indent=2))


if __name__ == "__main__":
    main(sys.argv[1])
