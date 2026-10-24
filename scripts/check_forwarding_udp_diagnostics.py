#!/usr/bin/env python3
"""Retain exact large-datagram premises and public failure observations."""
import argparse
from pathlib import Path
import socket
from check_forwarding import Runner, Node, Target


class DiagnosticRunner(Runner):
    def scenario(self):
        self.freeze_source(Path(__file__))
        target = Target(self, "udp", prefix=b"")
        provider = Node(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        forwarding = consumer.call("forward_start", address=provider.address, protocol="udp", target=target.address, bind="127.0.0.1:0")
        host, port = forwarding["listen"].rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            client.settimeout(10)
            client.connect((host, int(port)))
            for length in [0, 17, 65000, 65507]:
                payload = bytes((index % 251 for index in range(length)))
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as direct:
                    direct.settimeout(2)
                    direct.sendto(payload, target.socket.getsockname())
                    assert direct.recv(65536) == payload
                self.record("controller", "direct_exact_datagram_prerequisite", {"length": length, "observed": True})
                before = len(target.received)
                try:
                    client.send(payload)
                    response = client.recv(65536)
                    assert response == payload, (length, len(response))
                    assert target.received[before:] == [payload], length
                finally:
                    self.record("controller", "throughforward_datagram_observation", {"length": length,
                                "target_lengths": [len(data) for data in target.received[before:]],
                                "provider": provider.call("forward_status"), "consumer": consumer.call("forward_status")})
        consumer.call("forward_stop", task=forwarding["id"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", default="diagnostic")
    DiagnosticRunner(parser.parse_args()).run()
