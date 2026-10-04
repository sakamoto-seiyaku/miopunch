#!/usr/bin/env python3
"""Validate actual selected tuples against separate LAN/WAN pcaps and conntrack."""
import json
from datetime import datetime
import re
import socket
import struct


def packets(path):
    with path.open("rb") as source:
        header = source.read(24)
        if len(header) != 24 or header[:4] != b"\xd4\xc3\xb2\xa1" or struct.unpack("<I", header[20:])[0] != 1:
            raise ValueError("require little-endian Ethernet pcap")
        while True:
            record = source.read(16)
            if not record:
                return
            if len(record) != 16:
                raise ValueError("truncated pcap record")
            seconds, micros, captured, original = struct.unpack("<IIII", record)
            if captured > 256 or captured > original:
                raise ValueError("pcap exceeds owned snaplen")
            frame = source.read(captured)
            if len(frame) != captured:
                raise ValueError("truncated pcap frame")
            if len(frame) < 34 or frame[12:14] != b"\x08\x00":
                continue
            ip = frame[14:]
            length, offset = struct.unpack("!H", ip[2:4])[0], (ip[0] & 15)*4
            protocol = ip[9]
            if offset < 20 or len(ip) < offset+8 or struct.unpack("!H", ip[6:8])[0] & 0x1fff:
                raise ValueError("unsupported/truncated fragmented packet")
            if protocol in (6, 17):
                source_port, destination_port = struct.unpack("!HH", ip[offset:offset+4])
                transport_header = 8 if protocol == 17 else (ip[offset+12] >> 4)*4
            else:
                source_port = destination_port = transport_header = 0
            payload = ip[offset+transport_header:]
            yield {"tuple": (socket.inet_ntoa(ip[12:16]), socket.inet_ntoa(ip[16:20]), source_port, destination_port, protocol),
                "timestamp": seconds+micros/1000000, "ipBytes": length, "ethernetBytes": original,
                "transportPayloadBytes": max(0, length-offset-transport_header),
                "stun": protocol == 17 and len(payload) >= 20 and payload[4:8] == b"\x21\x12\xa4\x42"}


def totals(rows):
    return {"packets": len(rows), "ipBytes": sum(row["ipBytes"] for row in rows),
        "ethernetBytes": sum(row["ethernetBytes"] for row in rows),
        "transportPayloadBytes": sum(row["transportPayloadBytes"] for row in rows),
        "stunPackets": sum(row["stun"] for row in rows),
        "firstTimestamp": min((row["timestamp"] for row in rows), default=None),
        "lastTimestamp": max((row["timestamp"] for row in rows), default=None)}


def matching(rows, first, second, first_port, second_port):
    return [row for row in rows if row["tuple"] == (first, second, first_port, second_port, 17)]


def conntrack_contains(text, first, second, first_port, second_port):
    pattern = f"src={first} dst={second} sport={first_port} dport={second_port}"
    return any(pattern in line for line in text.splitlines())


