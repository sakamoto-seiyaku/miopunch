#!/usr/bin/env python3
"""Real isolated-system clipboard under the existing scoped guest authority."""
import argparse
from pathlib import Path
import time

from check_clipboard import Desktop, Host, Runner


class GuestRunner(Runner):
    def scenario(self):
        provider_desktop, guest_desktop = Desktop(self, "guest-provider"), Desktop(self, "clipboard-guest")
        provider_desktop.copy("M117 owned guest provider baseline")
        guest_desktop.copy("M117 owned guest baseline")
        provider = Host(self, "guest-provider", provider_desktop)
        provider.call("clipboard_enable")
        invitation = self.private / "clipboard-invitation"
        provider.call("guest_issue", path=str(invitation), provider=provider.address, service="clipboard", seconds=90)
        guest = Host(self, "clipboard-guest", guest_desktop, "join-guest", invitation)
        pending = guest.call("guest_status")
        assert pending["scope"]["service"] == "clipboard", "public invitation did not retain its selected Clipboard scope"
        assert pending["stage"] == "GuestPending"
        guest.call("clipboard_enable")
        guest.call("clipboard_start", ok=False, address=provider.address)
        assert guest.call("guest_redeem")["stage"] == "GuestActive"
        second_desktop = Desktop(self, "clipboard-second-identity")
        second_desktop.copy("M117 owned unbound identity baseline")
        second = Host(self, "clipboard-second-identity", second_desktop, "join-guest", invitation)
        second.call("guest_redeem", ok=False)
        assert guest.node not in [member["node"] for member in provider.call("status")["members"]]
        guest.call("clipboard_start", address=provider.address)
        first = "M117 owned scoped guest→provider 文本"
        guest_desktop.copy(first)
        self.until(lambda: provider_desktop.read() == first, "actual scoped guest write")
        self.until(lambda: guest.call("clipboard_status")["acknowledged"] == 1, "guest actual write acknowledgment")
        reverse = "M117 owned scoped provider→guest 文本"
        provider_desktop.copy(reverse)
        self.until(lambda: guest_desktop.read() == reverse, "actual scoped provider write")
        self.until(lambda: provider.call("clipboard_status")["acknowledged"] == 1, "provider actual write acknowledgment")
        revision = provider.call("service_rule", provider=provider.node, service="clipboard", previous=None,
                                 rule=dict(members=True, guests=True, allowed=[], denied=[guest.node]))["revision"]
        self.until(lambda: provider.call("clipboard_status")["peer"] is None, "current scoped guest ACL retires pair")
        self.until(lambda: guest.call("clipboard_status")["peer"] is None, "ACL closes actual guest connection")
        guest_desktop.copy("M117 owned after scoped Clipboard ACL denial")
        time.sleep(.8)
        assert provider_desktop.read() == reverse
        guest.call("clipboard_start", ok=False, address=provider.address)
        provider.call("service_rule", provider=provider.node, service="clipboard", previous=revision,
                      rule=dict(members=True, guests=True, allowed=[], denied=[]))
        # Public SDK observation in lifecycle-full-4 confirms the existing
        # first failed-contact retry-after is <=1s. Respect it once; retain the
        # original immediate-redeem failure and the same scope/id assertions.
        time.sleep(1.05)
        resumed_scope = guest.call("guest_redeem")
        assert resumed_scope["stage"] == "GuestActive" and resumed_scope["scope"]["expires"] == pending["scope"]["expires"]
        assert resumed_scope["scope"]["guest"] == guest.node
        sample = self.private / "clipboard-guest-not-files"
        sample.write_bytes(b"M117 owned File scope negative")
        guest.call("file_send", ok=False, address=provider.address, path=str(sample))
        other_desktop = Desktop(self, "guest-other-provider")
        other_desktop.copy("M117 owned other provider unchanged")
        other = self.member(provider, "guest-other-provider", other_desktop)
        other.call("clipboard_enable")
        guest.call("clipboard_stop")
        guest.call("clipboard_start", ok=False, address=other.address)
        assert other_desktop.read() == "M117 owned other provider unchanged"
        guest.call("clipboard_start", address=provider.address)
        ended = guest.call("guest_end")
        assert ended["ended_locally"] and all(item["saved"] for item in ended["delivery"])
        self.until(lambda: provider.call("clipboard_status")["peer"] is None, "guest exit retires provider pair")
        self.until(lambda: guest.call("clipboard_status")["peer"] is None, "guest exit retires local pair")
        guest_desktop.copy("M117 owned after guest exit")
        time.sleep(.8)
        assert provider_desktop.read() == reverse
        guest.call("clipboard_start", ok=False, address=provider.address)
        guest.call("guest_redeem", ok=False)
        guest.call("ping", ok=False, address=provider.address)
        self.record("guest", "scope_duplex_exit", {"actual_system_duplex": True, "no_ordinary_membership": True,
                    "second_identity_rejected": True, "no_file_scope": True, "ping_rejected_after_exit": True, "other_provider_rejected": True,
                    "current_acl_stops_effects": True, "exit_stops_effects": True})
        file_desktop = Desktop(self, "files-only-guest")
        file_desktop.copy("M117 owned Files-only guest marker")
        file_invitation = self.private / "files-only-invitation"
        # The original public operation remains a Files-only grant.
        provider.call("guest_issue", path=str(file_invitation), provider=provider.address, seconds=90)
        files_guest = Host(self, "files-only-guest", file_desktop, "join-guest", file_invitation)
        assert files_guest.call("guest_redeem")["scope"]["service"] == "files"
        files_guest.call("clipboard_enable")
        files_guest.call("clipboard_start", ok=False, address=provider.address)
        assert provider_desktop.read() == reverse
        for ending in ["expiry", "revoke"]:
            desktop = Desktop(self, ending + "-guest")
            desktop.copy("M117 owned " + ending + " guest baseline")
            path = self.private / (ending + "-invitation")
            provider.call("guest_issue", path=str(path), provider=provider.address, service="clipboard",
                          seconds=20 if ending == "expiry" else 90)
            temporary = Host(self, ending + "-guest", desktop, "join-guest", path)
            temporary.call("clipboard_enable")
            scope = temporary.call("guest_redeem")["scope"]
            temporary.call("clipboard_start", address=provider.address)
            value = "M117 owned actual " + ending + " scoped guest write"
            desktop.copy(value)
            self.until(lambda: provider_desktop.read() == value, "actual before " + ending)
            self.until(lambda: temporary.call("clipboard_status")["acknowledged"] == 1, "write acknowledgment before " + ending)
            if ending == "revoke":
                provider.call("guest_revoke", invitation=scope["invitation"])
            else:
                # Ordinary wall-clock expiration; no clock manipulation or
                # authority injection. Require real success before waiting.
                self.until(lambda: temporary.call("guest_status")["stage"] == "GuestExpired", "natural guest deadline", 25)
            self.until(lambda: provider.call("clipboard_status")["peer"] is None, ending + " stops provider")
            self.until(lambda: temporary.call("clipboard_status")["peer"] is None, ending + " stops guest")
            desktop.copy("M117 owned after guest " + ending)
            time.sleep(.8)
            assert provider_desktop.read() == value
            if ending == "expiry":
                # Real CLOCK_REALTIME was observed moving backwards while
                # monotonic/boottime advanced. Re-establish this operation's
                # actual current deadline premise without retrying business.
                self.until(lambda: temporary.call("guest_status")["stage"] == "GuestExpired"
                           and time.time() >= scope["expires"] + 1, "current expired-clock prerequisite", 45)
                self.record("expiry", "current_deadline_prerequisite", {"scope": scope,
                            "current_wall_at_least_deadline": time.time() >= scope["expires"]})
            temporary.call("clipboard_start", ok=False, address=provider.address)
            self.record(ending, "scoped_authority_ends_system_effects", {"actual_before": True, "no_write_after": True,
                        "ordinary_deadline_or_signed_revocation": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.case = "guests"
    GuestRunner(args).run()


if __name__ == "__main__":
    main()
