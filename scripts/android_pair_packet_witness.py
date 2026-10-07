#!/usr/bin/env python3
"""Correlate only exact SDK socket tuples in two private UDP captures."""

from __future__ import annotations

import hashlib
import ipaddress
from pathlib import Path
import struct

MAX_CAPTURE_BYTES = 32 * 1024 * 1024


def _ice_records(path: Path) -> tuple[list, bool]:
    raw = path.read_bytes()
    if not 24 <= len(raw) <= MAX_CAPTURE_BYTES:
        raise ValueError("packet witness capture size is invalid")
    orders = {b"\xd4\xc3\xb2\xa1": "<", b"\xa1\xb2\xc3\xd4": ">"}
    if raw[:4] not in orders:
        raise ValueError("packet witness requires a microsecond PCAP")
    order = orders[raw[:4]]
    link = struct.unpack(order + "IHHIIII", raw[:24])[-1]
    if link not in (1, 113):
        raise ValueError("packet witness link type is unsupported")
    records = []
    offset, incomplete = 24, False
    while offset < len(raw):
        if len(raw) - offset < 16:
            incomplete = True
            break
        _, _, size, wire = struct.unpack(order + "IIII", raw[offset:offset + 16])
        offset += 16
        if size > wire or size > 65536:
            raise ValueError("packet witness record length is invalid")
        if offset + size > len(raw):
            incomplete = True
            break
        frame = raw[offset:offset + size]
        offset += size
        start = 14 if link == 1 else 16
        if len(frame) < start:
            continue
        ether_type = int.from_bytes(frame[start - 2:start], "big")
        ip = frame[start:]
        if ether_type != 0x800 or len(ip) < 20 or ip[0] >> 4 != 4 or ip[9] != 17:
            continue
        header = (ip[0] & 15) * 4
        total = int.from_bytes(ip[2:4], "big")
        # Fragmented/truncated packets do not provide a complete payload witness.
        if header < 20 or total > len(ip) or total < header + 8 \
                or int.from_bytes(ip[6:8], "big") & 0x3FFF:
            continue
        source = str(ipaddress.ip_address(ip[12:16]))
        destination = str(ipaddress.ip_address(ip[16:20]))
        udp = ip[header:total]
        source_port, destination_port, udp_size, _ = struct.unpack("!HHHH", udp[:8])
        if not 8 <= udp_size <= len(udp):
            continue
        payload = udp[8:udp_size]
        if len(payload) < 20 or payload[:2] != b"\x00\x01" \
                or payload[4:8] != b"\x21\x12\xa4\x42":
            continue
        length = int.from_bytes(payload[2:4], "big")
        if len(payload) != length + 20 or length % 4:
            continue
        attributes, position = set(), 20
        while position < len(payload):
            kind, attribute_size = struct.unpack("!HH", payload[position:position + 4])
            position += 4 + ((attribute_size + 3) // 4) * 4
            if position > len(payload):
                break
            attributes.add(kind)
        if position != len(payload) or not {0x0006, 0x0008, 0x8028} <= attributes:
            continue
        # Presence identifies ICE-shaped requests, not independent MAC validation.
        # The SDK's authenticated selected RTC/M supplies the authentication proof.
        records.append((source, source_port, destination, destination_port,
                        hashlib.sha256(payload).hexdigest()))
    return records, incomplete


def _capture_paths(captures: dict) -> dict:
    if not isinstance(captures, dict) or set(captures) != {"native", "browser"}:
        raise ValueError("two exact packet witness captures are required")
    paths = {}
    for role, value in captures.items():
        if not isinstance(value, dict) or set(value) != {"path", "sha256"}:
            raise ValueError("packet witness path and SHA256 are required")
        path = Path(value["path"])
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != value["sha256"]:
            raise ValueError("packet witness capture changed")
        paths[role] = path
    return paths


def _ice_requests(path: Path, selected: dict) -> tuple[dict, bool]:
    records, incomplete = _ice_records(path)
    outgoing = (selected["localAddress"], selected["localPort"],
                selected["peerAddress"], selected["peerPort"])
    incoming = (outgoing[2], outgoing[3], outgoing[0], outgoing[1])
    return {"outbound": {v[4] for v in records if v[:4] == outgoing},
            "inbound": {v[4] for v in records if v[:4] == incoming}}, incomplete


def derive_missing_peer_address(captures: dict, native_tuple: dict,
                                browser_local_address: str, browser_local_port: int,
                                browser_peer_port: int) -> str | None:
    """Measure one hidden prflx address from a matching Native request at Pixel."""

    paths = _capture_paths(captures)
    native, _ = _ice_requests(paths["native"], native_tuple)
    browser, _ = _ice_records(paths["browser"])
    addresses = {v[0] for v in browser
                 if v[1] == browser_peer_port
                 and (v[2], v[3]) == (browser_local_address, browser_local_port)
                 and v[4] in native["outbound"]}
    return next(iter(addresses)) if len(addresses) == 1 else None


def correlate_selected_tuples(captures: dict, native_tuple: dict, browser_tuple: dict) -> dict:
    """Match complete request bytes through NAT on both SDK-selected tuples."""

    paths = _capture_paths(captures)
    parsed, private = {}, {}
    for role, selected in (("native", native_tuple), ("browser", browser_tuple)):
        parsed[role], incomplete = _ice_requests(paths[role], selected)
        private[role] = {"captureSha256": captures[role]["sha256"],
                         "incompleteSnapshotTail": incomplete,
                         "outboundIceRequests": len(parsed[role]["outbound"]),
                         "inboundIceRequests": len(parsed[role]["inbound"])}
    native_to_browser = parsed["native"]["outbound"] & parsed["browser"]["inbound"]
    browser_to_native = parsed["browser"]["outbound"] & parsed["native"]["inbound"]
    return {"method": "bidirectional-exact-ICE-payloads-on-SDK-selected-tuples",
            "nativeToBrowserMatchedRequests": len(native_to_browser),
            "browserToNativeMatchedRequests": len(browser_to_native),
            "correlated": bool(native_to_browser and browser_to_native),
            "captures": private, "independentMessageIntegrityValidation": False}
