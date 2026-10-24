#!/usr/bin/env python3
"""Public Web guest invitation/authorization acceptance on fresh owned state."""
import argparse
import base64
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


class GuestRunner(WebRunner):
    wait_seconds = 10
    poll_seconds = 0.05

    def positive(self, guest, provider, target):
        before = len(target.requests)
        response = guest.call("web_open", address=provider.address, url=target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        assert len(target.requests) == before + 1

    def denied(self, guest, provider, target):
        before = len(target.requests)
        active_before = len(target.active)
        guest.call("web_open", address=provider.address, url=target.url, ok=False)
        guest.call("web_socket_open", address=provider.address, url=target.socket_url, ok=False)
        assert len(target.requests) == before and len(target.active) == active_before
        time.sleep(2.05)  # preserve the existing refused-contact backoff

    def guest(self, provider, target, label, seconds=180):
        invitation = self.private / (label + "-invitation")
        provider.call("guest_issue", provider=provider.address, service="web", seconds=seconds, path=str(invitation))
        guest = OwnedNode(self, label, "join-guest", invitation)
        self.denied(guest, provider, target)
        status = guest.call("guest_redeem")
        assert status["stage"] == "GuestActive"
        assert status["scope"]["service"] == "web" and status["scope"]["provider"] == provider.identity
        assert status["scope"]["guest"] == guest.identity
        assert guest.identity not in [row["node"] for row in provider.call("status")["members"]]
        return guest, status, invitation

    def live_socket(self, guest, provider, target):
        identifier = guest.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
        assert guest.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned M119 ready"}
        guest.call("web_socket_send", socket=identifier, message={"type": "text", "text": "M119 guest scoped text"})
        assert guest.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned-target:M119 guest scoped text"}
        return identifier

    def ended(self, guest, provider, target, identifier, expires=None):
        window_started = None
        invalid_windows = 0
        def settled():
            nonlocal window_started, invalid_windows
            status = provider.call("web_status")
            wall, monotonic = time.time(), time.monotonic()
            self.record("controller", "guest_end_owner_observation", {"wall": wall, "monotonic": monotonic,
                        "target_active_sockets": len(target.active), "provider_active_flows": status["active_flows"]})
            if expires is not None:
                if wall <= expires:
                    if window_started is not None:
                        invalid_windows += 1
                        self.record("controller", "expiry_window_invalidated_by_wall_rollback", {
                            "wall": wall, "expires": expires, "continuous_expired_seconds": monotonic-window_started})
                    window_started = None
                    return False
                if window_started is None:
                    window_started = monotonic
                assert monotonic-window_started < 10, "current expiry held for ten seconds without closing Web owners"
            return not target.active and status["active_flows"] == 0
        self.until(settled,
                   "ended guest kept real WebSocket/provider owner", seconds=100 if expires is not None else 10)
        if expires is not None:
            self.record("controller", "qualified_current_expiry_close_window", {"expires": expires,
                        "invalid_observation_windows": invalid_windows, "close_bound_seconds": 10,
                        "continuous_current_expiry_seconds": time.monotonic()-window_started})
            # Only the subsequent negative-operation premise gets this margin
            # after the observed 16s host rollback. The close assertion above
            # remains ten seconds of continuously true current expiry.
            self.until(lambda: time.time() > expires + 20, "stable expiry premise for new access denial", seconds=100)
        outcome = guest.call("web_socket_read", socket=identifier, timeout_ms=2000)
        assert outcome["closed"] and outcome.get("failed"), outcome
        assert guest.call("web_socket_close", socket=identifier)["failed"]
        self.denied(guest, provider, target)

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        provider = OwnedNode(self, "provider")
        member = self.member(provider, "member")
        provider.call("web_enable", targets=[target.url])
        response = member.call("web_open", address=provider.address, url=target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        assert target.requests == ["/"]
        self.record("controller", "web_guest_preconditions", {"fresh_independent_member": True,
                    "public_web_currently_enabled": True, "real_target_exact_response": True})
        guest, status, invitation = self.guest(provider, target, "exit-guest", 40 if self.args.case == "expiry" else 180)
        self.positive(guest, provider, target)
        if self.args.case == "reopen":
            original_identity, original_deadline, directory = guest.identity, status["scope"]["expires"], guest.directory
            self.record("controller", "guest_before_reopen_cleanup", guest.close())
            self.nodes.remove(guest)
            guest = OwnedNode(self, "reopened-web-guest", "open", directory=directory)
            retained = guest.call("guest_redeem")
            assert guest.identity == original_identity and retained["scope"]["expires"] == original_deadline
            assert retained["scope"]["service"] == "web" and time.time() < original_deadline
            self.positive(guest, provider, target)
            identifier = self.live_socket(guest, provider, target)
            ended = guest.call("guest_end")
            assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
            self.ended(guest, provider, target, identifier)
            self.record("controller", "web_guest_saved_identity_deadline_reopen", {"same_identity": True,
                        "same_original_deadline": True, "real_http_socket_after_reopen": True, "exit_no_effect": True})
            return
        identifier = self.live_socket(guest, provider, target)
        if self.args.case == "expiry":
            expires = status["scope"]["expires"]
            assert time.time() < expires, "expiry premise already false during actual guest success"
            self.until(lambda: time.time() > expires and guest.call("guest_status")["stage"] == "GuestExpired",
                       "natural original guest deadline", seconds=100)
            self.ended(guest, provider, target, identifier, expires=expires)
            assert time.time() > expires, "wall rollback invalidated actual expiry denial premise"
            self.record("controller", "web_guest_natural_expiry", {"expires": expires, "wall_after": time.time(),
                        "http_and_websocket_before": True, "live_socket_closed": True, "no_effect_after": True})
            return
        duplicate = OwnedNode(self, "other-identity", "join-guest", invitation)
        duplicate.call("guest_redeem", ok=False)
        self.denied(duplicate, provider, target)
        outsider = OwnedNode(self, "different-provider")
        other_target = HttpTarget(self)
        self.denied(guest, outsider, target)
        self.denied(guest, provider, other_target)
        guest.call("web_enable", targets=[target.url], ok=False)
        guest.call("ping", address=provider.address, ok=False)
        guest.call("forward_start", protocol="tcp", address=provider.address, target="127.0.0.1:1", bind="127.0.0.1:0", ok=False)
        time.sleep(2.05)  # guest ping refusal uses the existing peer contact budget
        before = len(target.requests)
        ended = guest.call("guest_end")
        assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
        self.ended(guest, provider, target, identifier)
        assert len(target.requests) == before
        revoked, scope, _ = self.guest(provider, target, "revoked-guest")
        self.positive(revoked, provider, target)
        identifier = self.live_socket(revoked, provider, target)
        provider.call("guest_revoke", invitation=scope["scope"]["invitation"])
        self.ended(revoked, provider, target, identifier)
        self.record("controller", "scoped_web_guest_lifecycle", {"identity_binding": True, "other_provider_denied": True,
                    "other_service_denied": True, "other_target_denied": True, "not_member_or_provider": True,
                    "exit_and_revoke_close_real_socket": True, "http_and_websocket_no_effect_after": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="guests")
    GuestRunner(parser.parse_args()).run()
