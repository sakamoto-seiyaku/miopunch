#!/usr/bin/env python3
"""Observe the original third-member operation without resending its payload."""
import argparse
from pathlib import Path
import socket
import time
from check_forwarding_matrix import MatrixRunner


class GuardDiagnostic(MatrixRunner):
    def positive(self, consumer, provider, protocol, target, size=37):
        if consumer.label != "third":
            return super().positive(consumer, provider, protocol, target, size)
        self.target_precondition(target)
        self.record("controller", "third_member_actual_premises", {"consumer": consumer.call("status"), "provider": provider.call("status"),
                    "effective_policy": provider.call("service_status")})
        forwarding = self.start_listener(consumer, provider, protocol, target)
        payload = bytes((index % 251 for index in range(size)))
        before = len(target.received)
        started = time.monotonic()
        with self.application(forwarding) as app:
            app.settimeout(10)
            app.send(payload)
            try:
                response = app.recv(65536)
            except socket.timeout:
                self.record("controller", "original_ten_second_pending_observation", {"elapsed": time.monotonic() - started,
                            "target_lengths": [len(data) for data in target.received[before:]],
                            "provider": provider.call("forward_status"), "consumer": consumer.call("forward_status")})
                # Continue reading the exact original socket operation within
                # the SDK's existing 60s setup budget; never send another byte.
                app.settimeout(65)
                response = app.recv(65536)
            assert response == payload
            assert (b"".join(target.received[before:]) if protocol == "tcp" else target.received[before:]) == (payload if protocol == "tcp" else [payload])
            status = provider.call("forward_status")
        consumer.call("forward_stop", task=forwarding["id"])
        self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "diagnostic flow cleanup")
        self.record("controller", "same_original_operation_settled", {"elapsed": time.monotonic() - started, "protocol": protocol, "exact": True})
        return status

    def scenario(self):
        self.freeze_source(Path(__file__))
        super().scenario()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", default="guards")
    parser.add_argument("--loopback", default="::1")
    GuardDiagnostic(parser.parse_args()).run()
