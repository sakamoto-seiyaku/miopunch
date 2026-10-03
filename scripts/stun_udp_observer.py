#!/usr/bin/env python3
"""Owned RFC 8489 UDP Binding fixture; reflects the actual recvfrom tuple."""
import argparse
import selectors
import socket
import struct


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind", action="append", required=True)
    args = parser.parse_args()
    selector = selectors.DefaultSelector()
    for bind in args.bind:
        host, port = bind.rsplit(":", 1)
        server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        server.bind((host, int(port)))
        selector.register(server, selectors.EVENT_READ)
        print(f"STUN_UDP_READY destination={host}:{port}", flush=True)
    while True:
        for key, _ in selector.select():
            server = key.fileobj
            request, peer = server.recvfrom(2048)
            if len(request) != 20 or request[:8] != bytes.fromhex("000100002112a442"):
                continue
            mapped = struct.pack("!HHBBH", 0x0020, 8, 0, 1, peer[1] ^ 0x2112)
            mapped += bytes(a ^ b for a, b in zip(socket.inet_aton(peer[0]), request[4:8]))
            reply = struct.pack("!HH", 0x0101, len(mapped)) + request[4:20] + mapped
            server.sendto(reply, peer)
            print(f"STUN_UDP_BINDING destination={server.getsockname()} actual_source={peer} transaction={request[8:20].hex()}", flush=True)


if __name__ == "__main__":
    main()
