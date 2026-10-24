#!/usr/bin/env python3
"""Explicit public consumer close under independently paused outgoing traffic."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import time

from check_forwarding import isolated_environment
from check_web_service import OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process, run_process, wait_future
from web_protocol_target import WebTarget


class PressureRunner(WebRunner):
    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = WebTarget(self)
        asyncio.run(target.socket_prerequisite())
        assert target.observed == [("text", "M119 direct target prerequisite")]
        if self.args.platform == "native":
            build = json.loads(self.args.consumer_receipt.read_text())
            binary = Path(build["pin"])
            assert build["exit"] == 0 and build["CargoInvoked"] is False and build["dependencyRebuilds"] == 0
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["sha256"]
            source = Path(__file__).resolve().parents[1] / "tests/web_lifecycle.rs"
            assert hashlib.sha256(source.read_bytes()).hexdigest() == build["testSHA256"]
            self.freeze_source(source)
            self.freeze_source(Path(__file__).with_name("build_device_tests.py"))
            root = self.private / "sdk-pressure"
            root.mkdir(mode=0o700)
            environment = isolated_environment()
            environment.update(M119_WEB_PRIVATE=str(root), M119_WEB_LOCAL_RELAY=json.loads(self.configuration)["relays"][0],
                M119_WEB_PRESSURE_TARGET=target.url, M119_WEB_PRESSURE_RELEASE=str(target.pressure_release))
            result = run_process([str(binary), "--exact", "owned_web_socket_explicit_close_covers_outgoing_enqueue",
                "--nocapture", "--test-threads=1"], env=environment, timeout=90, owners=self.nodes,
                receipt=self.output / "sdk-pressure-owner.jsonl")
            (self.output / "sdk.stdout").write_text(result.stdout)
            (self.output / "sdk.stderr").write_text(result.stderr)
            self.record("controller", "public_sdk_pressure_result", {"exit": result.returncode,
                "target_read_paused": target.pressure_paused.is_set(), "source_receipt": build["SDKReceipt"]})
            assert target.pressure_paused.is_set(), "public consumer never reached independently paused target"
            assert result.returncode == 0, "public explicit-close pressure regression failed; original output retained"
        else:
            self.browser_scenario(target)
        self.until(lambda: not target.active, "explicit close/cleanup retained independent target socket", seconds=5)

    def browser_scenario(self, target):
        provider = OwnedNode(self, "provider")
        provider.call("web_enable", targets=[target.url])
        gateway = ContentGateway(target.url, lambda _: None, self.private, self.nodes, browser_origins=True)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        browser = WebBrowser(self, self.args, gateway)
        assert not browser.call("open")["browserSandboxDisabledByHarness"]
        invitation = self.private / "browser-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=300, max_uses=1)
        browser.call("start", "join", self.configuration, invitation.read_text())
        assert browser.call("request_join", False) == "Complete"
        browser.call("sync_services", provider.identity)
        identifier = browser.call("open_web_socket", provider.identity, target.socket_url+"?pressure=1", "[]")["id"]
        assert browser.call("read_web_socket", identifier, 2000)["message"] == {"type":"text", "text":"owned M119 ready"}
        assert target.pressure_paused.is_set()
        # JSON's shared Web wire encodes binary as base64. Ordinary target
        # traffic is bounded at 1024 frames of 32KiB, with no pressure replies.
        import base64
        message = json.dumps({"type":"binary", "data":base64.b64encode(bytes([0x5a])*32768).decode()})
        # The gateway already owns and joins its executor after Browser close.
        pool = gateway.executor
        pending, closing, primary = None, None, None
        try:
            for count in range(1024):
                pending = pool.submit(browser.call, "send_web_socket", identifier, message)
                try:
                    self.until(pending.done, "outgoing send remains Pending", seconds=1)
                    pending.result()
                except AssertionError:
                    assert not pending.done(), "send failed rather than remaining Pending"
                    break
            else:
                raise AssertionError("bounded Browser producer did not establish outgoing pressure")
            assert not pending.done() and browser.call("web_status")["active_operations"] == 1
            assert provider.call("web_status")["active_flows"] == 1
            self.record("controller", "outgoing_pressure_proved", {"accepted_frames":count, "frame_bytes":32768,
                "pending_send_ms":1000, "target_reads_paused":True, "provider_enabled":True})
            started = time.monotonic()
            # Expected error: this target cannot perform a close handshake.
            closing = pool.submit(browser.call, "close_web_socket", identifier, ok=False)
            self.until(closing.done, "explicit consumer close exceeded monotonic6.25s budget", seconds=6.25)
            closing.result()
            elapsed = time.monotonic()-started
            assert browser.call("web_status")["active_operations"] == 0
            self.record("controller", "explicit_close_bounded", {"seconds":elapsed,
                "failed_truthfully":True, "exact_consumer_owner_joined":True})
        except BaseException as failure:
            primary = failure
        finally:
            def join(future, operation):
                if future is None:
                    return {"not_started":True}
                try:
                    return wait_future(future, 10)
                except AssertionError as failure:
                    detail = failure.args[0] if failure.args else None
                    if (operation == "send_web_socket" and isinstance(detail, dict)
                            and detail.get("operation") == operation and detail.get("ok") is False):
                        return {"expected_cancelled_send":True}
                    raise
            # Every independent action runs even if release, disable or an
            # earlier join fails. None can satisfy the close business oracle.
            actions = []
            if primary is not None:
                actions.append(("failure-observation", lambda: self.record("controller", "explicit_close_failure_before_cleanup", {
                    "type":type(primary).__name__, "elapsed":time.monotonic()-started if closing else None,
                    "send_done":pending.done() if pending else None, "close_done":closing.done() if closing else None,
                    "target_still_paused":not target.pressure_release.exists(), "provider_disable_started":False})))
            self.finish_scenario(primary, actions + [
                ("target-release", target.pressure_release.touch),
                ("provider-disable", lambda: provider.call("web_disable")),
                ("pending-send-join", lambda: join(pending, "send_web_socket")),
                ("explicit-close-join", lambda: join(closing, "close_web_socket")),
            ])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "source-receipt", "output", "relay-binary", "relay-receipt"]:
        parser.add_argument("--"+name, type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--platform", choices=["native", "browser"], required=True)
    parser.add_argument("--consumer-receipt", type=Path)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--browser-receipt", type=Path)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="outgoing-pressure")
    PressureRunner(parser.parse_args()).run()