def validate(output, mode, story, observations):
    graph = json.loads((output/"topology.json").read_text())
    a = graph["roles"]["nat-a"]["links"]["wan-a"]
    b = graph["roles"]["nat-b"]["links"]["wan-b"]
    private_a = graph["roles"]["browser"]["links"]["lan-a"]
    private_b = graph["roles"]["native"]["links"]["lan-b"]
    captures = {}
    capture_statistics = {}
    for role, link in (("nat-a", "lan-a"), ("nat-a", "wan-a"), ("nat-b", "lan-b"), ("nat-b", "wan-b"), ("router", "wan-a"), ("router", "wan-b")):
        name = role+"-"+link
        rows = list(packets(output/(name+".pcap")))
        records = [json.loads(line) for line in (output/("capture-"+name+".log")).read_text().splitlines()]
        final = next(row for row in records if row["event"] == "capture-complete")
        if final["kernelDropped"] or final["packetLimitReached"] or final["captured"] != len(rows) or not rows:
            raise ValueError("incomplete/lossy capture: "+name)
        captures[name] = rows
        capture_statistics[name] = {**final, "ipv4": totals(rows)}
    snapshot = json.loads((output/"namespace-after-member.json").read_text())
    peer_rules = [line for line in snapshot["router"]["filterRules"].splitlines() if "owned-peer-udp" in line]
    rule_packets = sum(int(re.match(r"\[(\d+):", line)[1]) for line in peer_rules)
    result = {"passed": False, "captures": capture_statistics, "peerRouterRules": peer_rules, "members": [],
        "wireScope": "IPv4/transport wire counts incl retransmission; one WAN interface per endpoint, never summed across capture points",
        "limitations": ["outer DTLS/SCTP pcap cannot identify encrypted inner QUIC STREAMs", "snapshot stats are asynchronous, not atomic between endpoints",
            "browser internal copies/buffers and SCTP outstanding bytes unmeasured", "userspace capture timestamps, no hardware timestamps"],
        "endpointWire": {}}
    for side, name in (("browser", "nat-a-wan-a"), ("native", "nat-b-wan-b")):
        outer = a if side == "browser" else b
        rows = captures[name]
        groups = {}
        for direction in ("sent", "received"):
            candidates = [row for row in rows if row["tuple"][0 if direction == "sent" else 1] == outer]
            groups[direction] = {}
            for layer, predicate in (
                ("peerUdp", lambda row: row["tuple"][4] == 17 and {row["tuple"][0], row["tuple"][1]} == {a, b}),
                ("observerStunUdp", lambda row: row["tuple"][4] == 17 and graph["service"] in row["tuple"][:2] and 3478 in row["tuple"][2:4]),
                ("mqttWssTlsTcp", lambda row: row["tuple"][4] == 6 and graph["service"] in row["tuple"][:2] and any(port in row["tuple"][2:4] for port in (8883, 8084))),
                ("dataRelayTlsTcp", lambda row: row["tuple"][4] == 6 and graph["service"] in row["tuple"][:2] and 8443 in row["tuple"][2:4]),
            ):
                groups[direction][layer] = totals([row for row in candidates if predicate(row)])
        result["endpointWire"][side] = groups
    if mode == "direct":
        if any(group["dataRelayTlsTcp"]["packets"] for endpoint in result["endpointWire"].values() for group in endpoint.values()):
            raise ValueError("direct case used relay TCP")
        for label, browser, native in (("BUSINESS", story["businessBefore"]["bootstrap"], story["businessNativeFinal"]),
                                      ("REPLICA", story["replicaSession"]["before"]["bootstrap"], story["replicaNativeFinal"])):
            stats = browser["rtc"]["stats"]
            local, remote, pair = stats["localCandidate"], stats["remoteCandidate"], stats["selectedPair"]
            if stats["transport"]["selectedCandidatePairId"] != pair["id"] or pair["state"] != "succeeded":
                raise ValueError("actual browser selected pair absent")
            if any(candidate["candidateType"] == "relay" or candidate["protocol"] != "udp" for candidate in (local, remote)):
                raise ValueError("selected pair is not nonrelay UDP")
            pa, pb = local["port"], remote["port"]
            if local["address"] != a or remote["address"] != b or native["selectedPair"]["local"]["address"] != private_b or native["selectedPair"]["local"]["port"] != pb or native["selectedPair"]["remote"]["address"] != a or native["selectedPair"]["remote"]["port"] != pa:
                raise ValueError("selected candidates do not match translated topology")
            native_transport = next(row for row in native["stats"] if row["type"] == "transport")
            native_pair = next(row for row in native["stats"] if row["id"] == native_transport["selectedCandidatePairId"])
            if native_pair["state"] != "succeeded" or any(native["selectedPair"][side]["typ"] == "relay" for side in ("local", "remote")):
                raise ValueError("actual native selected pair absent/relay")
            if not all(any(entry["actualSource"] == [address, port] for entry in observations) for address, port in ((a, pa), (b, pb))):
                raise ValueError("actual STUN recvfrom did not observe both selected external tuples")
            member = {"label": label, "browserStableId": browser["memberStableId"], "nativeStableId": native["memberStableId"],
                "browserSelectedPair": stats, "nativeSelectedPair": native["selectedPair"], "nativeSelectedPairStats": native_pair,
                "pathId": browser["pathId"], "tupleCaptures": {}, "translationConntrack": {}}
            start_key, end_key = ("businessPayloadStartedUtc", "businessPayloadEndedUtc") if label == "BUSINESS" else ("replicaOperationStartedUtc", "replicaOperationEndedUtc")
            start = datetime.fromisoformat(story[start_key]).timestamp()
            end = datetime.fromisoformat(story[end_key]).timestamp()
            if end <= start:
                raise ValueError("actual application window timestamps absent/invalid")
            member["applicationWindow"] = {"startUtc": story[start_key], "endUtc": story[end_key],
                "scope": "real BUSINESS PING/PONG + path snapshots" if label == "BUSINESS" else "fresh independent REPLICA setup + verified durable save/Receipt",
                "browserWan": {}}
            for name, first, second in (("nat-a-lan-a", private_a, b), ("nat-a-wan-a", a, b),
                ("router-wan-a", a, b), ("router-wan-b", a, b), ("nat-b-wan-b", a, b), ("nat-b-lan-b", a, private_b)):
                forward = matching(captures[name], first, second, pa, pb)
                reverse = matching(captures[name], second, first, pb, pa)
                if not forward or not reverse or not any(not row["stun"] for row in forward+reverse):
                    raise ValueError("bidirectional actual selected tuple missing from "+name)
                member["tupleCaptures"][name] = {"browserToNative": totals(forward), "nativeToBrowser": totals(reverse)}
                if name == "nat-a-wan-a":
                    for direction, rows in (("sent", forward), ("received", reverse)):
                        window = [row for row in rows if start <= row["timestamp"] <= end]
                        if not window or not any(not row["stun"] for row in window):
                            raise ValueError("actual selected peer UDP absent in application window")
                        member["applicationWindow"]["browserWan"][direction] = totals(window)
            for name, first, second, pfirst, psecond in (("nat-a", private_a, b, pa, pb), ("nat-b", private_b, a, pb, pa)):
                text = snapshot[name]["udpConntrack"]
                # Either initiation direction is valid; require actual original/reply records containing private AND WAN tuples.
                lines = [line for line in text.splitlines() if conntrack_contains(line, first, second, pfirst, psecond) or conntrack_contains(line, second, first, psecond, pfirst)]
                if not lines or not any((a if name == "nat-a" else b) in line for line in lines):
                    raise ValueError("selected NAT conntrack translation absent: "+name)
                member["translationConntrack"][name] = lines
            result["members"].append(member)
        if rule_packets == 0 or any("-j DROP" in line for line in peer_rules):
            raise ValueError("direct peer UDP router acceptance absent")
    elif mode == "blocked":
        if rule_packets == 0 or len(peer_rules) != 2 or not all("-j DROP" in line for line in peer_rules):
            raise ValueError("actual peer UDP DROP counter absent")
        if not any({row["tuple"][0], row["tuple"][1]} == {a, b} and row["tuple"][4] == 17 for name in ("router-wan-a", "router-wan-b") for row in captures[name]):
            raise ValueError("blocked peer UDP attempt not captured")
        if not story["noOrdinaryRelease"] or story.get("unexpectedHandle") or story["nativeAfter"]["publishedRtcRoutes"]:
            raise ValueError("blocked case released ordinary data")
        if any(group["dataRelayTlsTcp"]["packets"] for endpoint in result["endpointWire"].values() for group in endpoint.values()):
            raise ValueError("blocked case fell back to relay")
    elif mode == "relay":
        if story["physical"] or any(group["peerUdp"]["packets"] for endpoint in result["endpointWire"].values() for group in endpoint.values()):
            raise ValueError("relay comparison used RTC/peer UDP")
        if not all(any(group["dataRelayTlsTcp"]["packets"] for group in endpoint.values()) for endpoint in result["endpointWire"].values()):
            raise ValueError("actual relay TCP absent")
    result["passed"] = True
    return result
