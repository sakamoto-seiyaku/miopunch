#!/usr/bin/env python3
"""Actual WASM issuer and consumer Web guests on fresh owned local resources."""
import argparse
import base64
import json
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process


class BrowserGuestRunner(WebRunner):
    wait_seconds = 10
    poll_seconds = 0.05

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        gateway = ContentGateway(target.url, lambda _: None, self.private, self.nodes, browser_origins=True)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        browser = WebBrowser(self, self.args, gateway)
        assert not browser.call("open")["browserSandboxDisabledByHarness"]
        browser.call("start", "create", self.configuration, None)
        card = self.private / "browser-card"
        card.write_text(browser.call("export_recovery_card", None))
        card.chmod(0o600)
        browser.call("acknowledge_recovery_saved")
        join = self.private / "native-invitation"
        join.write_text(browser.call("invitation"))
        join.chmod(0o600)
        provider = OwnedNode(self, "native-provider", "join", join)
        browser.call("begin_serve_admission")
        assert provider.call("request", path=str(join))["stage"] == "AwaitingApproval"
        assert browser.call("finish_serve_admission") == "AwaitingApproval"
        request = next(row for row in browser.call("admission_requests") if row["applicant"] == provider.identity)
        browser.call("approve_admission", provider.identity, request["request"])
        browser.call("begin_serve_admission")
        assert provider.call("request", path=str(join))["stage"] == "Complete"
        assert browser.call("finish_serve_admission") == "Complete"
        provider.call("web_enable", targets=[target.url])
        browser.call("sync_services", provider.identity)
        invitation = self.private / "browser-issued-web-invitation"
        invitation.write_text(browser.call("issue_service_guest", json.dumps(provider.address), "web", int(time.time()) + 180))
        invitation.chmod(0o600)
        native = OwnedNode(self, "native-guest", "join-guest", invitation)
        before = len(target.requests)
        native.call("web_open", address=provider.address, url=target.url, ok=False)
        assert len(target.requests) == before
        assert native.call("guest_redeem")["scope"]["service"] == "web"
        browser.call("sync_services", provider.identity)
        response = native.call("web_open", address=provider.address, url=target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        identifier = native.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
        assert native.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned M119 ready"}
        scope = native.call("guest_status")
        deadline = scope["scope"]["expires"]
        revision = browser.call("set_service_rule", provider.identity, "web", None,
                                json.dumps(dict(members=True, guests=True, allowed=[], denied=[native.identity])))
        browser.call("sync_services", provider.identity)
        self.until(lambda: provider.call("web_status")["active_flows"] == 0 and not target.active,
                   "WASM administrator current ACL did not close Native guest socket")
        assert native.call("web_socket_close", socket=identifier)["failed"]
        before = len(target.requests)
        native.call("web_open", address=provider.address, url=target.url, ok=False)
        assert len(target.requests) == before
        time.sleep(2.05)  # keep the existing refused-contact backoff before restored access
        browser.call("set_service_rule", provider.identity, "web", revision,
                     json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
        browser.call("sync_services", provider.identity)
        retained = native.call("guest_redeem")
        assert retained["scope"]["expires"] == deadline and time.time() < deadline
        response = native.call("web_open", address=provider.address, url=target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        identifier = native.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
        assert native.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned M119 ready"}
        browser.call("revoke_guest", scope["scope"]["invitation"])
        browser.call("sync_services", provider.identity)
        self.until(lambda: provider.call("web_status")["active_flows"] == 0 and not target.active,
                   "WASM issuer revoke did not close Native guest Web socket")
        assert native.call("web_socket_close", socket=identifier)["failed"]
        before = len(target.requests)
        native.call("web_open", address=provider.address, url=target.url, ok=False)
        assert len(target.requests) == before
        invitation = self.private / "native-issued-web-invitation"
        provider.call("guest_issue", provider=provider.address, service="web", seconds=180, path=str(invitation))
        assert not browser.call("open", actor="guest")["browserSandboxDisabledByHarness"]
        status = browser.call("start", "join_guest", self.configuration, invitation.read_text(), actor="guest")
        request = json.dumps(dict(url=target.url, method="GET", headers=[], body=""))
        before = len(target.requests)
        browser.call("request_web", provider.identity, request, actor="guest", ok=False)
        assert len(target.requests) == before
        scope = browser.call("redeem_guest", actor="guest")
        assert scope["stage"] == "GuestActive" and scope["scope"]["service"] == "web"
        assert scope["scope"]["guest"] == status["id"]
        response = browser.call("request_web", provider.identity, request, actor="guest")
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        original_deadline = scope["scope"]["expires"]
        browser.call("reload", actor="guest")
        reopened = browser.call("start", "start", self.configuration, None, actor="guest")
        retained = browser.call("redeem_guest", actor="guest")
        assert reopened["id"] == status["id"] and retained["scope"]["expires"] == original_deadline
        assert time.time() < original_deadline
        response = browser.call("request_web", provider.identity, request, actor="guest")
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        identifier = browser.call("open_web_socket", provider.identity, target.socket_url, "[]", actor="guest")["id"]
        assert browser.call("read_web_socket", identifier, 2000, actor="guest")["message"] == {"type": "text", "text": "owned M119 ready"}
        ended = browser.call("end_guest", actor="guest")
        assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
        self.until(lambda: not target.active and provider.call("web_status")["active_flows"] == 0,
                   "WASM guest exit did not close actual WebSocket")
        assert browser.call("close_web_socket", identifier, actor="guest")["failed"]
        before = len(target.requests)
        browser.call("request_web", provider.identity, request, actor="guest", ok=False)
        assert len(target.requests) == before
        self.record("controller", "actual_wasm_web_guest_issuer_consumer", {"browser_issuer_native_guest": True,
                    "native_issuer_browser_guest": True, "pending_no_effect": True, "real_exact_http_and_socket": True,
                    "issuer_revoke_and_guest_exit_close_socket": True, "no_effect_after": True,
                    "browser_administrator_current_acl_deny_restore": True,
                    "browser_guest_saved_identity_original_deadline_after_reopen": True})


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
    parser.add_argument("--case", default="guests")
    BrowserGuestRunner(parser.parse_args()).run()
