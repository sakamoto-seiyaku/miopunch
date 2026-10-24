#!/usr/bin/env python3
"""Real EOF-triggered TCP replies and per-packet UDP oversize isolation."""
import argparse
from pathlib import Path
import socket

from check_forwarding import Node, Target
from check_forwarding_matrix import MatrixRunner, receive_exact
from forwarding_local_resources import configure_local_resources


class EofTarget(Target):
    def __init__(self, runner, response):
        self.response = response
        self.completed = 0
        super().__init__(runner, "tcp", prefix=b"")

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    connection, _ = self.socket.accept()
                except socket.timeout:
                    continue
                self.connections.append(connection)
                with connection:
                    connection.settimeout(0.1)
                    request = bytearray()
                    while not self.stop.is_set():
                        try:
                            data = connection.recv(65536)
                        except socket.timeout:
                            continue
                        if not data:
                            self.received.append(bytes(request))
                            connection.settimeout(10)
                            connection.sendall(self.response)
                            connection.shutdown(socket.SHUT_WR)
                            self.completed += 1
                            break
                        request.extend(data)
        except BaseException as failure:
            self.errors.append(type(failure).__name__)


class CompletionRunner(MatrixRunner):
    def eof_reply(self):
        first = Node(self, "first")
        second = self.member(first, "second")
        request = bytes(index % 251 for index in range(128 * 1024 + 17))
        response = bytes((index + 41) % 251 for index in range(512 * 1024 + 17))
        for direction, (provider, consumer) in enumerate([(first, second), (second, first)]):
            target = EofTarget(self, response)
            with socket.socket(target.family, socket.SOCK_STREAM) as direct:
                direct.settimeout(5)
                direct.connect(target.socket.getsockname())
                direct.sendall(request)
                direct.shutdown(socket.SHUT_WR)
                assert receive_exact(direct, len(response)) == response
                assert direct.recv(1) == b""
            assert target.received == [request] and target.completed == 1
            self.record("controller", "eof_target_premise", {"direction": direction, "request_exact": True,
                        "reply_only_after_eof": True, "reply_length": len(response), "direct_exact": True})
            provider.call("forward_enable", protocol="tcp", targets=[target.address])
            forwarding = self.start_listener(consumer, provider, "tcp", target)
            reply = bytearray()
            failure = None
            with self.application(forwarding) as app:
                app.sendall(request)
                app.shutdown(socket.SHUT_WR)
                try:
                    while True:
                        data = app.recv(65536)
                        if not data:
                            break
                        reply.extend(data)
                        assert len(reply) <= len(response), "unexpected extra TCP response"
                except (ConnectionResetError, ConnectionAbortedError, socket.timeout) as error:
                    failure = type(error).__name__
            self.record("controller", "eof_triggered_response", {"direction": direction, "reply_length": len(reply),
                        "expected_length": len(response), "reply_exact": reply == response, "failure": failure,
                        "target_requests": len(target.received), "target_response_completed": target.completed})
            assert failure is None and reply == response, "response-after-half-close must arrive exactly through clean EOF"
            assert target.received == [request, request] and target.completed == 2
            consumer.call("forward_stop", task=forwarding["id"])
            self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "EOF provider flow closes")

    def udp_oversize(self):
        provider = Node(self, "provider")
        consumer = self.member(provider, "consumer")
        target = Target(self, "udp", prefix=b"")
        oversized = bytes(index % 251 for index in range(65508))
        with socket.socket(target.family, socket.SOCK_DGRAM) as direct:
            direct.settimeout(2)
            direct.connect(target.socket.getsockname())
            assert direct.send(oversized) == len(oversized)
            assert direct.recv(65536) == oversized
        self.record("controller", "oversize_direct_ipv6_premise", {"length": len(oversized), "direct_exact": True})
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        forwarding = self.start_listener(consumer, provider, "udp", target)
        with self.application(forwarding) as app:
            assert self.exchange(app, "udp", b"before-oversize") == b"before-oversize"
            before = len(target.received)
            assert app.send(oversized) == len(oversized)
            def settled():
                listener = next(item for item in consumer.call("forward_status")["listeners"] if item["id"] == forwarding["id"])
                return listener["counters"]["dropped_datagrams"] > 0 or not listener["running"]
            self.until(settled, "oversized packet must settle")
            status = consumer.call("forward_status")
            listener = next(item for item in status["listeners"] if item["id"] == forwarding["id"])
            self.record("controller", "oversize_listener_observation", {"running": listener["running"],
                        "dropped": listener["counters"]["dropped_datagrams"], "target_unchanged": len(target.received) == before})
            assert listener["running"] and listener["counters"]["dropped_datagrams"] == 1, "one oversized packet must not retire its listener"
            assert len(target.received) == before
            assert self.exchange(app, "udp", b"after-oversize") == b"after-oversize"
            assert target.received[before:] == [b"after-oversize"]
        consumer.call("forward_stop", task=forwarding["id"])
        self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "UDP provider flow closes")

    def scenario(self):
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(__file__).with_name("check_forwarding_matrix.py"))
        configure_local_resources(self, self.args)
        getattr(self, self.case)()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--case", choices=["eof_reply", "udp_oversize"], required=True)
    args = parser.parse_args()
    runner = CompletionRunner(args)
    runner.args = args
    runner.run()
