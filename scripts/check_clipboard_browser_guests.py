#!/usr/bin/env python3
"""Actual scoped Clipboard guests in both Native/Browser platform roles."""
import argparse
import json
from pathlib import Path
import time

from check_clipboard import Desktop, Host
from check_clipboard_browser import Browser, BrowserRunner


class GuestBrowserRunner(BrowserRunner):
    def scenario(self):
        native_desktop, browser_desktop = Desktop(self, "guest-native"), Desktop(self, "guest-browser")
        native_desktop.copy("M117 owned native guest baseline")
        browser_desktop.copy("M117 owned browser guest baseline")
        browser = Browser(self, self.args, browser_desktop)
        browser.call("open")
        config = json.dumps({"rtc_enabled": False, "relays": ["https://aps1-1.relay.n0.iroh.link./"]})
        path = self.private / "mixed-clipboard-invitation"
        if self.args.role == "browser-guest":
            native = Host(self, "guest-native", native_desktop)
            native.call("clipboard_enable")
            native.call("guest_issue", path=str(path), provider=native.address, service="clipboard",
                        seconds=60 if self.args.ending == "expiry" else 180)
            status = browser.call("start", "join_guest", config, path.read_text())
            browser.call("grant_clipboard")
            browser.call("enable_clipboard")
            browser.call("start_clipboard", native.node, ok=False)
            assert browser.call("redeem_guest")["scope"]["service"] == "clipboard"
            browser.call("start_clipboard", native.node)
        else:
            status = browser.call("start", "create", config)
            # Save the fresh test card privately, then acknowledge that save.
            card = self.private / "browser-provider-card"
            card.write_text(browser.call("export_recovery_card", None))
            card.chmod(0o600)
            browser.call("acknowledge_recovery_saved")
            browser.call("grant_clipboard")
            browser.call("enable_clipboard")
            address = dict(id=status["id"], addrs=[{"Relay": "https://aps1-1.relay.n0.iroh.link./"}])
            path.write_text(browser.call("issue_service_guest", json.dumps(address), "clipboard", int(time.time()) + 180))
            path.chmod(0o600)
            native = Host(self, "guest-native", native_desktop, "join-guest", path)
            native.call("clipboard_enable")
            native.call("clipboard_start", ok=False, address=address)
            assert native.call("guest_redeem")["scope"]["service"] == "clipboard"
            native.call("clipboard_start", address=address)
        first = "M117 owned real guest Native→WASM系统剪贴板"
        native_desktop.copy(first)
        self.until(lambda: browser_desktop.read() == first, "actual guest Native to Browser write", 30)
        self.until(lambda: native.call("clipboard_status")["acknowledged"] == 1, "actual Browser guest write acknowledgment")
        reverse = "M117 owned real guest WASM→Native系统剪贴板"
        browser_desktop.copy(reverse)
        self.until(lambda: native_desktop.read() == reverse, "actual guest Browser to Native write", 30)
        self.until(lambda: browser.call("clipboard_status")["acknowledged"] == 1, "actual Native guest write acknowledgment")
        time.sleep(.8)
        assert native.call("clipboard_status")["sent"] == browser.call("clipboard_status")["sent"] == 1
        if self.args.ending == "expiry":
            assert self.args.role == "browser-guest"
            scope = browser.call("guest_status")["scope"]
            self.until(lambda: browser.call("guest_status")["stage"] == "GuestExpired", "actual Browser natural expiry", 65)
            self.until(lambda: browser.call("clipboard_status")["peer"] is None, "Browser expiry ends pair")
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "Browser expiry ends Native peer")
            browser_desktop.copy("M117 owned Browser after actual expiry")
            time.sleep(.8)
            assert native_desktop.read() == reverse
            self.until(lambda: browser.call("guest_status")["stage"] == "GuestExpired"
                       and time.time() >= scope["expires"] + 1, "current Browser expiry-clock prerequisite", 45)
            browser.call("start_clipboard", native.node, ok=False)
            self.record("browser-guest", "actual_scoped_expiry", {"actual_before_expiry": True,
                        "current_expired_clock_prerequisite": True, "no_later_system_write": True})
            return
        if self.args.role == "browser-guest":
            scope = browser.call("guest_status")["scope"]
            assert scope["guest"] == status["id"]
            assert status["id"] not in [member["node"] for member in native.call("status")["members"]]
            revision = native.call("service_rule", provider=native.node, service="clipboard", previous=None,
                                   rule=dict(members=True, guests=True, allowed=[], denied=[status["id"]]))["revision"]
            self.until(lambda: browser.call("clipboard_status")["peer"] is None, "Native ACL retires Browser guest")
            browser_desktop.copy("M117 owned Browser guest after ACL denial")
            time.sleep(.8)
            assert native_desktop.read() == reverse
            browser.call("start_clipboard", native.node, ok=False)
            native.call("service_rule", provider=native.node, service="clipboard", previous=revision,
                        rule=dict(members=True, guests=True, allowed=[], denied=[]))
            assert browser.call("redeem_guest")["stage"] == "GuestActive"
            browser.call("start_clipboard", native.node)
            ended = browser.call("end_guest")
        else:
            scope = native.call("guest_status")["scope"]
            assert scope["guest"] == native.node and scope["provider"] == status["id"]
            # The ordinary Browser public ACL entry enforces the same rule.
            revision = browser.call("set_service_rule", status["id"], "clipboard", None,
                                    json.dumps(dict(members=True, guests=True, allowed=[], denied=[native.node])))
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "Browser ACL retires Native guest")
            native_desktop.copy("M117 owned Native guest after Browser ACL denial")
            time.sleep(.8)
            assert browser_desktop.read() == reverse
            native.call("clipboard_start", ok=False, address=address)
            browser.call("set_service_rule", status["id"], "clipboard", revision,
                         json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
            # Native public SDK proves the ordinary first-contact backoff;
            # restore current authority, respect that bound, then request once.
            time.sleep(1.05)
            assert native.call("guest_redeem")["stage"] == "GuestActive"
            native.call("clipboard_start", address=address)
            ended = native.call("guest_end")
        assert ended["ended_locally"] and all(item["saved"] for item in ended["delivery"])
        self.until(lambda: native.call("clipboard_status")["peer"] is None, "guest exit retires Native")
        self.until(lambda: browser.call("clipboard_status")["peer"] is None, "guest exit retires Browser")
        browser_before = browser_desktop.read()
        native_desktop.copy("M117 owned after mixed guest exit")
        time.sleep(.8)
        assert browser_desktop.read() == browser_before
        browser_desktop.copy("M117 owned reverse after mixed guest exit")
        time.sleep(.8)
        assert native_desktop.read() == "M117 owned after mixed guest exit"
        self.record(self.args.role, "scoped_guest_real_wasm_system_duplex", {"actual_system_reads": True,
                    "exact_invited_service": True, "no_echo": True, "current_acl_stops_effects": True,
                    "guest_exit_saved_and_no_later_write": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--role", choices=["browser-guest", "browser-provider"], required=True)
    parser.add_argument("--ending", choices=["acl-exit", "expiry"], default="acl-exit")
    args = parser.parse_args()
    args.case = args.role
    GuestBrowserRunner(args).run()


if __name__ == "__main__":
    main()
