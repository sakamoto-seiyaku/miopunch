#!/usr/bin/env python3
"""Actual unsupported format versus empty-text observations on the public pair.

Owned PNG is written by the real Clipboard API, and independent X11 reads
prove format absence. No clipboard methods or platform responses are replaced.
"""
import argparse
import json
from pathlib import Path
import time

from check_clipboard import Desktop, Host
from check_clipboard_browser import Browser, BrowserRunner


class FormatsRunner(BrowserRunner):
    def scenario(self):
        native_desktop, browser_desktop = Desktop(self, "format-native"), Desktop(self, "format-browser")
        native_desktop.copy("M117 owned native format baseline")
        browser_desktop.copy("M117 owned browser format baseline")
        native = Host(self, "format-native", native_desktop)
        native.call("clipboard_enable")
        invitation = self.private / "format-invitation"
        native.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        try:
            browser = Browser(self, self.args, browser_desktop)
            browser.call("open")
            configuration = json.dumps({"rtc_enabled": False, "relays": ["https://aps1-1.relay.n0.iroh.link./"]})
            browser.call("start", "join", configuration, invitation.read_text())
            assert browser.call("request_join", False) == "Complete"
            browser.call("grant_clipboard")
            browser.call("enable_clipboard")
            browser.call("start_clipboard", native.node)
            before = "M117 actual supported text before owned nontext format"
            browser_desktop.copy(before)
            self.until(lambda: native_desktop.read() == before, "real text prerequisite before image", 30)
            self.until(lambda: browser.call("clipboard_status")["acknowledged"] == 1, "confirmed system text write before image")
            browser.call("copy_owned_png")
            self.until(lambda: browser_desktop.read() is None, "owned system image has no UTF8 text target")
            time.sleep(1)
            after_image = native_desktop.read()
            self.record("formats", "unsupported_system_image", {"browser_has_no_text_target": True,
                        "native_text_retained": after_image == before,
                        "native_is_empty_text": after_image == "",
                        "text_only_capability": browser.call("clipboard_status")["capability"]["formats"]})
            assert after_image == before, "unsupported nontext clipboard was falsely transferred as empty text"
            # A genuine empty text selection is different from no text format.
            browser_desktop.copy("")
            assert browser_desktop.read() == ""
            self.until(lambda: native_desktop.read() == "", "real empty text propagates", 30)
            self.until(lambda: browser.call("clipboard_status")["acknowledged"] == 2, "empty text acknowledged after actual system write")
            self.record("formats", "actual_empty_text", {"empty_text_target_exists": True, "actual_native_system_empty_text": True})
            maximum = "M117-owned:" + "b" * (1024 * 1024 - len("M117-owned:"))
            browser_desktop.copy(maximum)
            self.until(lambda: native_desktop.read() == maximum, "actual maximum Browser text through all stream chunks", 30)
            self.until(lambda: browser.call("clipboard_status")["acknowledged"] == 3, "Browser maximum actual-write acknowledgment", 30)
            browser_desktop.copy("b" * (1024 * 1024 + 1))
            self.until(lambda: browser.call("clipboard_status")["peer"] is None, "Browser oversize stops without write", 30)
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "native observes Browser oversized session end", 30)
            failed = browser.call("clipboard_status")
            assert failed["failure"] == "clipboard_text_too_large" and failed["sent"] == 3
            assert native_desktop.read() == maximum
            self.record("formats", "browser_actual_text_bounds", {"maximum_bytes": 1024 * 1024,
                        "maximum_actual_system_write": True, "oversize_no_write_or_ack": True})
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
    args.case = "formats"
    FormatsRunner(args).run()


if __name__ == "__main__":
    main()
