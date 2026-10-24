#!/usr/bin/env python3
"""An active Clipboard guest has no unrelated member protocol qualification."""
import argparse
from pathlib import Path
import time

from check_clipboard import Desktop, Host, Runner


class IsolationRunner(Runner):
    def scenario(self):
        provider_desktop, guest_desktop = Desktop(self, "scope-provider"), Desktop(self, "scope-guest")
        provider_desktop.copy("M117 own active scope provider baseline")
        guest_desktop.copy("M117 own active scope guest baseline")
        provider = Host(self, "scope-provider", provider_desktop)
        provider.call("clipboard_enable")
        invitation = self.private / "active-scope-invitation"
        provider.call("guest_issue", path=str(invitation), provider=provider.address, service="clipboard", seconds=90)
        guest = Host(self, "scope-guest", guest_desktop, "join-guest", invitation)
        guest.call("clipboard_enable")
        original = guest.call("guest_redeem")
        assert original["stage"] == "GuestActive"
        guest.call("clipboard_start", address=provider.address)
        first = "M117 own real active Clipboard scope write"
        guest_desktop.copy(first)
        self.until(lambda: provider_desktop.read() == first, "actual permitted guest service")
        self.until(lambda: guest.call("clipboard_status")["acknowledged"] == 1, "first scoped write acknowledged")
        assert guest.call("guest_status")["stage"] == "GuestActive"
        assert guest.node not in [member["node"] for member in provider.call("status")["members"]]
        guest.call("ping", ok=False, address=provider.address)
        # Rejection of another ALPN must not end the already qualified pair.
        assert guest.call("guest_status")["stage"] == "GuestActive"
        assert guest.call("clipboard_status")["peer"] == provider.node
        second = "M117 own permitted Clipboard continues after member protocol denial"
        guest_desktop.copy(second)
        self.until(lambda: provider_desktop.read() == second, "same Clipboard operation after unrelated scope denial")
        self.until(lambda: guest.call("clipboard_status")["acknowledged"] == 2, "second actual scoped write acknowledgment")
        self.record("guest", "active_clipboard_grant_is_not_member_authority", {"active_before_and_after": True,
                    "not_ordinary_member": True, "member_ping_rejected": True, "original_clipboard_pair_continues": True})
        # Respect the ordinary first failed-contact delay already proved by the
        # public SDK, so exit's separate control delivery has its real premise.
        time.sleep(1.05)
        ended = guest.call("guest_end")
        assert ended["ended_locally"] and all(item["saved"] for item in ended["delivery"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.case = "active-guest-scope"
    IsolationRunner(args).run()


if __name__ == "__main__":
    main()
