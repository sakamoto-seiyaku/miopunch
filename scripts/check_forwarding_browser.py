#!/usr/bin/env python3
"""Independent headless Chromium/WASM uses the public bounded forwarding API.

Fresh loopback targets independently observe the generated data. Browser
profiles and capability returns stay in the owned private directory/RPC pipe.
"""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading

from check_forwarding import Node, Runner, Target, isolated_environment

ROOT = Path(__file__).resolve().parents[1]


class Browser(Node):
    def __init__(self, runner, args):
        self.runner, self.label = runner, "browser"
        self.cleanup = None
        harness = ROOT / "browser-probe/harness/forwarding.mjs"
        for path in [Path(__file__), harness,
                     ROOT / "browser-probe/harness/node_modules/playwright-core/package.json",
                     ROOT / "browser-probe/harness/node_modules/playwright-core/lib/coreBundle.js"]:
            runner.freeze_source(path)
        self.stderr = (runner.output / "browser.stderr").open("w")
        self.process = subprocess.Popen(["rtk", "proxy", "node", str(harness), str(args.package.resolve()),
                                         str(args.chromium.resolve()), str(runner.private / "browser-profiles")],
                                        cwd=ROOT, env=isolated_environment(), stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.queue = queue.Queue(maxsize=128)
        self.reader_error = None
        self.reader = threading.Thread(target=self.read, daemon=True)
        runner.nodes.append(self)
        self.reader.start()
        assert self.receive()["event"] == "ready"

    def call(self, operation, *args, ok=True, actor="wasm"):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, actor=actor, op=operation, args=args)) + "\n")
        self.process.stdin.flush()
        value = self.receive()
        public = {key: result for key, result in value.items() if key != "privateError"}
        if operation in {"export_recovery_card", "invitation", "precode_invitation", "issue_service_guest"}:
            public.pop("result", None)
            public["private_return"] = value["ok"]
        self.runner.record(actor, operation, public)
        assert value.get("id") == identifier and value["ok"] == ok, public
        return value.get("result")

    def close(self):
        errors, actors, accounted = [], [], False
        if self.process.poll() is None:
            try:
                result = self.call("close", actor="runner")
                actors = result["cleanup"]
                accounted = result["server_closed"] and all(actor["terminal"] for actor in actors)
                if not all(actor["closed"] for actor in actors):
                    errors.append("BrowserCleanup")
            except BaseException as failure:
                errors.append(type(failure).__name__)
            try:
                self.process.stdin.close()
            except BaseException as failure:
                errors.append(type(failure).__name__)
        else:
            errors.append("HostExitedBeforeCleanup")
        try:
            self.process.wait(timeout=25)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        self.reader.join(timeout=3)
        self.stderr.close()
        self.cleanup = {"node": "browser", "exit": self.process.poll(), "reader_joined": not self.reader.is_alive(),
                        "reader_error": self.reader_error, "actors": actors, "all_actors_terminal": accounted, "errors": errors}
        assert self.process.poll() == 0 and not self.reader.is_alive() and not self.reader_error and accounted and not errors, self.cleanup
        return self.cleanup

    def terminal(self):
        return super().terminal() and self.cleanup is not None and self.cleanup["all_actors_terminal"]


class BrowserRunner(Runner):
    def __init__(self, args):
        build = json.loads(args.build_receipt.read_text())
        assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
        actual = {p.relative_to(args.package).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                  for p in args.package.rglob("*") if p.is_file()}
        assert actual == build["artifacts"]
        super().__init__(args)
        self.args = args
        self.record("controller", "wasm_pin", {"package_sha256": build["packageSHA256"], "source_receipt": str(args.build_receipt),
                    "chromium_sha256": hashlib.sha256(args.chromium.read_bytes()).hexdigest(), "headless": True})

    def scenario(self):
        provider = Node(self, "provider")
        browser = Browser(self, self.args)
        browser.call("open")
        invitation = self.private / "browser-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = browser.call("start", "join", json.dumps({"rtc_enabled": False, "relays": ["https://aps1-1.relay.n0.iroh.link./"]}), invitation.read_text())
        assert status["id"] != provider.identity
        assert browser.call("request_join", False) == "Complete"
        capability = browser.call("forwarding_status")
        assert capability["native_listener"] is False and capability["provider"] is False and capability["bounded_exchange"] is True
        for protocol in ["tcp", "udp"]:
            target = Target(self, protocol, prefix=b"")
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            for length in [17, 60000] if protocol == "tcp" else [0, 17, 60000]:
                seed = 23
                before = len(target.received)
                result = browser.call("forward_exchange", provider.identity, protocol, target.address, length, seed)
                assert result == dict(length=length, exact=True), result
                observed = b"".join(target.received[before:]) if protocol == "tcp" else target.received[before:]
                payload = bytes((index + seed) % 251 for index in range(length))
                assert observed == (payload if protocol == "tcp" else [payload])
                self.record("controller", "independent_browser_socket_effect", {"protocol": protocol, "length": length, "target_exact": True, "reply_exact": True})
            provider.call("forward_disable", protocol=protocol)
            before = len(target.received)
            browser.call("forward_exchange", provider.identity, protocol, target.address, 19, 7, ok=False)
            assert len(target.received) == before
        assert browser.call("forwarding_status")["active_exchanges"] == 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="browser")
    parser.add_argument("--loopback", default="127.0.0.1")
    BrowserRunner(parser.parse_args()).run()
