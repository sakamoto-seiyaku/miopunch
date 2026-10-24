#!/usr/bin/env python3
"""Owned local HTTP service acceptance through the public client entry.

Consumes pinned artifacts; never compiles. The target, relay, node identities,
state and processes are all newly owned by this controller.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time
from urllib.request import ProxyHandler, build_opener

from check_forwarding import Node, Runner, isolated_environment
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process
from web_protocol_target import WebTarget


class OwnedNode(Node):
    def __init__(self, runner, label, action="create", invitation=None, directory=None):
        self.runner, self.label = runner, label
        self.directory = directory or str(runner.private / label)
        self.stderr = (runner.output / (label + ".stderr")).open("w")
        command = [str(runner.binary), action, self.directory]
        if invitation:
            command.append(str(invitation))
        if action in {"create", "join", "join-guest"}:
            command.append(str(runner.native_configuration))
        self.process = subprocess.Popen(command, env=isolated_environment(), stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.queue = queue.Queue(maxsize=128)
        self.reader_error, self.owner = None, None
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader_started = False
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / (label + "-owner.jsonl"))
        self.reader.start()
        self.reader_started = True
        ready = self.receive()
        assert ready["event"] == "ready"
        self.identity, self.address = ready["status"]["node"], ready["address"]
        if ready["status"]["stage"] == "AwaitingRecoverySave":
            self.call("export_recovery_card", path=str(runner.private / (label + "-card")))
            self.call("acknowledge_recovery_saved")

    def close(self):
        errors = []
        if self.process.poll() is None and self.reader_started:
            try:
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
        if self.process.poll() is None and self.owner:
            for number in [signal.SIGTERM, signal.SIGKILL]:
                if self.process.poll() is not None:
                    break
                try:
                    self.owner.send(number)
                    self.process.wait(timeout=5)
                except BaseException as failure:
                    errors.append(type(failure).__name__)
        if self.reader_started:
            self.reader.join(timeout=3)
        self.stderr.close()
        if self.owner and self.process.poll() is not None:
            try:
                self.owner.close()
            except BaseException as failure:
                errors.append(type(failure).__name__)
        self.cleanup = dict(node=self.label, exit=self.process.poll(), reader_joined=not self.reader.is_alive(),
                            reader_error=self.reader_error, errors=errors)
        assert self.process.poll() == 0 and not self.reader.is_alive() and not self.reader_error and not errors, self.cleanup
        return self.cleanup

    def terminal(self):
        return super().terminal() and (self.owner is None or self.owner.pidfd is None)


HttpTarget = WebTarget


class WebRunner(Runner):
    wait_seconds = 5
    poll_seconds = .02

    def until(self, predicate, label, seconds=None):
        deadline = time.monotonic() + (self.wait_seconds if seconds is None else seconds)
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(self.poll_seconds)
        raise AssertionError(label)

    def finish_scenario(self, primary, actions):
        """Keep the first failure while attempting each local cleanup step."""
        outcomes, failures = [], []
        for label, action in actions:
            try:
                outcomes.append({"step":label, "result":action()})
            except BaseException as failure:
                item = {"step":label, "type":type(failure).__name__, "message":str(failure)}
                outcomes.append(item)
                failures.append(item)
        try:
            self.record("controller", "scenario_cleanup", {"outcomes":outcomes, "errors":failures})
        except BaseException as failure:
            failures.append({"step":"cleanup-journal", "type":type(failure).__name__, "message":str(failure)})
        if primary is not None:
            if failures:
                primary.add_note("Independent cleanup failures: " + json.dumps(failures))
            raise primary
        if failures:
            raise RuntimeError("Independent scenario cleanup failed: " + json.dumps(failures))
        return outcomes

    def freeze_source(self, path):
        path = Path(path).resolve()
        content = path.read_bytes()
        digest = hashlib.sha256(content).hexdigest()
        if str(path) in self.controller_sources:
            assert self.controller_sources[str(path)] == digest, "controller changed during owned run"
            return
        super().freeze_source(path)
        root = Path(__file__).resolve().parents[1]
        relative = path.relative_to(root) if path.is_relative_to(root) else Path("external") / hashlib.sha256(str(path).encode()).hexdigest() / path.name
        destination = self.output / "controller-sources" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)
        assert hashlib.sha256(destination.read_bytes()).hexdigest() == digest

    def __init__(self, args):
        sanitized = isolated_environment()
        sanitized.pop("SSLKEYLOGFILE", None)
        os.environ.clear()
        os.environ.update(sanitized)
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.binary = args.binary.resolve()
        build = json.loads(args.source_receipt.read_text())
        assert build["exit"] == 0 and build["sha256"] == args.sha256
        assert hashlib.sha256(self.binary.read_bytes()).hexdigest() == args.sha256
        self.private = Path(tempfile.mkdtemp(prefix="m119-web-"))
        (self.private / ".owner").write_text(str(self.output))
        self.nodes, self.targets, self.sequence = [], [], 0
        self.controller_sources = {}
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(sys.argv[0]))
        self.freeze_source(Path(__file__).with_name("check_forwarding.py"))
        for name in ["web_process_owner.py", "web_protocol_target.py", "web_tls_fixture.py"]:
            self.freeze_source(Path(__file__).with_name(name))
        self.case = args.case
        self.args = args
        self.record("controller", "pin", {"sha256": args.sha256, "source_receipt": str(args.source_receipt)})

    def record(self, node, operation, result):
        if operation in {"web_open", "request_web"} and result.get("ok"):
            response = result["result"]
            result = {"id": result["id"], "ok": True, "result": {
                "status": response["status"], "body_bytes": len(base64.b64decode(response["body"])),
                "headers_and_body_omitted": True}}
        elif operation in {"web_socket_read", "read_web_socket"} and result.get("ok"):
            response = result["result"]
            message = response.get("message", {})
            public = {key: response[key] for key in ["pending", "closed"] if key in response}
            if message:
                public["type"] = message["type"]
                if message["type"] == "text":
                    public["message_bytes"] = len(message["text"].encode())
                elif message["type"] == "binary":
                    public["message_bytes"] = len(base64.b64decode(message["data"]))
                elif message["type"] == "close":
                    public["code"] = message["code"]
                public["message_content_omitted"] = True
            result = {"id": result["id"], "ok": True, "result": public}
        elif operation in {"web_socket_open", "open_web_socket"} and result.get("ok"):
            response = result["result"]
            result = {"id": result["id"], "ok": True, "result": {key: value for key, value in response.items() if key != "headers"}}
        super().record(node, operation, result)

    def member(self, creator, label):
        invitation = self.private / (label + "-invitation")
        creator.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        member = OwnedNode(self, label, "join", invitation)
        member.call("request", path=str(invitation))
        creator.call("service_sync", address=member.address)
        return member

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        with build_opener(ProxyHandler({})).open(target.url, timeout=2) as response:
            assert response.read() == b"owned M119 web target"
        assert target.requests == ["/"]
        self.record("controller", "target_precondition", {"http_live": True, "independent_exact": True})
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        assert provider.identity != consumer.identity
        assert consumer.call("ping", address=provider.address)["reply"] == "PONG"
        self.record("controller", "public_member_precondition", {"distinct_identities": True, "actual_reply": True})
        provider.call("web_enable", targets=[target.url])
        primary, observations = None, []
        try:
            opened = consumer.call("web_open", address=provider.address, url=target.url)
        except BaseException as failure:
            primary = failure
        self.record("controller", "independent_target_count", {"requests": len(target.requests)})
        for node in [provider, consumer]:
            try:
                node.call("web_status")
            except BaseException as failure:
                observations.append({"node": node.label, "failure": type(failure).__name__})
        self.record("controller", "status_observation_failures", {"failures": observations})
        if primary:
            raise primary
        assert not observations, observations
        assert opened["url"] == target.url and opened["status"] == 200
        assert base64.b64decode(opened["body"]) == b"owned M119 web target"
        assert target.requests == ["/", "/"]
        self.record("controller", "public_web_delivery", {"target_observed": True, "response_exact": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="entry")
    WebRunner(parser.parse_args()).run()
