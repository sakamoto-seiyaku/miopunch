#!/usr/bin/env python3
"""Owned dual-stack STUN-over-TCP Binding service; address comes from the real socket."""
import asyncio
import argparse
import ipaddress
import struct


async def binding(reader, writer):
    try:
        header = await asyncio.wait_for(reader.readexactly(20), 2)
        kind, length, cookie = struct.unpack("!HHI", header[:8])
        if (kind, length, cookie) != (1, 0, 0x2112A442):
            return
        address, port = writer.get_extra_info("peername")[:2]
        ip = ipaddress.ip_address(address)
        mask = header[4:20]
        encoded = bytes(byte ^ mask[index] for index, byte in enumerate(ip.packed))
        value = struct.pack("!BBH", 0, 1 if ip.version == 4 else 2, port ^ 0x2112) + encoded
        attribute = struct.pack("!HH", 0x0020, len(value)) + value
        response = struct.pack("!HHI", 0x0101, len(attribute), cookie) + header[8:] + attribute
        writer.write(response)
        await writer.drain()
        print(f"STUN_TCP_OBSERVED candidate={address}:{port}", flush=True)
        await asyncio.wait_for(reader.read(1), 20)
    except (OSError, asyncio.IncompleteReadError, asyncio.TimeoutError):
        pass
    finally:
        writer.close()
        await writer.wait_closed()


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="100.64.27.1")
    args = parser.parse_args()
    server = await asyncio.start_server(binding, args.host, 3478)
    print("STUN_TCP_READY actual_socket_observer=true", flush=True)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
