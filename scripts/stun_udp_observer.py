#!/usr/bin/env python3
"""Owned RFC 8489 UDP Binding fixture; reflects the actual recvfrom tuple."""
import argparse
import binascii
import json
import os
import selectors
import signal
import socket
import struct

COOKIE = 0x2112A442
FINGERPRINT = 0x8028
MAX_REQUEST = 2048


def attributes(request):
    """Validate the bounded Binding envelope/TLVs without recording credentials."""
    if len(request) < 20 or len(request) > MAX_REQUEST:
        return None
    kind, length, cookie = struct.unpack("!HHI", request[:8])
    if kind != 1 or cookie != COOKIE or length % 4 or length + 20 != len(request):
        return None
    result = []
    position = 20
    while position < len(request):
        if position + 4 > len(request):
            return None
        kind, length = struct.unpack("!HH", request[position:position + 4])
        end = position + 4 + length
        padded_end = end + (-length % 4)
        if padded_end > len(request):
            return None
        if kind == FINGERPRINT:
            if length != 4 or padded_end != len(request):
                return None
            fingerprint = struct.unpack("!I", request[position + 4:end])[0]
            if fingerprint != (binascii.crc32(request[:position]) ^ 0x5354554E):
                return None
        result.append((kind, length))
        position = padded_end
    return result


def binding_response(request, peer):
    """Return an RFC8489 response for the actual recvfrom tuple, or reject it."""
    parsed = attributes(request)
    if parsed is None:
        return None
    # Only SOFTWARE/FINGERPRINT are understood; other optional TLVs are ignored.
    # Unknown comprehension-required attributes receive a bounded 420 response.
    unknown = sorted({kind for kind, _ in parsed if kind < 0x8000})
    if unknown:
        reason = b"Unknown Attribute"
        error = bytes([0, 0, 4, 20]) + reason
        body = struct.pack("!HH", 9, len(error)) + error + bytes(-len(error) % 4)
        values = b"".join(struct.pack("!H", kind) for kind in unknown)
        body += struct.pack("!HH", 10, len(values)) + values + bytes(-len(values) % 4)
        kind = 0x0111
    else:
        body = struct.pack("!HHBBH", 0x0020, 8, 0, 1, peer[1] ^ 0x2112)
        body += bytes(a ^ b for a, b in zip(socket.inet_aton(peer[0]), request[4:8]))
        kind = 0x0101
    fingerprint = any(kind == FINGERPRINT for kind, _ in parsed)
    response = struct.pack("!HH", kind, len(body) + (8 if fingerprint else 0)) + request[4:20] + body
    if fingerprint:
        response += struct.pack("!HHI", FINGERPRINT, 4, binascii.crc32(response) ^ 0x5354554E)
    return response


def main():
    def stopped(signum, frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, stopped)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind", action="append", required=True)
    args = parser.parse_args()
    selector = selectors.DefaultSelector()
    for bind in args.bind:
        host, port = bind.rsplit(":", 1)
        server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        server.bind((host, int(port)))
        selector.register(server, selectors.EVENT_READ)
        print("STUN_UDP_READY " + json.dumps({"destination": server.getsockname(), "pid": os.getpid()}), flush=True)
    while True:
        for key, _ in selector.select():
            server = key.fileobj
            request, peer = server.recvfrom(MAX_REQUEST + 1)
            reply = binding_response(request, peer)
            if reply is None:
                continue
            server.sendto(reply, peer)
            print(("STUN_UDP_BINDING " if reply[:2] == b"\x01\x01" else "STUN_UDP_ERROR ") + json.dumps({
                "destination": server.getsockname(), "actualSource": peer, "transaction": request[8:20].hex(),
                "requestBytes": len(request), "responseBytes": len(reply), "attributes": attributes(request),
            }), flush=True)


if __name__ == "__main__":
    main()
