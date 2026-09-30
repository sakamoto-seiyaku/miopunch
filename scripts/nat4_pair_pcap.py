#!/usr/bin/env python3
"""Validate bilateral probe order from independent WAN SLL2 captures."""

import glob
import ipaddress
import re
import struct
from pathlib import Path
import json
import sys

from nat4_pcap_timing import packets


PUBLIC = ("100.64.27.2", "100.64.27.6")


def packet_records(path):
    """Keep SLL2 packet type: WAN ingress sees every gateway send exactly once."""
    with open(path, "rb") as file:
        head = file.read(24)
        if head[:4] != bytes.fromhex("d4c3b2a1") or struct.unpack("<I", head[20:24])[0] != 276:
            raise ValueError(f"expected little-endian SLL2 pcap: {path}")
        while header := file.read(16):
            sec, usec, caplen, _ = struct.unpack("<IIII", header)
            frame = file.read(caplen)
            if len(frame) < 48 or frame[:2] != b"\x08\x00":
                continue
            ip = frame[20:]
            if ip[0] >> 4 != 4 or ip[9] != 17:
                continue
            ihl = (ip[0] & 15) * 4
            sport, dport, udp_len = struct.unpack("!HHH", ip[ihl:ihl + 6])
            yield {"time": sec + usec / 1_000_000, "packet_type": frame[10],
                   "src": str(ipaddress.IPv4Address(ip[12:16])),
                   "dst": str(ipaddress.IPv4Address(ip[16:20])),
                   "sport": sport, "dport": dport, "length": udp_len - 8,
                   "payload": ip[ihl + 8:ihl + udp_len]}


def peer_ingress(path):
    return [event for event in packet_records(path)
            if event["packet_type"] == 0 and event["src"] in PUBLIC
            and event["dst"] in PUBLIC and event["src"] != event["dst"]]


def primes(path):
    return [event for event in peer_ingress(path) if event["payload"] == b"\0MIOPUNCH27"]


def costs(path):
    prime_events = primes(path)
    if not prime_events:
        raise ValueError(f"missing peer prime for cost measurement: {path}")
    peer = [event for event in peer_ingress(path)
            if event["time"] >= min(prime["time"] for prime in prime_events)]
    qad = [event for event in packet_records(path)
           if (event["packet_type"] == 0 and event["src"] in PUBLIC
               and event["dst"] == "100.64.27.1" and 46761 <= event["dport"] <= 46764)
           or (event["packet_type"] == 4 and event["src"] == "100.64.27.1"
               and event["dst"] in PUBLIC and 46761 <= event["sport"] <= 46764)]
    return {"trial": Path(path).parent.name, "application_primes": len(prime_events),
            "peer_udp_datagrams": len(peer),
            "peer_udp_payload_bytes": sum(event["length"] for event in peer),
            "qad_udp_datagrams": len(qad),
            "qad_udp_payload_bytes": sum(event["length"] for event in qad)}


def main(root):
    rows = []
    for path in sorted(glob.glob(root + "/*-auto/wan.pcap")):
        trial = path.split("/")[-2]
        _, dialer, _, strategy = trial.rsplit("-", 3)
        assert strategy == "auto"
        server = PUBLIC[1] if dialer == "a" else PUBLIC[0]
        client = PUBLIC[0] if dialer == "a" else PUBLIC[1]
        events = list(packets(path))
        prime_events = primes(path)
        server_events = [event for event in prime_events if event["src"] == server]
        client_events = [event for event in prime_events if event["src"] == client]
        server_probes = [event["time"] for event in server_events]
        client_probes = [event["time"] for event in client_events]
        directory = Path(path).parent
        expected = {}
        for name, public in (("server", server), ("client", client)):
            log = (directory / f"{name}.log").read_text()
            target = re.search(r"AUTO_SELECTED role=\w+ target=(\S+)", log).group(1)
            expected[public] = target
        for event in prime_events:
            if f"{event['dst']}:{event['dport']}" != expected[event["src"]]:
                raise SystemExit(f"prime went to an unselected target in {path}")
        for event in server_events + client_events:
            public = event["src"]
            other = client if public == server else server
            if f"{public}:{event['sport']}" != expected[other]:
                raise SystemExit(f"prime source mapping missed prediction in {path}")
        if len(server_probes) != 1 or len(client_probes) != 1:
            raise SystemExit(f"wrong prime packet count in {path}: "
                             f"{server_probes} / {client_probes}")
        if not server_probes[0] < client_probes[0]:
            raise SystemExit(f"wrong bilateral order in {path}")
        ages = {}
        for public, first in ((server, server_probes[0]), (client, client_probes[0])):
            qad = [time for time, src, dst, sport, _, _ in events
                   if src == "100.64.27.1" and dst == public and sport == 46764
                   and time < first]
            if not qad:
                raise SystemExit(f"missing final QAD result before peer probe in {path}")
            ages[public] = round((first - max(qad)) * 1000)
            if ages[public] > 2000:
                raise SystemExit(f"QAD evidence was stale at peer send in {path}")
        peer_packets = [event for event in peer_ingress(path)
                        if event["time"] >= server_probes[0]]
        if not any(event["payload"] != b"\0MIOPUNCH27" for event in peer_packets):
            raise SystemExit(f"successful trial lacked captured QUIC path packets in {path}")
        rows.append({"trial": trial, "server": server, "client": client,
                     "server_first_ms_before_client": round((client_probes[0] - server_probes[0]) * 1000),
                     "server_qad_age_ms": ages[server], "client_qad_age_ms": ages[client],
                     "application_primes": len(prime_events),
                     "peer_udp_datagrams": len(peer_packets),
                     "peer_udp_payload_bytes": sum(event["length"] for event in peer_packets),
                     "peer_quic_datagrams": sum(event["payload"] != b"\0MIOPUNCH27" for event in peer_packets)})
    if not rows:
        raise SystemExit("no auto captures found")
    cost_rows = [costs(path) for path in sorted(glob.glob(root + "/*/wan.pcap"))]
    print(json.dumps({"trials": len(rows), "rows": rows, "costs": cost_rows}, indent=2))


if __name__ == "__main__":
    main(sys.argv[1])
