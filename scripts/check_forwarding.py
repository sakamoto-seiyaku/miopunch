#!/usr/bin/env python3
"""Real public forwarding operations, with independent owned socket observers.

Consumes immutable pins and never builds. Only newly created loopback sockets,
node directories and processes belong to this controller. No desktop context.
"""
import argparse
import ipaddress
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import socket
import subprocess
import tempfile
import threading
import time


def isolated_environment():
    blocked = {"DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "XAUTHORITY", "MIR_SOCKET",
               "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS", "SESSION_MANAGER",
               "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"}
    return {key: value for key, value in os.environ.items() if key not in blocked}


class Target:
    def __init__(self, runner, protocol, prefix=b"reply:"):
        self.protocol = protocol
        self.prefix = prefix
        self.family = socket.AF_INET6 if ":" in runner.loopback else socket.AF_INET
        self.socket = socket.socket(self.family, socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM)
        self.socket.bind((runner.loopback, 0))
        host, port = self.socket.getsockname()[:2]
        self.address = ("[%s]:%s" if self.family == socket.AF_INET6 else "%s:%s") % (host, port)
        if protocol == "tcp":
            self.socket.listen(16)
        self.socket.settimeout(0.1)
        self.stop = threading.Event()
        self.received = []
        self.connections = []
        self.errors = []
        self.reader = threading.Thread(target=self.run, daemon=True)
        runner.targets.append(self)
        self.reader.start()

    def run(self):
        try:
            if self.protocol == "udp":
                while not self.stop.is_set():
                    try:
                        data, address = self.socket.recvfrom(65536)
                    except socket.timeout:
                        continue
                    self.received.append(data)
                    self.socket.sendto(self.prefix + data, address)
            else:
                while not self.stop.is_set():
                    try:
                        connection, _ = self.socket.accept()
                    except socket.timeout:
                        continue
                    self.connections.append(connection)
                    connection.settimeout(0.1)
                    with connection:
                        while not self.stop.is_set():
                            try:
                                data = connection.recv(65536)
                            except socket.timeout:
                                continue
                            if not data:
                                break
                            self.received.append(data)
                            connection.sendall(self.prefix + data)
        except BaseException as failure:
            self.errors.append(type(failure).__name__)

    def close(self):
        self.stop.set()
        self.reader.join(timeout=3)
        terminal = not self.reader.is_alive()
        self.socket.close()
        assert terminal and not self.errors, (terminal, self.errors)
        return {"protocol": self.protocol, "reader_joined": terminal, "errors": self.errors}


class Node:
    def __init__(self, runner, label, action="create", invitation=None, directory=None):
        self.runner, self.label = runner, label
        self.directory = directory or str(runner.private / label)
        self.stderr = (runner.output / (label + ".stderr")).open("w")
        configuration = getattr(runner, "native_configuration", None)
        self.process = subprocess.Popen([str(runner.binary), action, self.directory] + ([str(invitation)] if invitation else []) + ([str(configuration)] if configuration and action in {"create", "join", "join-guest"} else []),
                                        env=isolated_environment(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True)
        self.queue = queue.Queue(maxsize=128)
        self.reader_error = None
        self.reader = threading.Thread(target=self.read, daemon=True)
        runner.nodes.append(self)
        self.reader.start()
        ready = self.receive()
        assert ready["event"] == "ready", ready
        self.identity = ready["status"]["node"]
        self.address = ready["address"]
        if ready["status"]["stage"] == "AwaitingRecoverySave":
            self.call("export_recovery_card", path=str(runner.private / (label + "-card")))
            self.call("acknowledge_recovery_saved")

    def read(self):
        try:
            for line in self.process.stdout:
                if line.strip():
                    self.queue.put(json.loads(line), timeout=2)
        except BaseException as failure:
            self.reader_error = type(failure).__name__
        finally:
            try:
                self.queue.put(None, timeout=2)
            except queue.Full:
                self.reader_error = "QueueFull"

    def receive(self):
        deadline = time.monotonic() + 80
        while True:
            value = self.queue.get(timeout=max(0, deadline - time.monotonic()))
            assert value is not None, "node exited before response"
            if value.get("event") not in (None, "ready"):
                continue
            return value

    def call(self, operation, ok=True, **arguments):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, op=operation, **arguments)) + "\n")
        self.process.stdin.flush()
        value = self.receive()
        self.runner.record(self.label, operation, value)
        assert value.get("id") == identifier and value["ok"] == ok, value
        return value.get("result")

    def close(self):
        errors = []
        try:
            if self.process.poll() is None:
                self.call("stop")
        except BaseException as failure:
            errors.append(type(failure).__name__)
        try:
            self.process.stdin.close()
        except BaseException as failure:
            errors.append(type(failure).__name__)
        try:
            self.process.wait(timeout=20)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        self.reader.join(timeout=3)
        self.stderr.close()
        self.cleanup = {"node": self.label, "exit": self.process.poll(), "reader_joined": not self.reader.is_alive(),
                        "reader_error": self.reader_error, "errors": errors}
        assert self.process.poll() == 0 and not self.reader.is_alive() and not self.reader_error and not errors, self.cleanup
        return self.cleanup

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive()


