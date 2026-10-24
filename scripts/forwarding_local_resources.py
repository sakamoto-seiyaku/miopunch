#!/usr/bin/env python3
"""Owned loopback relay and persisted signal ownership for forwarding checks."""
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import threading
import time
from urllib.parse import urlsplit

from check_forwarding import isolated_environment


def identity(pid):
    root = Path("/proc") / str(pid)
    stat = (root / "stat").read_text()
    return {"pid": pid, "start_ticks": stat[stat.rfind(")") + 2:].split()[19],
            "cmdline_sha256": hashlib.sha256((root / "cmdline").read_bytes()).hexdigest()}


class OwnedProcess:
    """Only a registered Popen child; record intent before every possible signal."""
    def __init__(self, process, receipt):
        self.process, self.receipt = process, Path(receipt)
        self.expected = identity(process.pid)
        self.pidfd = os.pidfd_open(process.pid) if hasattr(os, "pidfd_open") else None
        self.record("registered", identity=self.expected, pidfd=self.pidfd is not None)

    def record(self, phase, **fields):
        with self.receipt.open("a") as output:
            output.write(json.dumps(dict(phase=phase, monotonic=time.monotonic(), **fields)) + "\n")
            output.flush()
            os.fsync(output.fileno())

    def send(self, number):
        if self.process.poll() is not None:
            self.record("already_terminal", exit=self.process.returncode)
            return
        current = identity(self.process.pid)
        if current != self.expected:
            self.record("refused_identity_change", expected=self.expected, observed=current, signal=number)
            raise RuntimeError("owned child identity changed; no signal sent")
        self.record("signal_intent", identity=self.expected, signal=number, pidfd=self.pidfd is not None)
        try:
            if self.pidfd is not None:
                signal.pidfd_send_signal(self.pidfd, number)
            else:
                self.process.send_signal(number)
            self.record("signal_sent", signal=number)
        except ProcessLookupError:
            self.record("exited_before_signal", signal=number)

    def close(self):
        if self.process.poll() is None:
            raise RuntimeError("cannot retire live owned child")
        if self.pidfd is not None:
            os.close(self.pidfd)
            self.pidfd = None
        self.record("retired", exit=self.process.returncode)


class LoopbackRelay:
    def __init__(self, runner, binary, receipt):
        build = json.loads(Path(receipt).read_text())
        binary = Path(binary).resolve()
        assert build["exit"] == 0 and build["binary"] == "browser-probe-native" and str(binary) == build["pin"]
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["sha256"]
        runner.freeze_source(Path(__file__))
        self.stderr = (runner.output / "local-relay.stderr").open("w")
        self.process = subprocess.Popen([str(binary), "relay", "0"], env=isolated_environment(),
                                        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.reader_error = None
        self.owner = OwnedProcess(self.process, runner.output / "local-relay-owner.jsonl")
        self.queue = queue.Queue(maxsize=32)
        self.reader = threading.Thread(target=self.read, daemon=True)
        runner.nodes.append(self)
        self.reader.start()
        ready = self.queue.get(timeout=10)
        assert ready is not None and ready.startswith("RELAY ")
        self.url = "http://" + ready.split()[1] + "/"
        parts = urlsplit(self.url)
        assert parts.scheme == "http" and ipaddress.ip_address(parts.hostname).is_loopback and parts.port
        runner.record("controller", "owned_local_relay", {"origin": self.url, "pin_sha256": build["sha256"],
                      "source_receipt": str(receipt), "external_relay": False, "proxy_to_external": False})

    def read(self):
        try:
            for line in self.process.stdout:
                self.queue.put(line.strip(), timeout=2)
        except BaseException as failure:
            self.reader_error = type(failure).__name__

    def close(self):
        errors = []
        try:
            self.owner.send(signal.SIGINT)
            self.process.wait(timeout=10)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        if self.process.poll() is None:
            try:
                self.owner.send(signal.SIGTERM)
                self.process.wait(timeout=5)
            except BaseException as failure:
                errors.append(type(failure).__name__)
        self.reader.join(timeout=3)
        self.stderr.close()
        if self.process.poll() is not None:
            self.owner.close()
        self.cleanup = {"owner": "local-relay", "exit": self.process.poll(), "reader_joined": not self.reader.is_alive(),
                        "reader_error": self.reader_error, "errors": errors}
        assert self.process.poll() == 0 and not self.reader.is_alive() and not self.reader_error and not errors, self.cleanup
        return self.cleanup

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive() and self.owner.pidfd is None


def configure_local_resources(runner, args):
    relay = LoopbackRelay(runner, args.relay_binary, args.relay_receipt)
    resource = {"url": relay.url, "uses": ["DataRelay"], "limitation": "fresh owned loopback-only forwarding fixture"}
    config = {"defaults": {"revision": 2, "resources": []}, "replacement": [resource], "disabled": [],
              "disabled_uses": ["Observation", "Lookup", "Signaling"], "lan_discovery": "Disabled", "port_mapping": "Disabled"}
    runner.native_configuration = runner.private / "local-resources.json"
    runner.native_configuration.write_text(json.dumps(config))
    runner.native_configuration.chmod(0o600)
    runner.configuration = json.dumps({"rtc_enabled": False, "relays": [relay.url]})
    runner.record("controller", "local_resource_allow_list", {"native": config, "browser": json.loads(runner.configuration),
                  "inherited_proxy_removed": True, "desktop_context_removed": True})
