#!/usr/bin/env python3
"""Scoped guest Web access and original lifetime observed by real Windows content."""
import base64
import time

from web_protocol_target import WebTarget
from web_windows_content_runtime import WindowsContentHost
from web_windows_gateway import WindowsWebNode, provider_gateway
from web_windows_policy_checks import WindowsPolicyChecks


class WindowsGuestChecks:
    def __init__(self, runner, provider, target):
        self.runner, self.provider, self.target = runner, provider, target
        self.record, self.until = runner.record, runner.until

    def denied(self, guest, provider, target):
        before, active = len(target.requests), len(target.active)
        guest.call("web_open", address=provider.address, url=target.url, ok=False)
        guest.call("web_socket_open", address=provider.address, url=target.socket_url, ok=False)
        assert len(target.requests) == before and len(target.active) == active
        time.sleep(2.05)  # Same retained guest refused-contact backoff.

    def guest(self, label, seconds):
        invitation = self.runner.private / (label + "-invitation")
        self.provider.call("guest_issue", provider=self.provider.address, service="web", seconds=seconds,
                           path=str(invitation))
        guest = WindowsWebNode(self.runner.binary, self.runner.private, self.runner.output, label,
                               self.runner.environment, self.runner.configuration, self.runner.nodes,
                               action="join-guest", invitation=invitation)
        self.denied(guest, self.provider, self.target)
        status = guest.call("guest_redeem")
        assert status["stage"] == "GuestActive"
        scope = status["scope"]
        assert scope["service"] == "web" and scope["provider"] == self.provider.identity
        assert scope["guest"] == guest.identity
        assert guest.identity not in [row["node"] for row in self.provider.call("status")["members"]]
        before = len(self.target.requests)
        response = guest.call("web_open", address=self.provider.address, url=self.target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        assert len(self.target.requests) == before + 1
        self.record("controller", "Windows_guest_scope_positive", dict(
            independent_identity=True, provider_bound=True, service_web_only=True,
            guest_not_ordinary_member=True, original_expires=scope["expires"], actual_HTTP_exact=True))
        return guest, status, invitation

    def content(self, guest, label):
        private = self.runner.private / (label + "-gateway")
        private.mkdir()
        gateway = provider_gateway(guest, self.provider.address, self.target.url + "page", private,
                                   self.runner.gateways)
        gateway.record = lambda operation, value: self.record(label, operation, value)
        host = WindowsContentHost(self.runner, gateway, label=label + "-content")
        self.until(lambda: host.evaluate("window.fixtureFetch===\"owned M119 fetched resource\""),
                   "actual guest content did not fetch provider-bound script/resource")
        assert host.evaluate("({css:getComputedStyle(document.getElementById('resource')).color,image:document.getElementById('image').naturalWidth})") == dict(css="rgb(12, 34, 56)", image=2)
        probe = WindowsPolicyChecks(self.runner, host, gateway, self.provider, guest, self.target)
        probe.fetch(True)
        probe.live_socket()
        self.record("controller", "actual_Windows_guest_content_positive", dict(
            page_script_fetch_CSS_image=True, actual_native_WebSocket_greeting=True,
            independent_target_active_socket=True, guest_scope_is_current=True))
        return probe

    def ended(self, probe, label, expires=None):
        window_started = None
        invalid_windows = 0

        def settled():
            nonlocal window_started, invalid_windows
            status = self.provider.call("web_status")
            wall, monotonic = time.time(), time.monotonic()
            self.record("controller", "Windows_guest_end_owner_observation", dict(
                cause=label, wall=wall, monotonic=monotonic, target_active_sockets=len(self.target.active),
                provider_active_flows=status["active_flows"]))
            if expires is not None:
                if wall <= expires:
                    if window_started is not None:
                        invalid_windows += 1
                        self.record("controller", "Windows_guest_expiry_window_invalidated_by_wall_rollback", dict(
                            wall=wall, expires=expires, continuous_expired_seconds=monotonic - window_started))
                    window_started = None
                    return False
                if window_started is None:
                    window_started = monotonic
                assert monotonic - window_started < 10, "current original expiry held ten seconds without closing real Web owners"
            return not self.target.active and status["active_flows"] == 0

        self.until(settled, "ended Windows guest retained actual target/provider owner",
                   seconds=100 if expires is not None else 10)
        if expires is not None:
            self.record("controller", "Windows_guest_qualified_original_expiry_close", dict(
                expires=expires, invalid_observation_windows=invalid_windows, close_bound_seconds=10,
                continuous_current_expiry_seconds=time.monotonic() - window_started))
            # Preserve the retained post-close premise margin for wall rollback;
            # it does not extend the ten continuous expired seconds above.
            self.until(lambda: time.time() > expires + 20, "stable current original expiry for new access denial", seconds=100)
        probe.closed(label)
        if expires is not None:
            assert time.time() > expires, "wall rollback invalidated current expiry denial premise"
        probe.denied(label)
        if expires is not None:
            assert time.time() > expires, "wall rollback invalidated observed expiry denial"

    def exercise(self, case):
        seconds = 40 if case == "guest-expiry" else 180
        guest, status, invitation = self.guest("scoped-guest", seconds)
        probe = self.content(guest, "scoped-guest")
        expires = status["scope"]["expires"]
        if case == "guest-expiry":
            assert time.time() < expires, "original guest deadline already expired during actual browser success"
            self.until(lambda: time.time() > expires and guest.call("guest_status")["stage"] == "GuestExpired",
                       "natural original Windows guest deadline", seconds=100)
            self.ended(probe, "guest_original_expiry", expires=expires)
            self.record("controller", "actual_Windows_guest_original_deadline", dict(
                original_deadline=expires, wall_after=time.time(), same_original_deadline=True,
                actual_content_HTTP_WebSocket_before=True, live_socket_closed=True, no_effect_after=True))
        elif case == "guest-end":
            ended = guest.call("guest_end")
            assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
            self.ended(probe, "guest_explicit_end")
            self.record("controller", "actual_Windows_guest_end", dict(
                actual_content_success_before=True, live_socket_closed=True, HTTP_WS_no_effect_after=True))
        elif case == "guest-revoke":
            self.provider.call("guest_revoke", invitation=status["scope"]["invitation"])
            self.ended(probe, "guest_issuer_revoke")
            self.record("controller", "actual_Windows_guest_revoke", dict(
                actual_content_success_before=True, live_socket_closed=True, HTTP_WS_no_effect_after=True))
        elif case == "guest-scope":
            duplicate = WindowsWebNode(self.runner.binary, self.runner.private, self.runner.output,
                                       "other-guest-identity", self.runner.environment, self.runner.configuration,
                                       self.runner.nodes, action="join-guest", invitation=invitation)
            duplicate.call("guest_redeem", ok=False)
            self.denied(duplicate, self.provider, self.target)
            other_provider = WindowsWebNode(self.runner.binary, self.runner.private, self.runner.output,
                                            "other-provider", self.runner.environment,
                                            self.runner.configuration, self.runner.nodes)
            other_target = WebTarget(self.runner)
            self.denied(guest, other_provider, self.target)
            self.denied(guest, self.provider, other_target)
            guest.call("web_enable", targets=[self.target.url], ok=False)
            guest.call("ping", address=self.provider.address, ok=False)
            guest.call("forward_start", protocol="tcp", address=self.provider.address,
                       target="127.0.0.1:1", bind="127.0.0.1:0", ok=False)
            time.sleep(2.05)
            ended = guest.call("guest_end")
            assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
            self.ended(probe, "guest_scoped_end")
            self.record("controller", "actual_Windows_guest_scope_binding", dict(
                invitation_bound_to_first_independent_identity=True, other_provider_denied=True,
                other_target_denied=True, other_services_denied=True, guest_cannot_provide=True,
                not_ordinary_member=True, actual_content_live_socket_closed=True,
                HTTP_WS_no_effect_after=True))
        else:
            raise ValueError("explicit guest case required")
        self.record("controller", "Windows_guest_content_private_close", probe.host.close())
