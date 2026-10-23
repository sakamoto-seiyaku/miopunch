#!/usr/bin/env python3
"""Actual independent Chromium/WASM file nodes, OPFS saves, and Native peers.

Public operations drive all business; the technical host reads bounded slices
of actual saved Files for an independent Node SHA256. It never compiles and
keeps node/profile state in a private temporary directory, outside receipts.
"""
import argparse
import hashlib
import json
from pathlib import Path
import time

from check_browser_onboarding import BrowserControl, CONFIGURATION
from check_file_transfer import FileRunner, TERMINAL
from check_native_client import Host


class BrowserFilesRunner(FileRunner):
    def __init__(self, args):
        super().__init__(args)
        self.args = args
        self.browser_private = self.private_root / "browser-profiles"
        self.browser_nodes = {}

    def task(self, host, identifier):
        if not isinstance(host, str):
            return super().task(host, identifier)
        tasks = self.browser.call(host, "file_status")["tasks"]
        return next((task for task in tasks if task["id"] == identifier), None)

    def native_creator(self, name):
        host = self.start(name, False)
        host.call("export_recovery_card", path=self.path(name + "-card.json", False))
        host.call("acknowledge_recovery_saved")
        return host

    def browser_member(self, native, name):
        invitation_name = name + "-invite.json"
        native.call("invite", path=self.path(invitation_name, False), text=True)
        invitation = Path(self.path(invitation_name, False)).read_text()
        status = self.browser.start(name, "join", invitation)
        self.browser_nodes[name] = status["id"]
        assert status["id"] != native.node and status["stage"] == "AwaitingApproval"
        assert self.browser.call(name, "file_api") == dict(send=True, receive=True, stream=True, opfs=True)
        self.browser.call(name, "file_send_source", native.node, "missing", ok=False)
        assert self.browser.call(name, "request_join", False) == "AwaitingApproval"
        pending = native.call("requests")["requests"]
        assert len(pending) == 1 and pending[0]["applicant"] == status["id"]
        native.call("approve", **pending[0])
        assert self.browser.call(name, "request_join", False) == "Complete"
        assert self.browser.reopen(name)["id"] == status["id"]
        return name

    def browser_creator(self, name):
        status = self.browser.start(name, "create")
        assert status["stage"] == "AwaitingRecoverySave"
        self.browser.call(name, "enable_files", ok=False)
        self.browser.call(name, "export_recovery_card")
        self.browser.call(name, "acknowledge_recovery_saved")
        self.browser_nodes[name] = status["id"]
        assert self.browser.reopen(name)["stage"] == "Complete"
        return name

    def native_member(self, browser, name):
        invitation = self.browser.call(browser, "invitation")
        path = Path(self.path(name + "-invite.json", False))
        path.write_text(invitation)
        path.chmod(0o600)
        native = self.start(name, False, "join", str(path))
        server = self.browser.send(browser, "serve_admission", False)
        assert native.call("request", path=str(path))["stage"] == "AwaitingApproval"
        assert self.browser.wait(server) == "AwaitingApproval"
        pending = self.browser.call(browser, "admission_requests")
        assert len(pending) == 1 and pending[0]["applicant"] == native.node
        self.browser.call(browser, "approve_admission", native.node, pending[0]["request"])
        server = self.browser.send(browser, "serve_admission", False)
        assert native.call("request", path=str(path))["stage"] == "Complete"
        assert self.browser.wait(server) == "Complete"
        assert self.browser_nodes[browser] != native.node
        return native

    def browser_peer(self, provider, name):
        invitation = self.browser.call(provider, "invitation")
        status = self.browser.start(name, "join", invitation)
        self.browser_nodes[name] = status["id"]
        server = self.browser.send(provider, "serve_admission", False)
        assert self.browser.call(name, "request_join", False) == "AwaitingApproval"
        assert self.browser.wait(server) == "AwaitingApproval"
        pending = [entry for entry in self.browser.call(provider, "admission_requests")
                   if entry["applicant"] == status["id"]]
        assert len(pending) == 1
        self.browser.call(provider, "approve_admission", status["id"], pending[0]["request"])
        server = self.browser.send(provider, "serve_admission", False)
        assert self.browser.call(name, "request_join", False) == "Complete"
        assert self.browser.wait(server) == "Complete"
        return name

    def stream_seam(self, a, b):
        accepting = self.browser.send(a, "member_accept", 60000)
        self.browser.call(b, "member_connect", self.browser_nodes[a])
        self.browser.wait(accepting)
        accepting = self.browser.send(a, "data_accept")
        self.browser.call(b, "data_open")
        self.browser.call(b, "data_write", 1, 7)
        self.browser.wait(accepting)
        assert self.browser.call(a, "data_read", 7) == dict(length=1, matches=True)
        # A pending read in each direction must permit the other direction's
        # write, while a second pending read is rejected instead of queued.
        reading_a = self.browser.send(a, "data_read", 17)
        reading_b = self.browser.send(b, "data_read", 23)
        self.browser.call(a, "data_read", 17, ok=False)
        self.browser.call(a, "data_write", 65536, 23)
        self.browser.call(b, "data_write", 65536, 17)
        assert self.browser.wait(reading_a) == dict(length=65536, matches=True)
        assert self.browser.wait(reading_b) == dict(length=65536, matches=True)
        self.browser.call(a, "data_write", 65537, 0, ok=False)
        reading_a = self.browser.send(a, "data_read", 0)
        self.browser.call(a, "data_cancel")
        self.browser.wait(reading_a, ok=False)
        self.browser.call(b, "data_cancel")
        for actor in [a, b]:
            self.browser.call(actor, "data_drop")
            assert not self.browser.call(actor, "member_call", "is_closed", [])
        # Stream cancellation retains the admitted member connection. Another
        # stream on it remains usable and is not a revived old stream.
        accepting = self.browser.send(a, "data_accept")
        self.browser.call(b, "data_open")
        self.browser.call(b, "data_write", 32, 11)
        self.browser.wait(accepting)
        assert self.browser.call(a, "data_read", 11) == dict(length=32, matches=True)
        for actor in [a, b]:
            self.browser.call(actor, "data_drop")
            self.browser.call(actor, "member_call", "close", [])
        self.record("stream", "actual_browser_stream_seam", dict(duplex=True, chunkBytes=65536,
                    oversizedRejected=True, secondPendingReadRejected=True,
                    cancelRead=True, cancellationRetainsMemberConnection=True, independentProfiles=True))

    def seams(self):
        provider = self.browser_creator("stream-provider")
        native = self.native_member(provider, "stream-native")
        peer = self.browser_peer(provider, "stream-peer")
        self.stream_seam(provider, peer)
        directory = self.enable(native, "browser-bounds")
        name, expected = self.generate_browser(provider, "bounded-browser-source.bin", 16 * 1024**2)
        tasks = [self.send(provider, native, name) for _ in range(4)]
        self.browser.call(provider, "file_send_source", native.node, name, ok=False)
        for identifier in tasks:
            sending = self.wait_task(provider, identifier)
            receiving = self.wait_task(native, identifier)
            assert sending["state"] == receiving["state"] == "complete", (sending, receiving)
            assert sending["receipt"] == receiving["receipt"]
            destination = directory + "/" + receiving["receipt"]["saved_name"]
            assert self.hash_file(destination, False) == expected
            self.remove(destination, False)
        assert not self.listing(directory, False)
        self.record(provider, "actual_browser_task_bound", dict(completed=4, fifthRejected=True, savedHashes=expected))
        native.call("files_disable")
        native.stop()
        for actor in [provider, peer]:
            self.browser.call(actor, "enable_files")
        for source, receiver in [(provider, peer), (peer, provider)]:
            name, expected = self.generate_browser(source, source + "-to-browser.bin", 4 * 1024**2)
            self.delivered_mixed(source, receiver, name, expected)
        for actor in [provider, peer]:
            self.browser.call(actor, "disable_files")

    def generate_browser(self, actor, name, size):
        assert self.browser.call(actor, "file_make", name, size) == dict(name=name, size=size)
        result = self.browser.call(actor, "file_hash", name, True)
        assert result["size"] == size
        return name, result

    def send(self, source, target, path):
        if isinstance(source, str):
            target_id = self.browser_nodes[target] if isinstance(target, str) else target.node
            return self.browser.call(source, "file_send_source", target_id, path)
        address = target.address if not isinstance(target, str) else dict(
            id=self.browser_nodes[target], addrs=[{"Relay": CONFIGURATION["relays"][0]}])
        # This is the permitted public relay from the browser's configuration,
        # addressed to that browser's independent public identity. No invitation
        # authority or host tunnel is used to obtain a business route.
        return source.call("file_send", address=address, path=path)["task"]

    def cancel(self, host, identifier):
        if isinstance(host, str):
            self.browser.call(host, "cancel_file", identifier)
        else:
            host.call("file_cancel", task=identifier)

    def disconnect(self, host, identifier):
        if isinstance(host, str):
            self.browser.call(host, "disconnect_file", identifier)
        else:
            host.call("file_disconnect", task=identifier)

    def wait_live(self, source, target, identifier):
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            sending, receiving = self.task(source, identifier), self.task(target, identifier)
            if sending and sending["state"] in TERMINAL:
                raise AssertionError(dict(stage="waiting_for_live_transfer", sender=sending, receiver=receiving))
            if receiving and receiving["state"] in TERMINAL:
                raise AssertionError(dict(stage="waiting_for_live_transfer", sender=sending, receiver=receiving))
            if receiving and receiving["state"] == "transferring" and receiving["bytes"] > 0:
                return receiving
            time.sleep(0.02)
        raise AssertionError(dict(stage="live_transfer_controller_deadline", sender=self.task(source, identifier), receiver=self.task(target, identifier)))

    def cancel_publication(self, native, browser, source, expected):
        for cancelling in [browser, native]:
            self.browser.call(browser, "file_hold_publish")
            try:
                task = self.send(native, browser, source)
                deadline = time.monotonic() + 90
                while not self.browser.call(browser, "file_publish_pending"):
                    assert time.monotonic() < deadline
                    time.sleep(0.02)
                self.cancel(cancelling, task)
                deadline = time.monotonic() + 30
                while self.task(browser, task)["state"] != "cancelling":
                    assert time.monotonic() < deadline
                    time.sleep(0.02)
                assert self.task(browser, task)["receipt"] is None
            finally:
                self.browser.call(browser, "file_release_publish")
            assert self.wait_task(browser, task)["state"] == "cancelled"
            assert self.wait_task(native, task)["state"] == "cancelled"
            assert self.browser.call(browser, "file_list") == []
            self.record(browser, "cancellation_owns_actual_publication", dict(
                cancelSource="receiver" if isinstance(cancelling, str) else "sender",
                lateFileRemoved=True, actualSize=expected["size"]))

    def delivered_mixed(self, source, target, path, expected, directory=None, interrupt=False, hold=False):
        if hold:
            assert isinstance(target, str)
            self.browser.call(target, "file_hold_save")
        identifier = self.send(source, target, path)
        if interrupt:
            self.wait_live(source, target, identifier)
            self.disconnect(source, identifier)
        if hold:
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                receiving = self.task(target, identifier)
                if receiving and receiving["state"] == "saving":
                    break
                time.sleep(0.1)
            assert receiving["state"] == "saving"
            sending = self.task(source, identifier)
            assert sending["state"] != "complete" and sending["receipt"] is None
            assert not any(name.startswith(identifier + "-") for name in self.browser.call(target, "file_list"))
            self.browser.call(target, "file_release_save")
        sending = self.wait_task(source, identifier)
        receiving = self.wait_task(target, identifier)
        assert sending["state"] == receiving["state"] == "complete", (sending, receiving)
        assert sending["receipt"] == receiving["receipt"]
        receipt = sending["receipt"]
        assert {key: receipt[key] for key in ["size", "sha256"]} == expected
        if isinstance(target, str):
            actual = self.browser.call(target, "file_hash", receipt["saved_name"], False)
            assert actual == expected
            # Saved file remains a real OPFS File after the node closes/reopens;
            # reopening retains identity but requires another explicit enable.
            before = self.browser.call(target, "status")
            assert self.browser.reopen(target) == before
            assert self.browser.call(target, "file_status")["enabled"] is False
            assert self.browser.call(target, "file_hash", receipt["saved_name"], False) == expected
            self.browser.call(target, "enable_files")
            self.browser.call(target, "file_remove", receipt["saved_name"])
            assert self.browser.call(target, "file_list") == []
        else:
            destination = directory + "/" + receipt["saved_name"]
            assert self.hash_file(destination, False) == expected
            self.remove(destination, False)
            assert not self.listing(directory, False)
        if interrupt:
            assert sending["attempts"] >= 2 and sending["resumed_bytes"] > 0, sending
        assert any(path["selected"] for path in sending["transport"]["paths"])
        self.qualifications.append(dict(source=source if isinstance(source, str) else source.label,
            receiver=target if isinstance(target, str) else target.label, **expected,
            receipt=receipt, attempts=sending["attempts"], resumed_bytes=sending["resumed_bytes"],
            holdSave=hold, resumed=interrupt, actualOPFS=isinstance(target, str), transport=sending["transport"]))

    def mixed(self, native, browser, prefix):
        functional = self.focus == "functional"
        # Both public services are initially off, and neither endpoint writes.
        native_path, small = self.generate(native, prefix + "-disabled.bin", 11)
        task = self.send(native, browser, native_path)
        failed = self.wait_task(native, task)
        assert failed["state"] == "failed" and failed["error"] == "service_disabled", failed
        self.browser.call(browser, "enable_files")
        directory = self.enable(native, prefix)
        # Sizes are qualification inputs, not product ceilings. Native separately
        # exercises 64/256 MiB; 16 MiB here exceeds the former demo's 4 MiB cap
        # and keeps the real WASM/storage matrix bounded on these resources.
        sizes = [0, 11] if self.focus == "smoke" else [0, 11, 4 * 1024**2, 16 * 1024**2]
        for index, size in enumerate(sizes):
            path, expected = self.generate(native, prefix + f"-Native中文 {index}.bin", size)
            self.delivered_mixed(native, browser, path, expected, hold=size == 11 and not functional)
            self.remove(path, False)
            name, expected = self.generate_browser(browser, prefix + f"-Browser中文 {index}.bin", size)
            self.delivered_mixed(browser, native, name, expected, directory)
        # Abort an actual OPFS writable before commit. No successful receipt is
        # possible, and both its partial and incomplete destination are removed.
        if not functional:
            self.browser.call(browser, "file_abort_save")
            task = self.send(native, browser, native_path)
            failed = self.wait_task(native, task)
            assert failed["state"] == "failed" and failed["error"] == "save_failed", failed
            assert self.wait_task(browser, task)["state"] == "failed"
            assert self.browser.call(browser, "file_list") == []
            self.cancel_publication(native, browser, native_path, small)
        self.remove(native_path, False)
        if self.focus != "smoke":
            path, expected = self.generate(native, prefix + "-resume.bin", 16 * 1024**2)
            self.delivered_mixed(native, browser, path, expected, interrupt=True)
            task = self.send(native, browser, path)
            self.wait_live(native, browser, task)
            if functional:
                self.cancel(browser, task)
            else:
                self.browser.call(browser, "file_hold_abort")
                try:
                    self.cancel(browser, task)
                    deadline = time.monotonic() + 30
                    while not self.browser.call(browser, "file_abort_pending"):
                        assert time.monotonic() < deadline
                        time.sleep(0.02)
                    assert self.task(browser, task)["state"] not in TERMINAL
                    assert self.browser.call(browser, "file_list")
                    self.record(browser, "cancel_waits_for_actual_OPFS_cleanup", dict(terminalBeforeAbort=False))
                finally:
                    self.browser.call(browser, "file_release_abort")
            assert self.wait_task(native, task)["state"] == "cancelled"
            assert self.wait_task(browser, task)["state"] == "cancelled"
            assert self.browser.call(browser, "file_list") == []
            self.remove(path, False)
            name, expected = self.generate_browser(browser, prefix + "-browser-resume.bin", 16 * 1024**2)
            self.delivered_mixed(browser, native, name, expected, directory, interrupt=True)
        self.browser.call(browser, "disable_files")
        native.call("files_disable")
        native.stop()

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.browser = BrowserControl(self, self.args)
            if self.focus == "seams":
                self.seams()
            else:
                native = self.native_creator("native-provider")
                browser = self.browser_member(native, "browser-member")
                self.mixed(native, browser, "native-creator")
                browser = self.browser_creator("browser-provider")
                native = self.native_member(browser, "native-member")
                self.mixed(native, browser, "browser-creator")
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
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed, qualifications=self.qualifications,
                browserCleanup=browser_cleanup, cleanup=cleanup,
                controller_sha256=self.controller_sources[str(Path(__file__).resolve())],
                controllerSourceDrift=drift,
                wasmPackage=self.args.package_sha, sourceAndSavedHashesIndependent=True,
                businessData="actual admitted SDK connection; browser saved-file reads only for evidence"), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--focus", choices=["smoke", "full", "seams", "functional"], default="full")
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    build = json.loads(args.build_receipt.read_text())
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
    actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in args.package.rglob("*") if path.is_file()}
    assert actual == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only = True, False
    BrowserFilesRunner(args).run()


if __name__ == "__main__":
    main()