class Runner:
    def __init__(self, args):
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.binary = args.binary.resolve()
        assert hashlib.sha256(self.binary.read_bytes()).hexdigest() == args.sha256
        self.private = Path(tempfile.mkdtemp(prefix="m118-forward-"))
        (self.private / ".owner").write_text(str(self.output))
        self.nodes, self.targets, self.sequence = [], [], 0
        self.controller_sources = {}
        self.freeze_source(Path(__file__))
        self.case = args.case
        self.loopback = getattr(args, "loopback", "127.0.0.1")
        assert ipaddress.ip_address(self.loopback).is_loopback
        self.record("controller", "pin", {"sha256": args.sha256, "source_receipt": args.source_receipt,
                                          "desktop_context_removed": True})

    def freeze_source(self, path):
        path = Path(path).resolve()
        self.controller_sources[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.output / "controller-start.json").write_text(json.dumps(self.controller_sources, indent=2) + "\n")

    def record(self, node, operation, result):
        with (self.output / "events.jsonl").open("a") as log:
            log.write(json.dumps(dict(node=node, operation=operation, result=result, monotonic=time.monotonic())) + "\n")

    def member(self, creator, label):
        invitation = self.private / (label + "-invitation")
        creator.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        member = Node(self, label, "join", invitation)
        member.call("request", path=str(invitation))
        creator.call("service_sync", address=member.address)
        return member

    def smoke(self):
        target = Target(self, "tcp")
        # Independent direct observation proves this run's target is live.
        with socket.create_connection(tuple(target.socket.getsockname()[:2]), timeout=2) as probe:
            probe.sendall(b"owned-target-precondition")
            assert probe.recv(1024) == b"reply:owned-target-precondition"
        provider = Node(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("forward_enable", protocol="tcp", targets=[target.address])
        forwarding = consumer.call("forward_start", address=provider.address, protocol="tcp", target=target.address, bind="127.0.0.1:0")
        listen = forwarding["listen"].rsplit(":", 1)
        with socket.create_connection((listen[0], int(listen[1])), timeout=5) as client:
            client.sendall(b"owned-forward-payload")
            assert client.recv(1024) == b"reply:owned-forward-payload"
        assert b"owned-forward-payload" in target.received
        self.record("controller", "independent_socket_effect", {"protocol": "tcp", "target_observed": True, "reply_exact": True})
        consumer.call("forward_stop", task=forwarding["id"])

    def udp(self):
        target = Target(self, "udp", prefix=b"")
        with socket.socket(target.family, socket.SOCK_DGRAM) as probe:
            probe.settimeout(2)
            probe.sendto(b"owned-udp-precondition", target.socket.getsockname())
            assert probe.recv(65536) == b"owned-udp-precondition"
        provider = Node(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        forwarding = consumer.call("forward_start", address=provider.address, protocol="udp", target=target.address, bind=("[%s]:0" if target.family == socket.AF_INET6 else "%s:0") % self.loopback)
        host, port = forwarding["listen"].rsplit(":", 1)
        with socket.socket(target.family, socket.SOCK_DGRAM) as client:
            client.settimeout(10)
            client.connect((host.strip("[]"), int(port)))
            for length in [0, 17, 65000, 65507]:
                payload = bytes((index % 251 for index in range(length)))
                before = len(target.received)
                client.send(payload)
                response = client.recv(65536)
                assert response == payload, (length, len(response))
                assert target.received[before:] == [payload], length
                self.record("controller", "independent_udp_datagram", {"length": length, "target_exact": True, "reply_exact": True})
        consumer.call("forward_stop", task=forwarding["id"])

    def scenario(self):
        getattr(self, self.case)()

    def run(self):
        started = time.monotonic()
        report = {"status": "failed"}
        primary = None
        try:
            self.scenario()
            report["status"] = "pass"
        except BaseException as failure:
            primary = failure
            report.update(failure_type=type(failure).__name__, failure=str(failure))
        cleanup, errors = [], []
        for owner in list(reversed(self.nodes)) + list(reversed(self.targets)):
            try:
                cleanup.append(owner.close())
            except BaseException as failure:
                errors.append(type(failure).__name__)
                cleanup.append(getattr(owner, "cleanup", {"owner": type(owner).__name__, "failed": True}))
        terminal = all(node.terminal() for node in self.nodes) and all(not target.reader.is_alive() for target in self.targets)
        if terminal and not errors:
            try:
                assert (self.private / ".owner").read_text() == str(self.output)
                shutil.rmtree(self.private)
            except BaseException as failure:
                errors.append(type(failure).__name__)
        if errors or not terminal or self.private.exists():
            report["status"] = "failed"
        report.update(seconds=time.monotonic() - started, cleanup=cleanup, cleanup_errors=errors,
                      all_owned_terminal=terminal, private_state_removed=not self.private.exists())
        end = {path: hashlib.sha256(Path(path).read_bytes()).hexdigest() for path in self.controller_sources}
        report["controller_drift"] = [path for path, digest in end.items() if digest != self.controller_sources[path]]
        if report["controller_drift"]:
            report["status"] = "failed"
        (self.output / "controller-end.json").write_text(json.dumps(end, indent=2) + "\n")
        (self.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        if primary:
            raise primary
        assert report["status"] == "pass", report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["smoke", "udp"], default="smoke")
    parser.add_argument("--loopback", default="127.0.0.1")
    Runner(parser.parse_args()).run()
