#!/usr/bin/env python3
"""Real WASM forwarding authorization, bounded pending work and service guests."""
import argparse
import json
from pathlib import Path
import socket
import time

from check_forwarding import Node, Target
from check_forwarding_browser import Browser, BrowserRunner
from check_forwarding_matrix import MatrixRunner
from forwarding_local_resources import configure_local_resources


class ControlledTarget(Target):
    def __init__(self, runner, protocol):
        self.respond = False
        super().__init__(runner, protocol, prefix=b"")

    def run(self):
        try:
            while not self.stop.is_set():
                if self.protocol == "udp":
                    try:
                        data, address = self.socket.recvfrom(65536)
                    except socket.timeout:
                        continue
                    self.received.append(data)
                    if self.respond:
                        self.socket.sendto(data, address)
                else:
                    try:
                        connection, _ = self.socket.accept()
                    except socket.timeout:
                        continue
                    self.connections.append(connection)
                    connection.settimeout(0.1)
                    with connection:
                        while not self.stop.is_set():
                            try:
                                data = connection.recv(65536)
                            except socket.timeout:
                                continue
                            except ConnectionResetError:
                                break
                            if not data:
                                break
                            self.received.append(data)
                            if self.respond:
                                connection.sendall(data)
        except BaseException as failure:
            self.errors.append(type(failure).__name__)


