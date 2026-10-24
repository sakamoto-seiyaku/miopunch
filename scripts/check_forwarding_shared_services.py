#!/usr/bin/env python3
"""Necessary real Files/OPFS regression for the changed common stream/guest seam."""
import argparse
import hashlib
import json
from pathlib import Path
from check_forwarding import Node, Target
from check_forwarding_browser_scopes import ScopeRunner


class SharedRunner(ScopeRunner):
    def task(self, owner, task, browser=False):
        status = owner.call("file_status" if browser else "files_status")
        return next((item for item in status["tasks"] if item["id"] == task), None)

    def completed(self, source, target, task, source_browser=False, target_browser=False):
        self.until(lambda: (self.task(source, task, source_browser) or {}).get("state") in {"complete", "failed", "cancelled"}, "actual source file terminal", timeout=75)
        self.until(lambda: (self.task(target, task, target_browser) or {}).get("state") in {"complete", "failed", "cancelled"}, "actual target file terminal", timeout=75)
        sent, received = self.task(source, task, source_browser), self.task(target, task, target_browser)
        assert sent["state"] == received["state"] == "complete" and sent["receipt"] == received["receipt"]
        return received["receipt"]

    def shared(self):
        provider = Node(self, "provider")
        browser, browser_id = self.browser_member(provider)
        native_files = self.private / "native-files"
        provider.call("files_enable", directory=str(native_files))
        browser.call("enable_files")
        source = self.private / "native-source.bin"
        chunk = bytes(index % 251 for index in range(65536))
        source.write_bytes(chunk * 64)
        expected = {"size": source.stat().st_size, "sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
        relay = json.loads(self.configuration)["relays"][0]
        address = dict(id=browser_id, addrs=[{"Relay": relay}])
        task = provider.call("file_send", address=address, path=str(source))["task"]
        receipt = self.completed(provider, browser, task, target_browser=True)
        assert browser.call("file_hash", receipt["saved_name"], False) == expected
        browser.call("file_make", "browser-source.bin", 4 * 1024**2)
        expected_browser = browser.call("file_hash", "browser-source.bin", True)
        task = browser.call("file_send_source", provider.identity, "browser-source.bin")
        receipt = self.completed(browser, provider, task, source_browser=True)
        actual = native_files / receipt["saved_name"]
        assert dict(size=actual.stat().st_size, sha256=hashlib.sha256(actual.read_bytes()).hexdigest()) == expected_browser
        invitation = self.private / "files-guest-invitation"
        provider.call("guest_issue", provider=provider.address, service="files", seconds=240, path=str(invitation))
        guest = Node(self, "files-guest", "join-guest", invitation)
        assert guest.call("guest_redeem")["scope"]["service"] == "files"
        target = Target(self, "tcp", prefix=b"")
        provider.call("forward_enable", protocol="tcp", targets=[target.address])
        guest.call("forward_start", ok=False, address=provider.address, protocol="tcp", target=target.address, bind="127.0.0.1:0")
        assert not target.received
        before = set(native_files.iterdir())
        task = guest.call("file_send", address=provider.address, path=str(source))["task"]
        self.until(lambda: (self.task(provider, task) or {}).get("state") == "awaiting_approval", "Files guest pending decision")
        assert self.task(provider, task)["bytes"] == 0 and self.task(provider, task)["receipt"] is None and set(native_files.iterdir()) == before
        provider.call("file_reject", task=task)
        self.until(lambda: (self.task(guest, task) or {}).get("state") == "failed", "rejected Files guest terminal")
        assert set(native_files.iterdir()) == before
        task = guest.call("file_send", address=provider.address, path=str(source))["task"]
        self.until(lambda: (self.task(provider, task) or {}).get("state") == "awaiting_approval", "next Files guest pending decision")
        provider.call("file_approve", task=task)
        receipt = self.completed(guest, provider, task)
        actual = native_files / receipt["saved_name"]
        assert dict(size=actual.stat().st_size, sha256=hashlib.sha256(actual.read_bytes()).hexdigest()) == expected
        self.record("controller", "shared_real_files_and_guest_regression", {"native_to_OPFS_4MiB_exact": True,
                    "browser_to_native_4MiB_exact": True, "same_saved_receipt": True, "Files_guest_not_TCP": True,
                    "pending_no_sink": True, "reject_no_save": True, "approve_exact_native_save": True})

    def scenario(self):
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(__file__).with_name("check_forwarding_browser_scopes.py"))
        super().scenario()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="shared")
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    SharedRunner(parser.parse_args()).run()
