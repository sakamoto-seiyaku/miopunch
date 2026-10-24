#!/usr/bin/env python3
"""Minimized real guest-expiry observation with untouched independent clocks."""
import argparse
from pathlib import Path
import time

from check_clipboard import Desktop, Host, Runner


class ExpiryRunner(Runner):
    def scenario(self):
        provider_desktop, guest_desktop = Desktop(self, "expiry-provider"), Desktop(self, "expiry-guest")
        provider_desktop.copy("M117 own minimal expiry provider baseline")
        guest_desktop.copy("M117 own minimal expiry guest baseline")
        provider = Host(self, "expiry-provider", provider_desktop)
        provider.call("clipboard_enable")
        invitation = self.private / "expiry-invitation"
        provider.call("guest_issue", path=str(invitation), provider=provider.address, service="clipboard", seconds=20)
        guest = Host(self, "expiry-guest", guest_desktop, "join-guest", invitation)
        guest.call("clipboard_enable")
        scope = guest.call("guest_redeem")["scope"]
        guest.call("clipboard_start", address=provider.address)
        value = "M117 own real write before minimal expiry"
        guest_desktop.copy(value)
        self.until(lambda: provider_desktop.read() == value, "real write before expiry")
        self.until(lambda: guest.call("clipboard_status")["acknowledged"] == 1, "real write acknowledged before expiry")
        self.until(lambda: guest.call("guest_status")["stage"] == "GuestExpired", "natural guest deadline", 25)
        self.until(lambda: provider.call("clipboard_status")["peer"] is None, "expiry stops provider")
        self.until(lambda: guest.call("clipboard_status")["peer"] is None, "expiry stops guest")
        guest_desktop.copy("M117 own after minimal expiry")
        time.sleep(.8)
        assert provider_desktop.read() == value
        self.until(lambda: guest.call("guest_status")["stage"] == "GuestExpired"
                   and time.time() >= scope["expires"] + 1, "current expired-clock prerequisite", 45)
        status = guest.call("guest_status")
        provider_status = provider.call("service_status")
        current = next(item for item in provider_status["guests"] if item["invitation"] == scope["invitation"])
        members = [item["node"] for item in provider.call("status")["members"]]
        self.record("expiry", "current_expiry_prerequisite", {"original_scope": scope, "current_guest_status": status,
                    "current_provider_scope": current, "guest_is_member": guest.node in members,
                    "controller_wall_still_expired": time.time() >= scope["expires"], "no_late_system_write": True})
        assert status["scope"] == scope and current["guest"] == guest.node and guest.node not in members
        # Keep the exact original negative public operation/core assertion.
        guest.call("clipboard_start", ok=False, address=provider.address)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.case = "expiry"
    ExpiryRunner(args).run()


if __name__ == "__main__":
    main()
