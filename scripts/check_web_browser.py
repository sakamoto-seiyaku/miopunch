#!/usr/bin/env python3
"""Independent real WASM node and isolated in-app content acceptance for M119."""
import argparse
import asyncio
import base64
import json
from pathlib import Path
import ssl
import secrets
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

from check_web_container import ContainerRunner
from check_web_service import OwnedNode
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import BrowserWebRequests, WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process
from web_protocol_target import WebTarget


class BrowserFrame:
    platform = "Browser/WASM/Chromium"
    def __init__(self, browser):
        self.browser = browser

    def evaluate(self, expression):
        return self.browser.call("content_evaluate", expression)


class BrowserRunner(ContainerRunner):
    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = WebTarget(self, tls=True)
        verification = ssl.create_default_context(cafile=target.certificate)
        with build_opener(ProxyHandler({}), HTTPSHandler(context=verification)).open(target.url, timeout=3) as response:
            assert response.read() == b"owned M119 web target"
        asyncio.run(target.socket_prerequisite())
        self.record("controller", "independent_browser_target_precondition", {"https_exact": True, "normal_tls_validation": True,
                    "real_websocket_handshake_and_bidirectional_message": True})
        provider = OwnedNode(self, "provider")
        provider.call("web_enable", targets=[target.url], trust_roots_pem=[target.trust_root_pem])
        holder = {}
        requests = BrowserWebRequests(provider.identity)
        request = requests.request

        callbacks = {
            "open": lambda url, headers: holder["browser"].call("open_web_socket", provider.identity, url, json.dumps(headers)),
            "send": lambda identifier, message: holder["browser"].call("send_web_socket", identifier, json.dumps(message)),
            "read": lambda identifier: holder["browser"].call("read_web_socket", identifier, 100),
            "close": lambda identifier: holder["browser"].call("close_web_socket", identifier),
        }
        gateway = ContentGateway(target.url + "page", request, self.private, self.nodes, socket_callbacks=callbacks,
                                 browser_origins=True, http_callbacks=requests.callbacks)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        browser = WebBrowser(self, self.args, gateway)
        holder["browser"] = browser
        requests.bind(browser)
        opened = browser.call("open")
        assert not opened["browserSandboxDisabledByHarness"]
        invitation = self.private / "browser-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = browser.call("start", "join", self.configuration, invitation.read_text())
        assert status["id"] != provider.identity
        assert browser.call("request_join", False) == "Complete"
        browser.call("sync_services", provider.identity)
        self.record("controller", "independent_wasm_admission_precondition", {"distinct_identity": True,
                    "real_admission_complete": True, "real_signed_service_sync": True})
        response = request(dict(url=target.url, method="GET", headers=[], body=""))
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        self.record("controller", "public_browser_web_delivery", {"target_observed": True, "response_exact": True})
        if self.args.case == "entry":
            return
        browser.call("content_open", gateway.bootstrap)
        frame = BrowserFrame(browser)
        canary = secrets.token_hex(16)
        assert browser.call("management_prepare", canary)["saved"]
        try:
            self.exercise_content(frame, target, gateway, lambda: browser.call("web_status")["active_operations"] == 0
                                  and provider.call("web_status")["active_flows"] == 0)
            self.exercise_isolation(frame, gateway, provider)
            unchanged = browser.call("management_check", canary)
            assert all(unchanged.values()), unchanged
            self.record("controller", "actual_management_storage_unchanged", unchanged)
        except BaseException:
            failures = []
            self.record("controller", "browser_gateway_phase_diagnostic", {"phases": dict(gateway.phases),
                        "last_denial": gateway.last_denial, "target_requests": len(target.requests)})
            try:
                state = frame.evaluate("({ready:document.readyState,bootstrap:location.pathname.startsWith('/__miopunch_open/'),contentType:document.contentType,hasResource:!!document.getElementById('resource'),secure:window.isSecureContext})")
                self.record("controller", "actual_browser_content_diagnostic", {"state": state})
            except BaseException as failure:
                failures.append(type(failure).__name__)
            self.record("controller", "browser_diagnostic_failures", {"failures": failures})
            raise


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
    parser.add_argument("--case", default="container")
    BrowserRunner(parser.parse_args()).run()
