#!/usr/bin/env python3
"""Actual retained-partial cancellation/disable and slot-reuse qualification.

Disconnect the receiver's one owned connection, then close the sender before
its automatic retry. No peer remains to finish or cancel the retained receive.
All observations use public SDK hosts and actual Native/OPFS files.
"""
import argparse
import hashlib
import json
from pathlib import Path
import time

from check_browser_files import BrowserFilesRunner, TERMINAL
from check_browser_onboarding import BrowserControl
from check_native_client import Host


class LifecycleRunner(BrowserFilesRunner):
    def native_actor(self, name):
        host = self.start(name, self.args.native_platform == "windows")
        host.call("export_recovery_card", path=self.path(name + "-card.json", host.remote))
        host.call("acknowledge_recovery_saved")
        return host

    def member(self, native, name):
        invitation_name = name + "-invite.json"
        native.call("invite", path=self.path(invitation_name, native.remote), text=True)
        self.transfer(invitation_name, native.remote, False)
        status = self.browser.start(name, "join", Path(self.path(invitation_name, False)).read_text())
        self.browser_nodes[name] = status["id"]
        assert self.browser.call(name, "request_join", False) == "AwaitingApproval"
        pending = [entry for entry in native.call("requests")["requests"] if entry["applicant"] == status["id"]]
        assert len(pending) == 1
        native.call("approve", **pending[0])
        assert self.browser.call(name, "request_join", False) == "Complete"
        return name

    def source(self, actor, prefix, size):
        if isinstance(actor, str):
            return self.generate_browser(actor, prefix + ".bin", size)
        path = self.path(prefix + ".bin", actor.remote)
        if actor.remote or not Path(path).exists():
            # A reopened native source reuses the original file; create once.
            if actor.remote:
                exists = self.remote("Test-Path -LiteralPath '" + path.replace("'", "''") + "'").strip() == "True"
                if exists:
                    return path, self.hash_file(path, True)
            return self.generate(actor, prefix + ".bin", size)
        return path, self.hash_file(path, False)

    def stop_source(self, actor):
        if isinstance(actor, str):
            self.browser.call(actor, "close")
        else:
            actor.stop()

    def reopen_source(self, actor, index):
        if isinstance(actor, str):
            before = self.browser_nodes[actor]
            self.browser.call(actor, "reload")
            status = self.browser.call(actor, "start", "start", "{}", None)
            assert status["id"] == before and status["stage"] == "Complete"
            return actor
        reopened = Host(self, actor.label + f"-reopen{index}", actor.remote, "open", actor.directory)
        assert reopened.node == actor.node
        return reopened

    def receiver_status(self, receiver):
        return self.browser.call(receiver, "file_status") if isinstance(receiver, str) else receiver.call("files_status")

    def files(self, receiver, directory):
        return self.browser.call(receiver, "file_list") if isinstance(receiver, str) else self.listing(directory, receiver.remote)

    def disable(self, receiver):
        if isinstance(receiver, str):
            self.browser.call(receiver, "disable_files")
        else:
            receiver.call("files_disable")

    def enable_again(self, receiver, directory):
        if isinstance(receiver, str):
            self.browser.call(receiver, "enable_files")
        else:
            receiver.call("files_enable", directory=directory)

    def scenario(self, kind):
        native = self.native_actor(kind + "-native")
        browser = self.member(native, kind + "-browser")
        source, receiver = (native, browser) if kind == "browser" else (browser, native)
        directory = None
        if isinstance(receiver, str):
            self.browser.call(receiver, "enable_files")
        else:
            directory = self.enable(receiver, kind + "-lifecycle")
        tasks = []
        expiring = getattr(self.args, "expire", False)
        for index in range(1 if self.args.expect_retained_red or expiring else 4):
            path, _ = self.source(source, kind + "-retained-source", 16 * 1024**2)
            identifier = self.send(source, receiver, path)
            self.wait_live(source, receiver, identifier)
            self.disconnect(receiver, identifier)
            self.stop_source(source)
            deadline = time.monotonic() + 30
            while (retained := self.task(receiver, identifier))["state"] != "interrupted":
                assert retained["state"] not in TERMINAL and time.monotonic() < deadline, retained
                time.sleep(0.02)
            assert retained["bytes"] > 0 and retained["receipt"] is None
            assert self.files(receiver, directory)
            if self.args.expect_retained_red:
                self.cancel(receiver, identifier)
                time.sleep(1)
                cancelled = self.task(receiver, identifier)
                before = self.files(receiver, directory)
                self.disable(receiver)
                after = self.task(receiver, identifier)
                assert cancelled["state"] == after["state"] == "cancelling", (cancelled, after)
                assert before and not self.files(receiver, directory)
                self.record(kind, "expected_inactive_partial_RED", dict(cancelled=cancelled,
                            afterDisable=after, partialRemovedOnlyByDisable=True, noPeerRetry=True))
                return
            if expiring:
                terminal = self.wait_task(receiver, identifier)
                assert terminal["state"] == "failed", terminal
                assert terminal["error"] == "interrupted", terminal
                assert not self.files(receiver, directory)
                self.record(kind, "retained_expiry_cleanup_GREEN", dict(
                    task=identifier, noPeerRetry=True, terminal=terminal,
                    partialRemoved=True))
                tasks.append(identifier)
                source = self.reopen_source(source, index + 1)
                continue
            operation = "cancel" if index < 2 else "disable"
            if operation == "cancel":
                self.cancel(receiver, identifier)
            else:
                self.disable(receiver)
            terminal = self.wait_task(receiver, identifier)
            assert terminal["state"] == "cancelled", terminal
            assert not self.files(receiver, directory)
            status = self.receiver_status(receiver)
            assert all(task["state"] in TERMINAL for task in status["tasks"])
            self.record(kind, "inactive_partial_cleanup_GREEN", dict(operation=operation,
                        task=identifier, noPeerRetry=True, partialRemoved=True, allSlotsReleased=True))
            if operation == "disable":
                self.enable_again(receiver, directory)
            tasks.append(identifier)
            source = self.reopen_source(source, index + 1)
        assert len(set(tasks)) == (1 if expiring else 4)
        path, expected = self.source(source, kind + "-fifth-proof", 11)
        identifier = self.send(source, receiver, path)
        sending, receiving = self.wait_task(source, identifier), self.wait_task(receiver, identifier)
        assert sending["state"] == receiving["state"] == "complete", (sending, receiving)
        receipt = sending["receipt"]
        if isinstance(receiver, str):
            assert self.browser.call(receiver, "file_hash", receipt["saved_name"], False) == expected
            self.browser.call(receiver, "file_remove", receipt["saved_name"])
        else:
            saved = directory + "/" + receipt["saved_name"]
            assert self.hash_file(saved, receiver.remote) == expected
            self.remove(saved, receiver.remote)
        assert not self.files(receiver, directory)
        self.record(kind, "transfer_after_expiry_cleanup" if expiring else "fifth_transfer_after_four_retained_cleanups",
                    dict(saved=expected, qualified=True))

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.deploy()
            self.browser = BrowserControl(self, self.args)
            for kind in (["native", "browser"] if self.args.receiver == "both" else [self.args.receiver]):
                self.scenario(kind)
            passed = True
        finally:
            if hasattr(self, "browser"):
                try:
                    browser_cleanup = self.browser.close()
                except Exception as error:
                    browser_cleanup = dict(error=type(error).__name__)
            cleanup = self.cleanup()
            drift = self.controller_drift()
            passed = passed and not drift and browser_cleanup is not None and "error" not in browser_cleanup and all("error" not in item for item in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed,
                expectedRED=self.args.expect_retained_red, receiver=self.args.receiver,
                expiry=getattr(self.args, "expire", False),
                nativePlatform=self.args.native_platform, browserCleanup=browser_cleanup,
                cleanup=cleanup, controllerSourceDrift=drift, package=self.args.package_sha), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--native-platform", choices=["linux", "windows"], default="linux")
    parser.add_argument("--receiver", choices=["native", "browser", "both"], default="both")
    parser.add_argument("--expect-retained-red", action="store_true")
    parser.add_argument("--expire", action="store_true")
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    if args.native_platform == "windows" and not (args.windows and args.windows_sha):
        parser.error("actual Windows pin and hash required")
    build = json.loads(args.build_receipt.read_text())
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
    assert {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in args.package.rglob("*") if path.is_file()} == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.local_only, args.failure_only, args.focus = args.native_platform == "linux", False, "lifecycle"
    LifecycleRunner(args).run()


if __name__ == "__main__":
    main()
