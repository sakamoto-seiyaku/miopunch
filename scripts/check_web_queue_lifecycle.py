#!/usr/bin/env python3
"""Public WebSocket owner closure with a backpressured application reader."""
import argparse
import json
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process


class QueueRunner(WebRunner):

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        provider = OwnedNode(self, "provider")
        provider.call("web_enable", targets=[target.url])
        if self.args.platform == "native":
            consumer = self.member(provider, "consumer")
            identifier = consumer.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
            read = lambda: consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)
            send = lambda message: consumer.call("web_socket_send", socket=identifier, message=message)
            status = lambda: consumer.call("web_status")["active_flows"]
            close = lambda: consumer.call("web_socket_close", socket=identifier)
        else:
            gateway = ContentGateway(target.url, lambda _: None, self.private, self.nodes, browser_origins=True)
            gateway.record = lambda operation, result: self.record("controller", operation, result)
            consumer = WebBrowser(self, self.args, gateway)
            assert not consumer.call("open")["browserSandboxDisabledByHarness"]
            invitation = self.private / "browser-invitation"
            provider.call("issue_precode", path=str(invitation), seconds=300, max_uses=1)
            consumer.call("start", "join", self.configuration, invitation.read_text())
            assert consumer.call("request_join", False) == "Complete"
            consumer.call("sync_services", provider.identity)
            identifier = consumer.call("open_web_socket", provider.identity, target.socket_url, "[]")["id"]
            read = lambda: consumer.call("read_web_socket", identifier, 2000)
            send = lambda message: consumer.call("send_web_socket", identifier, json.dumps(message))
            status = lambda: consumer.call("web_status")["active_operations"]
            close = lambda: consumer.call("close_web_socket", identifier)
        assert read()["message"] == {"type":"text", "text":"owned M119 ready"}
        for index in range(32):
            send({"type":"text", "text":"M119 queued echo %02d" % index})
        expected = [("text", "M119 queued echo %02d" % index) for index in range(32)]
        self.until(lambda: target.observed == expected, "independent target did not observe all32 ordinary frames")
        assert status() == 1
        self.record("controller", "backpressured_socket_precondition", {"platform": self.args.platform,
                    "target_observed_exact_frames": 32, "application_reads_after_greeting": 0, "owner_active": True})
        provider.call("web_disable")
        self.until(lambda: not target.active and provider.call("web_status")["active_flows"] == 0,
                   "provider disable retained actual target socket")
        self.until(lambda: status() == 0, "peer closure did not release backpressured Web consumer owner")
        assert close()["failed"]
        self.record("controller", "backpressured_socket_owner_closed", {"platform": self.args.platform,
                    "consumer_owner_joined": True, "provider_owner_and_target_closed": True, "failed_close_truthful": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--platform", choices=["native","browser"], required=True)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--browser-receipt", type=Path)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="queue-lifecycle")
    QueueRunner(parser.parse_args()).run()
