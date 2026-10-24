#!/usr/bin/env python3
"""Fresh scoped forwarding guests: real effects, binding, exit/revoke/expiry."""
import argparse
from pathlib import Path
import socket
import time
from check_forwarding import Node, Target
from check_forwarding_matrix import MatrixRunner


class GuestRunner(MatrixRunner):
    def guest(self, provider, label, protocol, seconds=180, check_pending=True):
        invitation = self.private / (label + "-invitation")
        provider.call("guest_issue", provider=provider.address, service=protocol + "_forward", seconds=seconds, path=str(invitation))
        guest = Node(self, label, "join-guest", invitation)
        if check_pending:
            guest.call("forward_start", ok=False, address=provider.address, protocol=protocol, target=self.current_target.address, bind="127.0.0.1:0")
        status = guest.call("guest_redeem")
        assert status["stage"] == "GuestActive" and status["scope"]["service"] == protocol + "_forward"
        assert status["scope"]["guest"] == guest.identity and status["scope"]["provider"] == provider.identity
        assert guest.identity not in [member["node"] for member in provider.call("status")["members"]]
        return guest, status, invitation

    def guests(self):
        provider = Node(self, "provider")
        outsider = Node(self, "different-provider")
        for protocol in self.protocols:
            target = Target(self, protocol, prefix=b"")
            self.current_target = target
            self.target_precondition(target)
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            guest, status, invitation = self.guest(provider, protocol + "-exit-guest", protocol)
            duplicate = Node(self, protocol + "-second-identity", "join-guest", invitation)
            duplicate.call("guest_redeem", ok=False)
            guest.call("forward_start", ok=False, address=provider.address, protocol="udp" if protocol == "tcp" else "tcp", target=target.address, bind="127.0.0.1:0")
            guest.call("forward_start", ok=False, address=outsider.address, protocol=protocol, target=target.address, bind="127.0.0.1:0")
            guest.call("forward_enable", ok=False, protocol=protocol, targets=[target.address])
            guest.call("ping", ok=False, address=provider.address)
            time.sleep(2.05)
            self.positive(guest, provider, protocol, target)
            ended = []
            self.stopped_effect(guest, provider, protocol, target,
                                lambda: ended.append(guest.call("guest_end")), "guest_exit_ends_active_forward")
            assert ended[0]["ended_locally"] and all(peer["saved"] for peer in ended[0]["delivery"])
            guest.call("forward_start", ok=False, address=provider.address, protocol=protocol, target=target.address, bind="127.0.0.1:0")
            revoked, scope, _ = self.guest(provider, protocol + "-revoked-guest", protocol)
            self.stopped_effect(revoked, provider, protocol, target,
                                lambda: provider.call("guest_revoke", invitation=scope["scope"]["invitation"]), "guest_revoke_ends_active_forward")
            self.negative(revoked, provider, protocol, target, "revoked_guest_new_access_has_no_effect")
            self.record("controller", "scoped_guest_lifecycle", {"protocol": protocol, "identity_bound": True,
                        "not_member": True, "other_service_denied": True, "other_provider_denied": True,
                        "guest_not_provider": True, "member_ping_denied": True, "actual_allowed_target": True,
                        "exit_and_revoke_end_effect": True})

    def expiry(self):
        provider = Node(self, "provider")
        for protocol in self.protocols:
            target = Target(self, protocol, prefix=b"")
            self.current_target = target
            self.target_precondition(target)
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            guest, status, _ = self.guest(provider, protocol + "-expiring-guest", protocol, seconds=40)
            expires = status["scope"]["expires"]
            forwarding = self.start_listener(guest, provider, protocol, target)
            with self.application(forwarding) as app:
                assert time.time() < expires, "grant must still be live at the actual success operation"
                assert self.exchange(app, protocol, b"owned-before-expiry") == b"owned-before-expiry"
                before = len(target.received)
                self.until(lambda: time.time() > expires + 0.1 and guest.call("guest_status")["stage"] == "GuestExpired",
                           "actual wall-clock expiry prerequisite", timeout=100)
                self.until(lambda: provider.call("forward_status")["active_flows"] == 0, "expiry closes actual target association")
                # The inherited host clock can roll back about 16 seconds.
                # This is an observation margin for the negative test only:
                # the provider was already required to stop at first expiry.
                # It does not change the grant, product clock or a grace rule.
                self.until(lambda: time.time() > expires + 20 and guest.call("guest_status")["stage"] == "GuestExpired",
                           "stable current expiry premise after observed host rollback", timeout=100)
                wall_before = time.time()
                monotonic = time.monotonic()
                assert wall_before > expires
                app.settimeout(1)
                try:
                    app.send(b"owned-after-expiry")
                    assert app.recv(65536) == b""
                except (ConnectionResetError, ConnectionRefusedError, BrokenPipeError, socket.timeout):
                    pass
                wall_after = time.time()
                self.record("controller", "actual_current_expiry", {"protocol": protocol, "expires": expires,
                            "wall_before": wall_before, "wall_after": wall_after, "monotonic": monotonic,
                            "actual_success_before": True, "no_effect_after": len(target.received) == before, "provider_flows_zero": True})
                assert wall_after > expires, "wall-clock rollback invalidated the expiry prerequisite"
                assert len(target.received) == before
            guest.call("forward_stop", task=forwarding["id"])
            assert time.time() > expires
            guest.call("forward_start", ok=False, address=provider.address, protocol=protocol, target=target.address, bind="127.0.0.1:0")
            assert time.time() > expires, "rollback invalidated new access denial premise"

    def scenario(self):
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(__file__).with_name("check_forwarding_matrix.py"))
        if getattr(self.args, "relay_binary", None):
            from forwarding_local_resources import configure_local_resources
            configure_local_resources(self, self.args)
        self.protocols = ["tcp", "udp"] if self.selected_protocol == "both" else [self.selected_protocol]
        getattr(self, self.case)()

    def __init__(self, args):
        super().__init__(args)
        self.selected_protocol = args.protocol


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["guests", "expiry"], required=True)
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--protocol", choices=["both", "tcp", "udp"], default="both")
    parser.add_argument("--relay-binary", type=Path)
    parser.add_argument("--relay-receipt", type=Path)
    GuestRunner(parser.parse_args()).run()
