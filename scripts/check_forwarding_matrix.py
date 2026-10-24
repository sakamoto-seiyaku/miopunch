#!/usr/bin/env python3
"""Public forwarding role/scope/ACL/guest/bound checks on real owned sockets."""
import argparse
from contextlib import ExitStack
from pathlib import Path
import socket
import time

from check_forwarding import Node, Runner, Target


def endpoint(text):
    host, port = text.rsplit(":", 1)
    return host.strip("[]"), int(port)


def receive_exact(sock, length):
    result = bytearray()
    while len(result) < length:
        data = sock.recv(length - len(result))
        assert data, (len(result), length)
        result.extend(data)
    return bytes(result)


class MatrixRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.args = args

    def until(self, condition, message, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if condition():
                return
            time.sleep(0.05)
        raise AssertionError(message)

    def start_listener(self, consumer, provider, protocol, target):
        bind = ("[%s]:0" if ":" in self.loopback else "%s:0") % self.loopback
        return consumer.call("forward_start", address=provider.address, protocol=protocol, target=target.address, bind=bind)

    def application(self, forwarding):
        family = socket.AF_INET6 if ":" in self.loopback else socket.AF_INET
        sock = socket.socket(family, socket.SOCK_STREAM if forwarding["protocol"] == "tcp" else socket.SOCK_DGRAM)
        sock.settimeout(75)
        sock.connect(endpoint(forwarding["listen"]))
        return sock

    def target_precondition(self, target):
        with socket.socket(target.family, socket.SOCK_STREAM if target.protocol == "tcp" else socket.SOCK_DGRAM) as probe:
            probe.settimeout(2)
            probe.connect(target.socket.getsockname())
            probe.sendall(b"owned-live-target")
            assert probe.recv(1024) == b"owned-live-target"
        self.record("controller", "independent_target_prerequisite", {"protocol": target.protocol, "loopback": self.loopback, "exact_reply": True})

    def exchange(self, sock, protocol, payload):
        if protocol == "tcp":
            sock.sendall(payload)
            return receive_exact(sock, len(payload))
        sock.send(payload)
        return sock.recv(65536)

    def positive(self, consumer, provider, protocol, target, size=37):
        forwarding = self.start_listener(consumer, provider, protocol, target)
        before = len(target.received)
        payload = bytes((index % 251 for index in range(size)))
        with self.application(forwarding) as app:
            assert self.exchange(app, protocol, payload) == payload
            observed = target.received[before:]
            assert (b"".join(observed) if protocol == "tcp" else observed) == (payload if protocol == "tcp" else [payload])
            status = provider.call("forward_status")
            assert 0 < status["active_flows"] <= status["max_flows"]
        consumer.call("forward_stop", task=forwarding["id"])
        self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "provider flow closes after listener stop")
        return status

    def negative(self, consumer, provider, protocol, target, label):
        forwarding = self.start_listener(consumer, provider, protocol, target)
        before = len(target.received)
        with self.application(forwarding) as app:
            app.settimeout(2)
            try:
                app.send(b"owned-forbidden-effect")
                response = app.recv(65536)
                assert response == b"", "denied operation returned business bytes"
            except (ConnectionResetError, ConnectionRefusedError, BrokenPipeError, socket.timeout):
                pass
            self.until(lambda: next(item for item in consumer.call("forward_status")["listeners"] if item["id"] == forwarding["id"])["counters"]["failed"] > 0,
                       "actual failed forwarding operation must settle")
            assert len(target.received) == before, "unauthorized target had a business effect"
        consumer.call("forward_stop", task=forwarding["id"])
        self.record("controller", label, {"protocol": protocol, "target_unchanged": True, "actual_failed_flow": True})
        # This is the known shared per-peer first/second refused-contact budget,
        # not a retry of the denied operation or an altered product deadline.
        time.sleep(2.05)

    def stopped_effect(self, consumer, provider, protocol, target, mutate, label):
        forwarding = self.start_listener(consumer, provider, protocol, target)
        with self.application(forwarding) as app:
            assert self.exchange(app, protocol, b"owned-before-change") == b"owned-before-change"
            before = len(target.received)
            mutate()
            self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "current authorization must close provider flow")
            app.settimeout(1)
            try:
                app.send(b"owned-after-change")
                assert app.recv(65536) == b"", "stopped flow returned business bytes"
            except (ConnectionResetError, ConnectionRefusedError, BrokenPipeError, socket.timeout):
                pass
            assert len(target.received) == before, "stopped flow reached the target"
        consumer.call("forward_stop", task=forwarding["id"])
        self.record("controller", label, {"protocol": protocol, "active_before": True, "provider_joined_zero": True, "no_later_effect": True})

    def core(self):
        first = Node(self, "first")
        second = self.member(first, "second")
        for direction, (provider, consumer) in enumerate([(first, second), (second, first)]):
            for protocol in ["tcp", "udp"]:
                target = Target(self, protocol, prefix=b"")
                provider.call("forward_enable", protocol=protocol, targets=[target.address])
                if protocol == "tcp":
                    forwarding = self.start_listener(consumer, provider, protocol, target)
                    payload = bytes((index % 251 for index in range(256 * 1024 + 17)))
                    before = len(target.received)
                    with self.application(forwarding) as app:
                        assert self.exchange(app, protocol, payload) == payload
                        app.shutdown(socket.SHUT_WR)
                        assert app.recv(1) == b"", "TCP half-close did not reach the application"
                    assert b"".join(target.received[before:]) == payload
                    consumer.call("forward_stop", task=forwarding["id"])
                else:
                    self.positive(consumer, provider, protocol, target, size=65507)
                self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "role flow cleanup")
                provider.call("forward_disable", protocol=protocol)
                self.record("controller", "provider_role_exchange", {"direction": direction, "protocol": protocol, "target_exact": True, "reply_exact": True})

    def guards(self):
        admin = Node(self, "admin")
        provider = self.member(admin, "provider")
        third = self.member(admin, "third")
        admin.call("service_sync", address=provider.address)
        outsider = Node(self, "outside-network")
        for protocol in ["tcp", "udp"]:
            service = protocol + "_forward"
            allowed = Target(self, protocol, prefix=b"")
            forbidden = Target(self, protocol, prefix=b"")
            self.target_precondition(allowed)
            self.target_precondition(forbidden)
            self.negative(admin, provider, protocol, allowed, "service_off_has_no_target_effect")
            provider.call("forward_enable", protocol=protocol, targets=[allowed.address])
            self.positive(admin, provider, protocol, allowed)
            self.negative(admin, provider, protocol, forbidden, "target_outside_scope_has_no_effect")
            outsider.call("forward_start", ok=False, address=provider.address, protocol=protocol, target=allowed.address, bind="127.0.0.1:0")
            deny = dict(members=True, guests=True, allowed=[], denied=[admin.identity])
            revisions = []
            self.stopped_effect(admin, provider, protocol, allowed,
                                lambda: revisions.append(provider.call("service_rule", provider=provider.identity, service=service, previous=None, rule=deny)["revision"]),
                                "local_acl_revokes_active_flow")
            self.negative(admin, provider, protocol, allowed, "current_acl_denies_new_effect")
            self.positive(third, provider, protocol, allowed)
            admin.call("service_sync", address=provider.address)
            current = next(item for item in admin.call("service_status")["policies"] if item["provider"] == provider.identity and item["service"] == service)
            assert current["revision"] == revisions[0], "remote editor must verify the exact signed predecessor first"
            restored = admin.call("service_rule", provider=provider.identity, service=service, previous=revisions[0],
                                  rule=dict(members=True, guests=True, allowed=[], denied=[]))["revision"]
            admin.call("service_sync", address=provider.address)
            policy = next(item for item in provider.call("service_status")["policies"] if item["provider"] == provider.identity and item["service"] == service)
            assert policy["revision"] == restored
            self.positive(admin, provider, protocol, allowed)
            self.stopped_effect(admin, provider, protocol, allowed,
                                lambda: provider.call("forward_enable", protocol=protocol, targets=[forbidden.address]),
                                "scope_replacement_revokes_active_target")
            self.negative(admin, provider, protocol, allowed, "old_target_scope_not_restored_by_listener")
            self.positive(admin, provider, protocol, forbidden)
            self.stopped_effect(admin, provider, protocol, forbidden,
                                lambda: provider.call("forward_disable", protocol=protocol), "disable_revokes_active_flow")

    def bounds(self):
        provider = Node(self, "provider")
        consumer = self.member(provider, "consumer")
        target = Target(self, "udp", prefix=b"")
        self.target_precondition(target)
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        forwards = [self.start_listener(consumer, provider, "udp", target) for _ in range(8)]
        consumer.call("forward_start", ok=False, address=provider.address, protocol="udp", target=target.address, bind="127.0.0.1:0")
        forwarding = forwards[0]
        with ExitStack() as stack:
            clients = []
            for index in range(16):
                app = stack.enter_context(self.application(forwarding))
                payload = bytes([index]) * 17
                assert self.exchange(app, "udp", payload) == payload
                clients.append(app)
            before = len(target.received)
            excess = stack.enter_context(self.application(forwarding))
            excess.settimeout(1)
            excess.send(b"owned-seventeenth-source")
            try:
                excess.recv(65536)
                raise AssertionError("seventeenth flow unexpectedly succeeded")
            except socket.timeout:
                pass
            assert len(target.received) == before
            status = consumer.call("forward_status")
            assert status["active_flows"] == status["max_flows"] == 16
            assert provider.call("forward_status")["active_flows"] == 16
            counters = next(item["counters"] for item in status["listeners"] if item["id"] == forwarding["id"])
            assert counters["dropped_datagrams"] >= 1
            burst = [bytes([70 + index]) * (31 + index) for index in range(8)]
            for payload in burst:
                clients[0].send(payload)
            assert [clients[0].recv(65536) for _ in burst] == burst
            assert target.received[before:] == burst
            for app in clients[1:]:
                app.settimeout(0.05)
                try:
                    app.recv(65536)
                    raise AssertionError("reply leaked into another UDP source")
                except socket.timeout:
                    pass
        for forwarding in forwards:
            consumer.call("forward_stop", task=forwarding["id"])
        assert consumer.call("forward_status")["active_flows"] == 0
        self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "all sixteen remote associations joined")
        self.record("controller", "actual_bounds_and_datagram_isolation", {"listeners": 8, "ninth_rejected": True, "flows": 16,
                    "seventeenth_has_no_effect": True, "existing_source_still_works_at_capacity": True, "burst_exact": True, "cross_source_reply": False})

    def scenario(self):
        self.freeze_source(Path(__file__))
        if getattr(self.args, "relay_binary", None):
            from forwarding_local_resources import configure_local_resources
            configure_local_resources(self, self.args)
        getattr(self, self.case)()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["core", "guards", "bounds"], required=True)
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--relay-binary", type=Path)
    parser.add_argument("--relay-receipt", type=Path)
    MatrixRunner(parser.parse_args()).run()
