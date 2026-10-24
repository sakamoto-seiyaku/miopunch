#!/usr/bin/env python3
"""Actual headless/X11 capability, text bounds, and desktop-loss acceptance."""
import argparse
import json
from pathlib import Path
import time
from types import SimpleNamespace

from check_clipboard import Desktop, Host, Runner


class CapabilityRunner(Runner):
    def scenario(self):
        source_desktop, target_desktop = Desktop(self, "bounds-source"), Desktop(self, "bounds-target")
        source_desktop.copy("M117 bounds source initial")
        target_desktop.copy("M117 bounds target initial")
        environment = dict(source_desktop.env)
        environment.pop("DISPLAY", None)
        environment.pop("WAYLAND_DISPLAY", None)
        headless = Host(self, "headless", SimpleNamespace(env=environment))
        headless.call("clipboard_enable", ok=False)
        absent = headless.call("clipboard_status")
        assert not absent["enabled"] and not absent["capability"]["read"] and absent["capability"]["reason"]
        assert not next(item for item in headless.call("service_status")["services"] if item["service"] == "clipboard")["capable"]
        self.record("headless", "truthful_no_desktop", {"no_fake_buffer": True, "unavailable_and_disabled": True})
        headless.close()
        source = Host(self, "bounds-source", source_desktop)
        target = self.member(source, "bounds-target", target_desktop)
        third_desktop = Desktop(self, "bounds-third")
        third_marker = "M117 unselected third independent system clipboard"
        third_desktop.copy(third_marker)
        third = self.member(source, "bounds-third", third_desktop)
        source.call("service_sync", address=target.address)
        source.call("clipboard_enable")
        target.call("clipboard_enable")
        source.call("clipboard_start", address=target.address)
        maximum = "M117-owned:" + "a" * (1024 * 1024 - len("M117-owned:"))
        assert len(maximum.encode()) == 1024 * 1024
        source_desktop.copy(maximum)
        self.until(lambda: target_desktop.read() == maximum, "actual maximum multichunk system write", 30)
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "maximum system-write receipt", 30)
        source_desktop.copy("")
        self.until(lambda: target_desktop.read() == "", "actual empty-text system write", 30)
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 2, "empty-text actual acknowledgment")
        retained = "M117 Unicode 空文本后の🙂\n"
        source_desktop.copy(retained)
        self.until(lambda: target_desktop.read() == retained, "actual Unicode after empty", 30)
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 3, "Unicode actual acknowledgment")
        source_desktop.copy("x" * (1024 * 1024 + 1))
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "oversize stops without a write", 30)
        failed = source.call("clipboard_status")
        assert failed["failure"] == "clipboard_text_too_large" and failed["sent"] == 3
        assert target_desktop.read() == retained
        assert third_desktop.read() == third_marker
        self.record("bounds", "actual_text_boundaries", {"maximum_bytes": 1024 * 1024,
                    "maximum_actual_system_write": True, "empty_and_unicode": True,
                    "oversize_has_no_write_or_ack": True})
        self.until(lambda: target.call("clipboard_status")["peer"] is None,
                   "recipient observes ended oversize session before a new pair", 30)
        source_desktop.copy("M117 owned valid source baseline after oversize")
        source.call("clipboard_start", address=target.address)
        restored = "M117 owned actual value after explicit restart"
        source_desktop.copy(restored)
        self.until(lambda: target_desktop.read() == restored, "same system operation after size failure", 30)
        third.call("clipboard_enable")
        source.call("clipboard_start", ok=False, address=third.address)
        assert source.call("clipboard_status")["peer"] == target.node
        assert third_desktop.read() == third_marker
        source.call("clipboard_stop")
        self.until(lambda: target.call("clipboard_status")["peer"] is None, "old selected peer stopped before explicit replacement")
        source.call("clipboard_start", address=third.address)
        replacement = "M117 actual explicit replacement pair value"
        source_desktop.copy(replacement)
        self.until(lambda: third_desktop.read() == replacement, "actual newly selected peer system write", 30)
        assert target_desktop.read() == restored
        source.call("clipboard_stop")
        self.until(lambda: third.call("clipboard_status")["peer"] is None, "replacement peer stopped before new pair")
        source.call("clipboard_start", address=target.address)
        restored = "M117 owned actual value before private desktop loss"
        source_desktop.copy(restored)
        self.until(lambda: target_desktop.read() == restored, "actual selected peer before desktop loss", 30)
        self.until(lambda: (state := source.call("clipboard_status"))["sent"] == state["acknowledged"], "last real write acknowledged before desktop loss")
        self.record("bounds", "explicit_pair_replacement", {"active_replacement_rejected": True,
                    "explicit_stop_then_new_pair": True, "old_peer_not_written": True,
                    "third_unselected_until_explicit_choice": True})
        # Stop only this owned isolated desktop. No global desktop, route,
        # firewall, service or unrelated process is changed.
        source_desktop.close()
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "actual desktop loss stops pair", 30)
        self.until(lambda: target.call("clipboard_status")["peer"] is None, "desktop loss closes remote pair", 30)
        unavailable = source.call("clipboard_status")
        assert not unavailable["capability"]["read"] and not unavailable["capability"]["write"]
        assert not next(item for item in source.call("service_status")["services"] if item["service"] == "clipboard")["capable"], "unified service status overstated actual lost desktop capability"
        assert unavailable["failure"] and target_desktop.read() == restored
        assert third_desktop.read() == replacement
        source.call("clipboard_enable", ok=False)
        self.record("bounds-source", "actual_desktop_loss", {"real_desktop_removed": True,
                    "capability_unavailable": True, "pair_stopped": True, "independent_target_retained": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.case = "capabilities"
    CapabilityRunner(args).run()


if __name__ == "__main__":
    main()
