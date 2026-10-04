#!/usr/bin/env python3
"""Protocol-free parser regressions; synthetic packets are never runtime evidence."""
import struct
import tempfile
import unittest
from pathlib import Path
from rtc_cross_network_evidence import packets
from rtc_cross_network_process import public_event


class EvidenceTests(unittest.TestCase):
    def test_ipv4_control_packets_are_counted_and_truncation_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"unit.pcap"
            header = struct.pack("<IHHIIII", 0xa1b2c3d4, 2, 4, 0, 0, 256, 1)
            # ICMP belongs to IPv4 capture totals despite having no UDP/TCP ports.
            frame = bytes(12)+b"\x08\x00"+b"\x45\x00\x00\x1c\x00\x00\x00\x00\x40\x01\x00\x00"+bytes([10, 0, 0, 1, 10, 0, 0, 2])+bytes(8)
            record = struct.pack("<IIII", 1, 0, len(frame), len(frame))
            path.write_bytes(header+record+frame)
            rows = list(packets(path))
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["tuple"], ("10.0.0.1", "10.0.0.2", 0, 0, 1))
            self.assertEqual(rows[0]["ipBytes"], 28)
            path.write_bytes(header+record+frame[:-1])
            with self.assertRaisesRegex(ValueError, "truncated pcap frame"):
                list(packets(path))

    def test_private_material_is_omitted_from_nested_public_events(self):
        value = {"event": "test", "code": "private", "invitation": "private", "nested": {
            "usernameFragment": "private", "sdp": "private", "fingerprint": "private", "bytesSent": 36}}
        self.assertEqual(public_event(value), {"event": "test", "nested": {"bytesSent": 36}})


if __name__ == "__main__":
    unittest.main()
