#!/usr/bin/env python3
"""Owned IPv4 STUN-over-TCP Binding service; address comes from the real socket."""
import asyncio
import ipaddress
import struct


async def binding(reader, writer):
    try:
        header = await asyncio.wait_for(reader.readexactly(20), 2)
        kind, length, cookie = struct.unpack("!HHI", header[:8])
        if (kind, length, cookie) != (1, 0, 0x2112A442):
            return
        address, port = writer.get_extra_info("peername")[:2]
        value = struct.pack("!BBHI", 0, 1, port ^ 0x2112, int(ipaddress.IPv4Address(address)) ^ cookie)
        response = struct.pack("!HHI", 0x0101, 12, cookie) + header[8:] + struct.pack("!HH", 0x0020, 8) + value
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
    server = await asyncio.start_server(binding, "100.64.27.1", 3478)
    print("STUN_TCP_READY actual_socket_observer=true", flush=True)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
