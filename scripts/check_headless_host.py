#!/usr/bin/env python3
"""Real CLI/service ownership, socket lifetime and independent saved-file check."""
import argparse
import hashlib
import json
import queue
import signal
import socket
import subprocess
import threading
import time
from pathlib import Path

from check_forwarding import Node, isolated_environment
from check_web_service import OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import OrdinaryChild, register_process
from web_protocol_target import WebTarget


class Service(Node):
    def __init__(self, runner, label, directory):
        self.runner, self.label, self.directory = runner, label, directory
        self.stop_requested = False
        self.socket = runner.private / "control.sock"
        self.stderr = (runner.output / (label + ".stderr")).open("w")
        self.process = subprocess.Popen([str(runner.binary), "serve", str(directory), str(self.socket)],
            env=isolated_environment(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.owner, self.reader_error = None, None
        self.queue = queue.Queue(maxsize=128)
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader_started = False
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / (label + "-owner.jsonl"))
        self.reader.start()
        self.reader_started = True
        ready = self.receive()
        assert ready["event"] == "ready" and ready["status"]["stage"] == "Complete"
        self.identity, self.address = ready["status"]["node"], ready["address"]
        assert self.socket.stat().st_mode & 0o777 == 0o600
        runner.record(label, "ready", {"node": self.identity, "stdin": "devnull", "socket_mode": "0600"})

    def exchange(self, data):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(20)
            stream.connect(str(self.socket))
            stream.sendall(data)
            response = stream.makefile("rb").readline(262145)
            assert len(response) <= 262144 and response.endswith(b"\n")
            return json.loads(response)

    def call(self, operation, /, ok=True, **arguments):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        value = self.exchange((json.dumps(dict(id=identifier, op=operation, **arguments)) + "\n").encode())
        self.runner.record(self.label, operation, value)
        assert value["id"] == identifier and value["ok"] == ok, value
        if operation == "stop" and value["ok"]:
            assert value["result"]["stopped"] is True
            self.stop_requested = True
        return value.get("result")

    def close(self):
        if hasattr(self, "cleanup"):
            return self.cleanup
        if self.process.poll() is None and not self.stop_requested:
            self.call("stop")
        self.process.wait(timeout=20)
        if self.reader_started:
            self.reader.join(timeout=3)
        self.stderr.close()
        if self.owner:
            self.owner.close()
        self.cleanup = {"node": self.label, "exit": self.process.returncode,
            "socket_removed": not self.socket.exists(), "reader_joined": not self.reader.is_alive(),
            "reader_error": self.reader_error}
        assert self.process.returncode == 0 and not self.socket.exists() and not self.reader.is_alive() and not self.reader_error
        return self.cleanup

    def terminal(self):
        return super().terminal() and (self.owner is None or self.owner.pidfd is None)


class HostRunner(WebRunner):
    def denied(self, arguments, label, expected):
        receipt = self.output / (label + "-owner.jsonl")
        process = subprocess.Popen([str(self.binary), *map(str, arguments)], env=isolated_environment(),
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        child = OrdinaryChild(process, receipt)
        self.nodes.append(child)
        try:
            try:
                child.owner = register_process(process, receipt)
            except RuntimeError:
                # Rejection may finish before signal ownership is registered.
                # Accept only this original Popen's observed terminal state.
                if process.poll() is None:
                    raise
            stdout, stderr = child.communicate(timeout=15)
            result = subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)
        finally:
            child.close()
        (self.output / (label + ".stdout")).write_text(result.stdout)
        (self.output / (label + ".stderr")).write_text(result.stderr)
        assert result.returncode != 0 and not result.stdout and expected in result.stderr, label
        self.record("controller", label, {"exit": result.returncode, "denied_before_ready": True, "expected_error": expected})

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        creator = OwnedNode(self, "creator")
        member = self.member(creator, "member")
        identity = creator.identity
        directory = Path(creator.directory)
        key_digest = hashlib.sha256((directory / "node.key").read_bytes()).hexdigest()
        alias = self.private / "alias"
        alias.symlink_to(directory, target_is_directory=True)
        parent_alias = self.private / "parent-alias"
        parent_alias.symlink_to(self.private, target_is_directory=True)
        # Same failure seam as the old fixed CLI reproduction, plus all entry
        # families that can mutate saved state before starting a live node.
        cases = [["open", directory], ["open", alias], ["open", parent_alias / directory.name],
            ["create", alias], ["pair", alias], ["join", alias, "missing"], ["join-guest", alias, "missing"],
            ["resume", alias, "missing"], ["restore", alias, "missing"],
            ["recover", alias, "missing", "missing"], ["finalize-replacement", alias, "missing"],
            ["abort-replacement", alias, "missing", "target-not-activated"]]
        for index, arguments in enumerate(cases):
            self.denied(arguments, "cli-owner-denied-%02d" % index, "node already has an application host")
        creator.close()
        service = Service(self, "service", alias)
        assert service.identity == identity
        self.denied(["open", directory], "cli-versus-service-denied", "node already has an application host")
        self.denied(["serve", directory, self.private / "second.sock"], "service-versus-service-denied", "node already has an application host")
        assert not (self.private / "second.sock").exists()
        with socket.socket(socket.AF_UNIX) as abandoned:
            abandoned.connect(str(service.socket))
        assert service.call("status")["node"] == identity
        for malformed in [b"bad json\n", b" " * 65537 + b"\n"]:
            assert service.exchange(malformed)["error"] == "invalid_command"
        assert member.call("ping", address=service.address)["reply"] == "PONG"
        destination = self.private / "saved"
        service.call("files_enable", directory=str(destination))
        payload = bytes(i % 239 for i in range(65539))
        source = self.private / "actual.bin"
        source.write_bytes(payload)
        task = member.call("file_send", address=service.address, path=str(source))["task"]
        self.until(lambda: any(t["id"] == task and t["state"] == "complete" for t in member.call("files_status")["tasks"]), "service file not saved", seconds=20)
        files = list(destination.iterdir())
        assert len(files) == 1 and files[0].read_bytes() == payload
        self.record("controller", "independent_saved_file", {"bytes": len(payload), "sha256": hashlib.sha256(files[0].read_bytes()).hexdigest(), "stdin_eof_did_not_stop_service": True})
        target = WebTarget(self)
        member.call("web_enable", targets=[target.url])
        service.call("service_sync", address=member.address)
        result = []
        def held():
            try:
                result.append(service.call("web_open", ok=False, address=member.address, url=target.url + "pending"))
            except BaseException as failure:
                result.append(failure)
        pending = threading.Thread(target=held)
        pending.start()
        try:
            assert target.pending_started.wait(10), "real HTTP prerequisite absent"
            assert pending.is_alive()
            started = time.monotonic()
            assert service.call("stop")["stopped"]
            pending.join(timeout=5)
            assert not pending.is_alive() and result == [None], result
            service.close()
            self.record("controller", "stop_pending", {"actual_http_pending": True, "seconds": time.monotonic()-started, "caller_failed_truthfully": True})
        finally:
            target.pending_release.set()
            pending.join(timeout=20)
        reopened = Service(self, "reopened", directory)
        assert reopened.identity == identity
        assert hashlib.sha256((directory / "node.key").read_bytes()).hexdigest() == key_digest
        assert member.call("ping", address=reopened.address)["reply"] == "PONG"
        # SIGTERM is this service's documented graceful shutdown. Signal only
        # the registered original Popen/pidfd after its full identity check.
        reopened.owner.send(signal.SIGTERM)
        reopened.process.wait(timeout=20)
        reopened.close()
        final = OwnedNode(self, "final-cli", "open", directory=str(directory))
        assert final.identity == identity
        self.record("controller", "lifecycle", {"same_identity": True, "complete_membership": True,
            "normal_stop_then_reopen": True, "owned_sigterm_exit": reopened.process.returncode,
            "cli_reopens_after_service": True, "login_autostart_proven": False})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "source-receipt", "output", "relay-binary", "relay-receipt"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.set_defaults(case="headless")
    HostRunner(parser.parse_args()).run()
