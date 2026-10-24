#!/usr/bin/env python3
"""Actual WASM eight-owner bound, pending stop and current Web policy."""
import argparse
import base64
import json
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process, wait_future


class BrowserBoundsRunner(WebRunner):
    wait_seconds = 15

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
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
        pending_path = "/pending-stream"
        request = json.dumps(dict(url=target.url + "pending-stream", method="GET", headers=[], body=""))
        pool = gateway.executor
        primary = None
        futures = []
        stop_attempted = False
        try:
            for _ in range(8):
                futures.append(pool.submit(browser.call, "request_web", provider.identity, request, ok=False))
            previous_count = -1
            def eight_started():
                nonlocal previous_count
                count = target.requests.count(pending_path)
                if count != previous_count:
                    previous_count = count
                    self.record("controller", "browser_pending_capacity_observation", {"target_requests": count,
                                "browser": browser.call("web_status"), "provider": provider.call("web_status")})
                return count == 8
            # Readiness includes eight ordinary serialized handshakes.
            # The five-second stop bound below is independent of setup.
            self.until(eight_started, "eight Browser owners did not reach actual pending target", seconds=30)
            status = browser.call("web_status")
            assert status["active_operations"] == status["max_operations"] == 8
            browser.call("request_web", provider.identity, request, ok=False)
            assert target.requests.count(pending_path) == 8
            started = time.monotonic()
            stop_attempted = True
            browser.call("stop_web")
            stopped_seconds = time.monotonic()-started
            assert stopped_seconds < 5
            assert browser.call("web_status")["active_operations"] == 0
            self.until(lambda: provider.call("web_status")["active_flows"] == 0,
                       "WASM pending stop left provider HTTP owners", seconds=5)
        except BaseException as failure:
            primary = failure
        finally:
            actions = [("pending-target-release", target.pending_release.set)]
            if primary is not None and not stop_attempted:
                actions.append(("browser-stop", lambda: browser.call("stop_web")))
            actions.extend(("pending-request-%d-join" % index, lambda future=future: wait_future(future, 5))
                           for index, future in enumerate(futures))
            actions.extend([
                ("final-browser-status", lambda: browser.call("web_status")),
                ("final-provider-status", lambda: provider.call("web_status")),
            ])
            outcomes = self.finish_scenario(primary, actions)
            self.record("controller", "browser_pending_capacity_final", {
                "target_requests":target.requests.count(pending_path), "cleanup_outcomes":outcomes})
        response = browser.call("request_web", provider.identity, json.dumps(dict(url=target.url, method="GET", headers=[], body="")))
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        identifiers = []
        for _ in range(8):
            identifier = browser.call("open_web_socket", provider.identity, target.socket_url, "[]")["id"]
            assert browser.call("read_web_socket", identifier, 2000)["message"] == {"type": "text", "text": "owned M119 ready"}
            identifiers.append(identifier)
        assert len(target.active) == 8
        before = len(target.requests)
        browser.call("open_web_socket", provider.identity, target.socket_url, "[]", ok=False)
        assert len(target.requests) == before
        browser.call("send_web_socket", identifiers[0], json.dumps({"type": "text", "text": "M119 Browser at capacity"}))
        assert browser.call("read_web_socket", identifiers[0], 2000)["message"] == {"type": "text", "text": "owned-target:M119 Browser at capacity"}
        provider.call("service_rule", provider=provider.identity, service="web", previous=None,
                      rule={"members": False, "guests": False, "allowed": [], "denied": []})
        self.until(lambda: not target.active and provider.call("web_status")["active_flows"] == 0,
                   "current provider ACL did not close eight Browser sockets")
        for identifier in identifiers:
            result = browser.call("read_web_socket", identifier, 2000)
            assert result["closed"] and result.get("failed"), result
            assert browser.call("close_web_socket", identifier)["failed"]
        assert browser.call("web_status")["active_operations"] == 0
        self.record("controller", "actual_wasm_web_bounds_stop_policy", {"pending_http_target_effects": 8,
                    "ninth_http_no_effect": True, "stop_seconds": stopped_seconds,
                    "pending_requests_failed_and_owners_joined": True, "new_operation_after_stop_works": True,
                    "live_websockets": 8, "ninth_socket_no_effect": True, "duplex_at_capacity": True,
                    "current_acl_closes_all": True, "failed_close_truthful": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--browser-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="bounds")
    BrowserBoundsRunner(parser.parse_args()).run()
