#!/usr/bin/env python3
"""Real current terminal ACL/guest effects on fresh private tmux workspaces."""
import argparse
from pathlib import Path
import time

from check_terminal_service import TerminalRunner, WorkspaceObserver
from check_web_service import HttpTarget, OwnedNode
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


class PolicyRunner(TerminalRunner):
    def setup(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        self.provider = OwnedNode(self, "provider")
        self.consumer = self.member(self.provider, "consumer")
        self.directory = self.private / "terminal-policy"
        assert self.consumer.call("ping", address=self.provider.address)["reply"] == "PONG"
        self.observer = WorkspaceObserver(self, self.directory)

    def snapshot(self):
        return sorted(path.name for path in self.directory.glob("mio-*"))

    def no_new_access(self, node, provider=None, workspace=None):
        provider = provider or self.provider
        before = self.snapshot()
        node.call("terminal_create", address=provider.address, ok=False)
        node.call("terminal_attach", address=provider.address, workspace=workspace or self.workspace, ok=False)
        assert self.snapshot() == before, "denied terminal operation created a workspace"
        time.sleep(2.05)  # preserve existing refused-contact backoff

    def attach(self, node, workspace=None):
        workspace = workspace or self.workspace
        identifier = node.call("terminal_attach", address=self.provider.address, workspace=workspace)["id"]
        self.send(node, identifier, "stty -echo; printf 'POLICY_'; printf 'READY\\n'\n")
        self.read_until(node, identifier, b"POLICY_READY")
        return identifier

    def effect(self, node, identifier, filename, value):
        self.send(node, identifier, "printf '%s' " + value + " > " + filename + "; printf 'EFFECT_'; printf 'COMPLETE\\n'\n")
        self.read_until(node, identifier, b"EFFECT_COMPLETE")
        assert (self.directory/self.workspace/filename).read_bytes() == value.encode()

    def enable(self):
        self.provider.call("terminal_enable", directory=str(self.directory))
        self.workspace = self.consumer.call("terminal_create", address=self.provider.address)["workspace"]
        self.observer.observe_server()

    def ended_attachment(self, node, identifier, *, expires=None):
        window_started, invalid_windows = None, 0
        def settled():
            nonlocal window_started, invalid_windows
            status = self.provider.call("terminal_status")
            wall, now = time.time(), time.monotonic()
            if expires is not None:
                if wall <= expires:
                    if window_started is not None:
                        invalid_windows += 1
                    window_started = None
                    return False
                if window_started is None:
                    window_started = now
                assert now-window_started < 10, "ten continuously expired seconds retained terminal owners"
            return status["active_operations"] == 0 and status["attachment_process_owners"] == 0
        self.until(settled, "current terminal access end retained attachment/process owners", seconds=100 if expires else 10)
        outcome = node.call("terminal_read", attachment=identifier, timeout_ms=100)
        assert not outcome["attached"] and outcome.get("failed"), outcome
        before = self.directory/self.workspace/"forbidden"
        assert not before.exists()
        node.call("terminal_send", attachment=identifier, message={"type":"bytes","data":list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not before.exists()
        node.call("terminal_cancel", attachment=identifier)
        if expires is not None:
            self.record("controller", "continuous_terminal_expiry_window", {"expires":expires,"invalid_windows":invalid_windows,
                        "elapsed_expired":time.monotonic()-window_started,"owner_close_bound":10})
            self.until(lambda: time.time() > expires+20, "stable current expiry before new denied operations", seconds=100)

    def new_guest(self, label, seconds=180):
        invitation = self.private/(label+"-invitation")
        self.provider.call("guest_issue", provider=self.provider.address, service="terminal", seconds=seconds, path=str(invitation))
        guest = OwnedNode(self, label, "join-guest", invitation)
        self.no_new_access(guest)
        status = guest.call("guest_redeem")
        assert status["stage"] == "GuestActive" and status["scope"]["service"] == "terminal"
        assert status["scope"]["provider"] == self.provider.identity and status["scope"]["guest"] == guest.identity
        assert guest.identity not in [row["node"] for row in self.provider.call("status")["members"]]
        return guest, status, invitation

    def policy(self):
        assert not self.provider.call("terminal_status")["enabled"]
        self.workspace = "mio-"+"0"*32
        self.no_new_access(self.consumer)
        assert not self.directory.exists()
        self.enable()
        identifier = self.attach(self.consumer)
        self.effect(self.consumer, identifier, "member-positive", "owned-member")
        self.provider.call("terminal_disable")
        self.ended_attachment(self.consumer, identifier)
        self.no_new_access(self.consumer)
        self.provider.call("terminal_enable", directory=str(self.directory))
        rule = {"members":False,"guests":True,"allowed":[],"denied":[]}
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal", previous=None, rule=rule)["revision"]
        self.no_new_access(self.consumer)
        rule["allowed"] = [self.consumer.identity]
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=rule)["revision"]
        identifier = self.attach(self.consumer)
        self.effect(self.consumer, identifier, "explicit-allow", "owned-allow")
        rule["denied"] = [self.consumer.identity]
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=rule)["revision"]
        self.ended_attachment(self.consumer, identifier)
        self.no_new_access(self.consumer)
        rule = {"members":True,"guests":True,"allowed":[],"denied":[]}
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=rule)["revision"]
        admin = self.member(self.provider, "future-admin")
        admin.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=rule, ok=False)
        operation = self.provider.call("genesis_propose", action="appoint-administrator", target=admin.identity)["operation"]["id"]
        self.provider.call("genesis_confirm", operation=operation)
        self.provider.call("service_sync", address=admin.address)
        identifier = self.attach(self.consumer)
        self.effect(self.consumer, identifier, "before-remote-admin", "owned-before-admin")
        denied = dict(rule, denied=[self.consumer.identity])
        revision = admin.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=denied)["revision"]
        admin.call("service_sync", address=self.provider.address)
        self.ended_attachment(self.consumer, identifier)
        self.no_new_access(self.consumer)
        revision = admin.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=rule)["revision"]
        admin.call("service_sync", address=self.provider.address)
        identifier = self.attach(self.consumer)
        self.effect(self.consumer, identifier, "restored-remote-admin", "owned-restored")
        operation = self.provider.call("genesis_propose", action="demote-administrator", target=admin.identity)["operation"]["id"]
        self.provider.call("genesis_confirm", operation=operation)
        self.provider.call("service_sync", address=admin.address)
        admin.call("service_rule", provider=self.provider.identity, service="terminal", previous=revision, rule=denied, ok=False)
        operation = self.provider.call("genesis_propose", action="remove-member", target=self.consumer.identity)["operation"]["id"]
        self.provider.call("device_execute", operation=operation)
        self.ended_attachment(self.consumer, identifier)
        self.no_new_access(self.consumer)
        assert self.workspace in admin.call("terminal_list", address=self.provider.address)["workspaces"]
        admin.call("terminal_end", address=self.provider.address, workspace=self.workspace)
        self.record("controller", "actual_terminal_current_acl_and_management", {"off_no_effect":True,"default_member":True,
            "explicit_allow_and_deny":True,"one_effective_rule":True,"remote_admin_deny_restore":True,
            "demotion_stops_management_authority":True,"member_removal_closes_actual_attachment":True,
            "retained_workspace_distinct_from_access":True})

    def guests(self):
        self.enable()
        member_attachment = self.attach(self.consumer)
        self.effect(self.consumer, member_attachment, "premise", "owned-guest-premise")
        assert not self.consumer.call("terminal_detach", attachment=member_attachment)["failed"]
        guest, status, invitation = self.new_guest("terminal-guest", 25 if self.case == "expiry" else 180)
        deadline = status["scope"]["expires"]
        identifier = self.attach(guest)
        self.effect(guest, identifier, "guest-positive", "owned-terminal-guest")
        if self.case == "expiry":
            assert time.time() < deadline
            self.until(lambda: time.time() > deadline and guest.call("guest_status")["stage"] == "GuestExpired",
                       "natural original terminal guest deadline", seconds=100)
            self.ended_attachment(guest, identifier, expires=deadline)
            assert time.time() > deadline
            self.no_new_access(guest)
            assert time.time() > deadline
            self.record("controller", "actual_terminal_guest_expiry", {"original_deadline":deadline,"current_expired":True,
                        "real_command_before":True,"live_attachment_closed":True,"new_effects_denied":True})
            return
        if self.case == "reopen":
            original, directory = guest.identity, guest.directory
            self.send(guest, identifier, "printf 'started' > reconnect-start; (sleep 1; printf 'retained' > reconnect-done) &\n")
            self.until((self.directory/self.workspace/"reconnect-start").exists, "reconnect job not started")
            self.record("controller", "original_guest_close", guest.close())
            self.nodes.remove(guest)
            self.until((self.directory/self.workspace/"reconnect-done").exists, "guest disconnect ended retained workspace")
            guest = OwnedNode(self, "reopened-terminal-guest", "open", directory=directory)
            reopened = guest.call("guest_redeem")
            assert guest.identity == original and reopened["scope"]["expires"] == deadline and time.time() < deadline
            identifier = self.attach(guest)
            self.effect(guest, identifier, "reopened-positive", "owned-reopened-guest")
        else:
            duplicate = OwnedNode(self, "different-guest-identity", "join-guest", invitation)
            duplicate.call("guest_redeem", ok=False)
            self.no_new_access(duplicate)
            target = HttpTarget(self)
            self.provider.call("web_enable", targets=[target.url])
            before = len(target.requests)
            guest.call("web_open", address=self.provider.address, url=target.url, ok=False)
            assert len(target.requests) == before
            guest.call("ping", address=self.provider.address, ok=False)
            guest.call("terminal_enable", directory=str(self.private/"guest-must-not-provide"), ok=False)
            assert not (self.private/"guest-must-not-provide").exists()
            guest.call("service_rule", provider=self.provider.identity, service="terminal", previous=None,
                       rule={"members":True,"guests":True,"allowed":[],"denied":[]}, ok=False)
            time.sleep(2.05)
        ended = guest.call("guest_end")
        assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
        self.ended_attachment(guest, identifier)
        self.no_new_access(guest)
        if self.case == "reopen":
            self.record("controller", "terminal_guest_original_deadline_reopen", {"same_identity":True,"same_deadline":True,
                        "disconnect_retains_job":True,"real_access_after_reopen":True,"end_stops_access":True})
            return
        revoked, scope, _ = self.new_guest("revoked-terminal-guest")
        identifier = self.attach(revoked)
        self.effect(revoked, identifier, "before-revoke", "owned-revoked-guest")
        self.provider.call("guest_revoke", invitation=scope["scope"]["invitation"])
        self.ended_attachment(revoked, identifier)
        self.no_new_access(revoked)
        assert self.workspace in self.consumer.call("terminal_list", address=self.provider.address)["workspaces"]
        self.consumer.call("terminal_end", address=self.provider.address, workspace=self.workspace)
        self.record("controller", "actual_terminal_scoped_guest_lifecycle", {"single_identity":True,"pending_no_effect":True,
                    "other_service_denied":True,"no_member_or_provider_or_management_grant":True,"end_and_revoke_close_attachment":True,
                    "current_access_distinct_retained_workspace":True})

    def scope(self):
        self.enable()
        other = self.member(self.provider, "other-provider")
        other_directory = self.private/"other-terminal"
        other.call("terminal_enable", directory=str(other_directory))
        self.provider.call("service_sync", address=other.address)
        self.consumer.call("service_sync", address=self.provider.address)
        other_observer = WorkspaceObserver(self, other_directory)
        other_workspace = self.consumer.call("terminal_create", address=other.address)["workspace"]
        other_observer.observe_server()
        attachment = self.consumer.call("terminal_attach", address=other.address, workspace=other_workspace)["id"]
        self.send(self.consumer, attachment, "stty -echo; printf 'actual-other-node' > scope-positive; printf 'OTHER_'; printf 'NODE_READY\\n'\n")
        self.read_until(self.consumer, attachment, b"OTHER_NODE_READY")
        assert (other_directory/other_workspace/"scope-positive").read_bytes() == b"actual-other-node"
        assert not self.consumer.call("terminal_detach", attachment=attachment)["failed"]
        guest, _, _ = self.new_guest("node-scoped-terminal-guest")
        attachment = self.attach(guest)
        self.effect(guest, attachment, "correct-node", "owned-correct-node")
        before = sorted(path.name for path in other_directory.glob("mio-*"))
        self.no_new_access(guest, provider=other, workspace=other_workspace)
        assert sorted(path.name for path in other_directory.glob("mio-*")) == before
        assert other.call("terminal_status")["attachment_process_owners"] == 0
        target = HttpTarget(self)
        self.provider.call("web_enable", targets=[target.url])
        web_invitation = self.private/"web-only-invitation"
        self.provider.call("guest_issue", provider=self.provider.address, service="web", seconds=180, path=str(web_invitation))
        web_guest = OwnedNode(self, "web-only-guest", "join-guest", web_invitation)
        assert web_guest.call("guest_redeem")["scope"]["service"] == "web"
        assert web_guest.call("web_open", address=self.provider.address, url=target.url)["status"] == 200
        assert target.requests == ["/"]
        self.no_new_access(web_guest)
        before = len(target.requests)
        guest.call("web_open", address=self.provider.address, url=target.url, ok=False)
        assert len(target.requests) == before
        ended = guest.call("guest_end")
        assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
        self.ended_attachment(guest, attachment)
        self.consumer.call("terminal_end", address=self.provider.address, workspace=self.workspace)
        self.consumer.call("terminal_end", address=other.address, workspace=other_workspace)
        self.record("controller", "actual_terminal_node_service_scope", {"other_provider_actual_pty_premise":True,
                    "terminal_guest_wrong_node_denied":True,"web_guest_actual_http_premise":True,"web_guest_terminal_denied":True,
                    "terminal_guest_web_denied":True,"independent_no_effects":True})

    def scenario(self):
        self.setup()
        if self.case == "policy":
            self.policy()
        elif self.case == "scope":
            self.scope()
        else:
            self.guests()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", choices=["policy","guests","expiry","reopen","scope"], default="policy")
    PolicyRunner(parser.parse_args()).run()
