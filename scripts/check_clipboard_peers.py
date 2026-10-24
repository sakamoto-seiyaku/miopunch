#!/usr/bin/env python3
"""Actual Browser public-start cancellation and exclusive peer replacement."""
import argparse
import json
from pathlib import Path

from check_clipboard import Desktop, Host
from check_clipboard_browser import Browser, BrowserRunner


class PeerRunner(BrowserRunner):
    def scenario(self):
        first_desktop, third_desktop, browser_desktop = (Desktop(self, "peer-first"),
                                                       Desktop(self, "peer-third"),
                                                       Desktop(self, "peer-browser"))
        first_marker, third_marker = "M117 first owned system marker", "M117 third owned system marker"
        first_desktop.copy(first_marker)
        third_desktop.copy(third_marker)
        browser_desktop.copy("M117 browser own initial marker")
        first = Host(self, "peer-first", first_desktop)
        third = self.member(first, "peer-third", third_desktop)
        first.call("clipboard_enable")
        third.call("clipboard_enable")
        invitation = self.private / "peer-browser-invitation"
        first.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        try:
            browser = Browser(self, self.args, browser_desktop)
            browser.call("open")
            config = json.dumps({"rtc_enabled": False, "relays": ["https://aps1-1.relay.n0.iroh.link./"]})
            status = browser.call("start", "join", config, invitation.read_text())
            assert browser.call("request_join", False) == "Complete"
            browser.call("grant_clipboard")
            browser.call("enable_clipboard")
            third.call("service_sync", address=first.address)
            for operation in ["stop_clipboard", "disable_clipboard"]:
                browser.call("begin_clipboard_start", first.node)
                self.until(lambda: browser.call("clipboard_status")["phase"] == "connecting"
                           and browser.call("clipboard_start_pending"), "actual Browser start-pending prerequisite")
                browser.call(operation)
                pending = browser.call("finish_clipboard_start")
                assert not pending["ok"] and browser.call("clipboard_status")["peer"] is None
                self.until(lambda: first.call("clipboard_status")["peer"] is None, "peer's pending setup closed before a fresh start")
                assert first_desktop.read() == first_marker and third_desktop.read() == third_marker
                if operation == "disable_clipboard":
                    assert not browser.call("clipboard_status")["enabled"]
                    browser.call("enable_clipboard")
            browser.call("start_clipboard", first.node)
            browser.call("start_clipboard", first.node, ok=False)
            browser.call("start_clipboard", third.node, ok=False)
            assert browser.call("clipboard_status")["peer"] == first.node
            selected = "M117 only selected first peer receives Browser system copy"
            browser_desktop.copy(selected)
            self.until(lambda: first_desktop.read() == selected, "actual selected first system write", 30)
            assert third_desktop.read() == third_marker
            address = dict(id=status["id"], addrs=[{"Relay": "https://aps1-1.relay.n0.iroh.link./"}])
            third.call("clipboard_start", ok=False, address=address)
            assert browser.call("clipboard_status")["peer"] == first.node
            assert third_desktop.read() == third_marker
            browser.call("stop_clipboard")
            self.until(lambda: first.call("clipboard_status")["peer"] is None, "first peer stopped before explicit replacement")
            browser.call("start_clipboard", third.node)
            replacement = "M117 explicit Browser-to-third selected pair"
            browser_desktop.copy(replacement)
            self.until(lambda: third_desktop.read() == replacement, "actual replacement target system write", 30)
            assert first_desktop.read() == selected
            self.record("browser", "actual_pending_exclusive_and_replacement", {"actual_pending_stop": True,
                        "actual_pending_disable": True, "no_late_pair": True,
                        "duplicate_and_implicit_replacement_rejected": True, "third_cannot_join_active_pair": True,
                        "explicit_replacement_system_write": True, "old_peer_unchanged": True})
        finally:
            if hasattr(self, "browser"):
                self.browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    args.case = "peers"
    PeerRunner(args).run()


if __name__ == "__main__":
    main()
