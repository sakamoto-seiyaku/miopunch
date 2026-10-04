#!/usr/bin/env python3
"""Public UDP Binding fixture regression; not browser/native ICE qualification."""
import binascii
import json
import selectors
import socket
import struct
import subprocess
import sys
import unittest
from pathlib import Path

from stun_udp_observer import binding_response


class BindingFixture(unittest.TestCase):
    def test_udp_attribute_request_reflects_actual_client_tuple_and_fingerprint(self):
        fixture = subprocess.Popen(
            ["rtk", "proxy", sys.executable, str(Path(__file__).with_name("stun_udp_observer.py")),
             "--bind", "127.0.0.1:0"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            selector = selectors.DefaultSelector()
            selector.register(fixture.stdout, selectors.EVENT_READ)
            self.assertTrue(selector.select(3), "owned Binding fixture ready within3s")
            line = fixture.stdout.readline()
            if not line:
                raise RuntimeError("owned Binding fixture exited before ready: " + fixture.stderr.read())
            ready = json.loads(line.split(" ", 1)[1])
            server = tuple(ready["destination"])
            transaction = bytes.fromhex("0102030405060708090a0b0c")
            request = struct.pack("!HHI", 1, 16, 0x2112A442) + transaction
            request += bytes.fromhex("802200046d696f31")
            request += struct.pack("!HHI", 0x8028, 4, binascii.crc32(request) ^ 0x5354554E)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
                client.bind(("127.0.0.1", 0))
                actual = client.getsockname()
                client.settimeout(3)
                client.sendto(request, server)
                response, source = client.recvfrom(2048)
            self.assertEqual(source, server)
            self.assertEqual(response[:2], bytes.fromhex("0101"))
            self.assertEqual(response[8:20], transaction)
            self.assertEqual(response[20:26], bytes.fromhex("002000080001"))
            self.assertEqual(struct.unpack("!H", response[26:28])[0] ^ 0x2112, actual[1])
            decoded = bytes(a ^ b for a, b in zip(response[28:32], bytes.fromhex("2112a442")))
            self.assertEqual(socket.inet_ntoa(decoded), actual[0])
            self.assertEqual(response[32:36], bytes.fromhex("80280004"))
            self.assertEqual(struct.unpack("!I", response[36:40])[0], binascii.crc32(response[:32]) ^ 0x5354554E)
            broken = bytearray(request)
            broken[-1] ^= 1
            self.assertIsNone(binding_response(bytes(broken), actual))
            self.assertIsNone(binding_response(request[:-1], actual))
        finally:
            fixture.terminate()
            try:
                fixture.communicate(timeout=1)
            except subprocess.TimeoutExpired:
                fixture.kill()
                fixture.communicate(timeout=1)


if __name__ == "__main__":
    unittest.main()
