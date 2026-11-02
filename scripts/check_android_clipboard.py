#!/usr/bin/env python3
"""Installed Android foreground clipboard public entry, using frozen SDK pins."""
import argparse
import json
from pathlib import Path
import time

from check_android_host import AndroidActor
from check_clipboard import Desktop, Host
from check_web_service import OwnedNode, WebRunner


class ClipboardDesktop:
    def __init__(self, runner, name):
        self.runner, self.name, self.reader = runner, name, self
        runner.targets.append(self)
        self.desktop = Desktop(runner, name)

    def is_alive(self):
        return not self.desktop.terminal()

    def close(self):
        self.desktop.close()
        self.desktop.release_lease()
        assert self.desktop.terminal()
        return {"desktop": self.name, "owned_X11_terminal": True}


class ClipboardNode(Host):
    def __init__(self, runner, label, desktop, action="create", invitation=None):
        runner.nodes.append(self)
        super().__init__(runner, label, desktop, action, invitation)
        self.identity = self.node


class AndroidClipboardRunner(WebRunner):
    def scenario(self):
        resources = {
            "defaults": {"revision": 2, "resources": []},
            "replacement": [], "disabled": [],
            "disabled_uses": ["Observation", "Lookup", "Signaling", "DataRelay"],
            "lan_discovery": "Disabled", "port_mapping": "Disabled",
        }
        self.native_configuration = self.private / "resources.json"
        self.native_configuration.write_text(json.dumps(resources))
        self.configuration = self.native_configuration
        self.freeze_source(Path(__file__).with_name("check_android_host.py"))
        self.freeze_source(Path(__file__).with_name("check_clipboard.py"))
        self.hosts, self.desktops = [], []
        if self.args.tools:
            self.tools = self.args.tools.resolve()
            assert (self.tools / "xclip").is_file()
            desktop = ClipboardDesktop(self, "linux-clipboard").desktop
            desktop.copy("M121 independently owned Linux baseline")
            peer = ClipboardNode(self, "linux-peer", desktop)
        else:
            peer = OwnedNode(self, "linux-peer")
        android = AndroidActor(self)
        invitation = self.private / "android-invitation"
        peer.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        private_invitation = android.write_private("invitation.json", invitation.read_bytes())
        started = android.call("start", mode={"action": "join", "invitation": invitation.read_text()}, resources=resources)
        assert started["ok"] and started["result"]["status"]["node"] != peer.identity
        assert android.sdk("request", path=private_invitation)["stage"] == "Complete"
        assert android.sdk("ping", address=peer.address)["reply"] == "PONG"
        self.until(lambda: android.call("snapshot")["window_focused"], "actual Android foreground window", seconds=15)
        self.record("controller", "clipboard_entry_preconditions", {
            "installed_APK": True, "actual_focused_Activity": True,
            "independent_admitted_peer": True, "authenticated_PING": True,
            "original_clipboard_not_read_or_changed_by_controller": True,
        })
        if self.args.tools:
            assert android.call("clipboard_begin", label="baseline", bytes=128)["matches"]
        response = android.call("request", request={"id": android.counter + 1, "op": "clipboard_enable"})
        self.record("controller", "actual_foreground_clipboard_enable", response)
        assert response["ok"], "actual foreground APK clipboard_enable rejected: " + str(response.get("error"))
        state = response["result"]
        assert state["enabled"] and state["capability"]["read"] and state["capability"]["write"]
        self.record("controller", "actual_android_clipboard_entry", {
            "foreground_system_adapter_enabled": True, "memory_fallback": False,
        })
        if self.args.tools:
            def marker(label, size=256):
                prefix = "M121 Android clipboard " + android.lease + " " + label + " Unicode中文🙂"
                return "" if size == 0 else prefix + "x" * (size - len(prefix.encode()))

            enabled = peer.call("clipboard_enable")
            assert enabled["enabled"] and enabled["capability"]["read"] and enabled["capability"]["write"]
            self.record("controller", "service_sync_entry_observation", {
                "Linux": peer.call("service_status"),
                "Android": android.sdk("service_status"),
                "Android_window": android.call("snapshot"),
            })
            android.sdk("service_sync", address=peer.address)
            peer.call("service_sync", address=started["result"]["address"])
            peer.call("clipboard_start", address=started["result"]["address"])
            assert desktop.read() == "M121 independently owned Linux baseline"
            assert android.call("clipboard_expect", label="baseline", bytes=128)["matches"]
            desktop.copy(marker("from-linux"))
            self.until(lambda: android.call("clipboard_expect", label="from-linux", bytes=256)["matches"], "independent Android system read", seconds=20)
            self.until(lambda: peer.call("clipboard_status")["acknowledged"] == 1, "Android actual write acknowledgment", seconds=20)
            assert android.sdk("clipboard_status")["written"] == 1
            assert android.call("clipboard_copy", label="from-android", bytes=256)["matches"]
            self.until(lambda: desktop.read() == marker("from-android"), "independent X11 reverse read", seconds=20)
            self.until(lambda: android.sdk("clipboard_status")["acknowledged"] == 1, "X11 actual write acknowledgment", seconds=20)
            time.sleep(1)
            assert peer.call("clipboard_status")["sent"] == android.sdk("clipboard_status")["sent"] == 1
            desktop.copy("")
            self.until(lambda: android.call("clipboard_expect", label="empty", bytes=0)["matches"], "Android actual empty text", seconds=20)
            assert android.call("clipboard_copy", label="large", bytes=65536)["matches"]
            self.until(lambda: desktop.read() == marker("large", 65536), "actual64KiB Android-to-X11 clipboard", seconds=25)
            self.record("controller", "actual_android_clipboard_duplex", {
                "Unicode_both_directions": True, "independent_framework_and_X11_reads": True,
                "acknowledgments_each": True, "no_echo": True, "empty_text": True,
                "large_Android_to_Linux_bytes": 65536, "initial_baselines_not_overwritten": True,
            })
            peer.call("clipboard_stop")
            self.until(lambda: android.sdk("clipboard_status")["peer"] is None, "Android observes pair stop", seconds=10)
            desktop.copy(marker("after-stop"))
            time.sleep(.8)
            assert android.call("clipboard_expect", label="large", bytes=65536)["matches"]
            android.sdk("clipboard_start", address=peer.address)
            assert android.call("clipboard_copy", label="reverse-initiator", bytes=256)["matches"]
            self.until(lambda: desktop.read() == marker("reverse-initiator"), "Android-initiated actual pair", seconds=20)
            android.call("close_window")
            self.until(lambda: not android.call("snapshot")["activity_exists"], "Activity actually destroyed", seconds=10)
            self.until(lambda: not android.sdk("clipboard_status")["capability"]["read"], "Android background capability unavailable", seconds=10)
            self.until(lambda: peer.call("clipboard_status")["peer"] is None, "Background stops actual clipboard pair", seconds=10)
            desktop.copy(marker("background-denied"))
            time.sleep(.8)
            assert android.sdk("ping", address=peer.address)["reply"] == "PONG"
            android.call("show_window")
            self.until(lambda: android.call("snapshot")["window_focused"], "Actual foreground regained", seconds=15)
            assert android.call("clipboard_expect", label="reverse-initiator", bytes=256)["matches"]
            android.sdk("clipboard_disable")
            peer.call("clipboard_disable")
            assert not android.sdk("clipboard_status")["enabled"]
            assert android.call("clipboard_copy", label="final-owned", bytes=256)["matches"]
            restored = android.call("clipboard_restore")
            assert restored["restored_if_matching_owned_revision"] and not restored["original_clipboard_exported"]
            self.record("controller", "android_clipboard_lifecycle", {
                "public_stop_prevents_write": True, "Android_can_initiate": True,
                "actual_background_reports_unavailable": True, "background_ends_pair_without_stopping_node": True,
                "no_background_write": True, "same_service_foreground_reopen": True,
                "explicit_disable": True, "conditional_original_restoration_before_uninstall": restored,
            })
        android.call("finish")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "source-receipt", "apk-receipt", "output", "device-selection"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--tools", type=Path, help="Existing private xclip directory for real duplex and lifecycle cases")
    args = parser.parse_args()
    selected = json.loads(args.device_selection.read_text())
    args.adb, args.serial, args.case = Path(selected["adb"]), selected["serial"], "android-clipboard-entry"
    AndroidClipboardRunner(args).run()


if __name__ == "__main__":
    main()