class ScopeRunner(BrowserRunner, MatrixRunner):
    def browser_member(self, provider):
        browser = Browser(self, self.args)
        browser.call("open")
        invitation = self.private / "browser-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = browser.call("start", "join", self.configuration, invitation.read_text())
        assert browser.call("request_join", False) == "Complete"
        capability = browser.call("forwarding_status")
        assert capability["native_listener"] is False and capability["provider"] is False
        assert capability["bounded_exchange"] is True
        return browser, status["id"]

    def scopes(self):
        provider = Node(self, "provider")
        browser, identity = self.browser_member(provider)
        for protocol in ["tcp", "udp"]:
            target = Target(self, protocol, prefix=b"")
            other = Target(self, protocol, prefix=b"")
            self.target_precondition(target)
            self.target_precondition(other)
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            assert browser.call("forward_exchange", provider.identity, protocol, target.address, 17, 5)["exact"]
            revision = provider.call("service_rule", provider=provider.identity, service=protocol + "_forward", previous=None,
                                     rule=dict(members=True, guests=True, allowed=[], denied=[identity]))["revision"]
            before = len(target.received)
            browser.call("forward_exchange", provider.identity, protocol, target.address, 19, 7, ok=False)
            assert len(target.received) == before
            provider.call("service_rule", provider=provider.identity, service=protocol + "_forward", previous=revision,
                          rule=dict(members=True, guests=True, allowed=[], denied=[]))
            assert browser.call("forward_exchange", provider.identity, protocol, target.address, 23, 11)["exact"]
            provider.call("forward_enable", protocol=protocol, targets=[other.address])
            before = len(target.received)
            browser.call("forward_exchange", provider.identity, protocol, target.address, 29, 13, ok=False)
            assert len(target.received) == before
            assert browser.call("forward_exchange", provider.identity, protocol, other.address, 31, 17)["exact"]
            self.record("controller", "browser_current_acl_and_scope", {"protocol": protocol, "denied_identity_no_effect": True,
                        "explicit_repair_same_operation": True, "old_scope_no_effect": True, "current_scope_works": True})
            pending_target = ControlledTarget(self, protocol)
            provider.call("forward_enable", protocol=protocol, targets=[pending_target.address])
            count = 4 if protocol == "udp" else 1
            for index in range(count):
                browser.call("begin_forward_exchange", index, provider.identity, protocol, pending_target.address, 37, index)
                self.until(lambda: len(pending_target.received) == index + 1, "actual pending request reached owned target", timeout=75)
            assert browser.call("forwarding_status")["active_exchanges"] == count
            if count == 4:
                before = len(pending_target.received)
                browser.call("forward_exchange", provider.identity, protocol, pending_target.address, 41, 1, ok=False)
                assert len(pending_target.received) == before
            started = time.monotonic()
            browser.call("stop_forwarding")
            stopped_seconds = time.monotonic() - started
            assert stopped_seconds < 5 and browser.call("forwarding_status")["active_exchanges"] == 0
            for index in range(count):
                assert browser.call("finish_forward_exchange", index) == dict(ok=False)
            self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "browser stop joins provider effects")
            pending_target.respond = True
            assert browser.call("forward_exchange", provider.identity, protocol, pending_target.address, 43, 3)["exact"]
            self.record("controller", "browser_actual_pending_stop_bound", {"protocol": protocol, "actual_target_received": count,
                        "concurrent": count, "fifth_rejected": count == 4, "stop_seconds": stopped_seconds, "all_owners_joined": True, "explicit_new_operation_works": True})

    def guest(self):
        provider = Node(self, "provider")
        browser = Browser(self, self.args)
        for protocol in ["tcp", "udp"]:
            actor = protocol + "-guest"
            browser.call("open", actor=actor)
            target = Target(self, protocol, prefix=b"")
            self.target_precondition(target)
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            invitation = self.private / (actor + "-invitation")
            provider.call("guest_issue", provider=provider.address, service=protocol + "_forward", seconds=240, path=str(invitation))
            status = browser.call("start", "join_guest", self.configuration, invitation.read_text(), actor=actor)
            browser.call("forward_exchange", provider.identity, protocol, target.address, 17, 1, ok=False, actor=actor)
            scope = browser.call("redeem_guest", actor=actor)
            assert scope["scope"]["service"] == protocol + "_forward" and scope["scope"]["guest"] == status["id"]
            assert browser.call("forward_exchange", provider.identity, protocol, target.address, 19, 3, actor=actor)["exact"]
            browser.call("forward_exchange", provider.identity, "udp" if protocol == "tcp" else "tcp", target.address, 23, 5, ok=False, actor=actor)
            assert browser.call("forwarding_status", actor=actor)["native_listener"] is False
            ended = browser.call("end_guest", actor=actor)
            assert ended["ended_locally"] and all(peer["saved"] for peer in ended["delivery"])
            before = len(target.received)
            browser.call("forward_exchange", provider.identity, protocol, target.address, 29, 7, ok=False, actor=actor)
            assert len(target.received) == before
            self.record("controller", "actual_browser_scoped_guest", {"protocol": protocol, "before_redeem_denied": True,
                        "actual_effect_after_redeem": True, "other_service_denied": True, "exit_no_effect": True, "no_native_listener": True})

    def browser_creator_and_native(self):
        browser = Browser(self, self.args)
        browser.call("open")
        status = browser.call("start", "create", self.configuration, None)
        card = self.private / "browser-card"
        card.write_text(browser.call("export_recovery_card", None))
        card.chmod(0o600)
        browser.call("acknowledge_recovery_saved")
        invitation = self.private / "native-invitation"
        invitation.write_text(browser.call("invitation"))
        invitation.chmod(0o600)
        native = Node(self, "native-provider", "join", invitation)
        browser.call("begin_serve_admission")
        assert native.call("request", path=str(invitation))["stage"] == "AwaitingApproval"
        assert browser.call("finish_serve_admission") == "AwaitingApproval"
        pending = browser.call("admission_requests")
        request = next(item for item in pending if item["applicant"] == native.identity)
        browser.call("approve_admission", native.identity, request["request"])
        browser.call("begin_serve_admission")
        assert native.call("request", path=str(invitation))["stage"] == "Complete"
        assert browser.call("finish_serve_admission") == "Complete"
        return browser, status["id"], native

    def admin(self):
        browser, _, provider = self.browser_creator_and_native()
        for protocol in ["tcp", "udp"]:
            target = Target(self, protocol, prefix=b"")
            self.target_precondition(target)
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            browser.call("sync_services", provider.identity)
            invitation = self.private / (protocol + "-browser-issued-invitation")
            invitation.write_text(browser.call("issue_service_guest", json.dumps(provider.address), protocol + "_forward", int(time.time()) + 300))
            invitation.chmod(0o600)
            guest = Node(self, protocol + "-native-guest", "join-guest", invitation)
            status = guest.call("guest_redeem")
            assert status["scope"]["service"] == protocol + "_forward"
            duplicate = Node(self, protocol + "-duplicate-guest", "join-guest", invitation)
            duplicate.call("guest_redeem", ok=False)
            browser.call("sync_services", provider.identity)
            self.positive(guest, provider, protocol, target)
            revisions = []
            def deny():
                revisions.append(browser.call("set_service_rule", provider.identity, protocol + "_forward", None,
                                 json.dumps(dict(members=True, guests=True, allowed=[], denied=[guest.identity]))))
                browser.call("sync_services", provider.identity)
            self.stopped_effect(guest, provider, protocol, target, deny, "browser_admin_denies_active_native_guest")
            self.negative(guest, provider, protocol, target, "browser_admin_rule_enforced_on_native_target")
            browser.call("set_service_rule", provider.identity, protocol + "_forward", revisions[0],
                         json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
            browser.call("sync_services", provider.identity)
            assert guest.call("guest_redeem")["stage"] == "GuestActive"
            self.positive(guest, provider, protocol, target)
            def revoke():
                browser.call("revoke_guest", status["scope"]["invitation"])
                browser.call("sync_services", provider.identity)
            self.stopped_effect(guest, provider, protocol, target, revoke, "browser_issuer_revoke_ends_active_native_forward")
            self.record("controller", "browser_issuer_native_guest", {"protocol": protocol, "single_identity": True,
                        "actual_native_effect": True, "remote_single_acl": True, "repair": True, "active_revoke": True})

    def expiry(self):
        provider = Node(self, "provider")
        target = ControlledTarget(self, "udp")
        target.respond = True
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        browser = Browser(self, self.args)
        browser.call("open")
        invitation = self.private / "browser-expiring-invitation"
        provider.call("guest_issue", provider=provider.address, service="udp_forward", seconds=55, path=str(invitation))
        browser.call("start", "join_guest", self.configuration, invitation.read_text())
        scope = browser.call("redeem_guest")["scope"]
        assert time.time() < scope["expires"]
        assert browser.call("forward_exchange", provider.identity, "udp", target.address, 17, 1)["exact"]
        target.respond = False
        browser.call("begin_forward_exchange", 0, provider.identity, "udp", target.address, 19, 3)
        self.until(lambda: len(target.received) == 2, "browser live pending request reaches target", timeout=75)
        assert time.time() < scope["expires"] and browser.call("forwarding_status")["active_exchanges"] == 1
        self.until(lambda: time.time() > scope["expires"] and browser.call("guest_status")["stage"] == "GuestExpired", "actual browser expiry", timeout=110)
        self.until(lambda: browser.call("forwarding_status")["active_exchanges"] == 0 and provider.call("forward_status")["active_flows"] == 0,
                   "current expiry cancels pending exchange and provider")
        assert browser.call("finish_forward_exchange", 0) == dict(ok=False)
        self.until(lambda: time.time() > scope["expires"] + 20, "stable current wall expiry after known rollback", timeout=100)
        before = len(target.received)
        wall_before = time.time()
        browser.call("forward_exchange", provider.identity, "udp", target.address, 23, 7, ok=False)
        wall_after = time.time()
        self.record("controller", "actual_browser_guest_expiry", {"expires": scope["expires"], "wall_before": wall_before,
                    "wall_after": wall_after, "actual_pending_cancelled": True, "target_unchanged": len(target.received) == before})
        assert wall_before > scope["expires"] and wall_after > scope["expires"] and len(target.received) == before

    def scenario(self):
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(__file__).with_name("check_forwarding_matrix.py"))
        configure_local_resources(self, self.args)
        getattr(self, self.case)()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", choices=["scopes", "guest", "admin", "expiry"], required=True)
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    ScopeRunner(parser.parse_args()).run()
