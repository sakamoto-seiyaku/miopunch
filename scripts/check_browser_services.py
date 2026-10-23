#!/usr/bin/env python3
"""Real Native/WASM service ACL, guest confirmation and OPFS acceptance.

Uses ordinary public operations in a minimal Chromium host. There are no
prototype replacements, injected storage faults, or business proxy paths.
"""
import argparse
import hashlib
import json
from pathlib import Path
import time

from check_browser_onboarding import BrowserControl, CONFIGURATION
from check_service_guests import GuestRunner


class BrowserServiceRunner(GuestRunner):
    def __init__(self, args):
        super().__init__(args)
        self.args = args
        self.browser_private = self.private_root / "browser-profiles"
        self.browser_nodes = {}

    def task(self, host, task):
        if isinstance(host, str):
            return next((t for t in self.browser.call(host, "file_status")["tasks"] if t["id"] == task), None)
        return super().task(host, task)

    def identity(self, host):
        return self.browser_nodes[host] if isinstance(host, str) else host.node

    def address_of(self, host):
        return dict(id=self.identity(host), addrs=[{"Relay": CONFIGURATION["relays"][0]}]) if isinstance(host, str) else host.address

    def native_creator(self, name):
        host = self.start(name, False)
        host.call("export_recovery_card", path=self.path(name + "-card.json", False))
        host.call("acknowledge_recovery_saved")
        return host

    def browser_creator(self, name):
        status = self.browser.start(name, "create")
        self.browser_nodes[name] = status["id"]
        assert status["stage"] == "AwaitingRecoverySave"
        self.browser.call(name, "export_recovery_card")
        self.browser.call(name, "acknowledge_recovery_saved")
        return name

    def browser_member(self, issuer, name):
        filename = name + "-invite.json"
        issuer.call("issue_precode", path=self.path(filename, False), seconds=3600, max_uses=1)
        invitation = Path(self.path(filename, False)).read_text()
        status = self.browser.start(name, "join", invitation)
        self.browser_nodes[name] = status["id"]
        assert self.browser.call(name, "request_join", False) == "Complete"
        return name

    def native_member(self, issuer, name):
        invitation = self.browser.call(issuer, "invitation")
        path = Path(self.path(name + "-invite.json", False))
        path.write_text(invitation)
        path.chmod(0o600)
        member = self.start(name, False, "join", str(path))
        serving = self.browser.send(issuer, "serve_admission", False)
        assert member.call("request", path=str(path))["stage"] == "AwaitingApproval"
        assert self.browser.wait(serving) == "AwaitingApproval"
        pending = [r for r in self.browser.call(issuer, "admission_requests") if r["applicant"] == member.node]
        assert len(pending) == 1
        self.browser.call(issuer, "approve_admission", member.node, pending[0]["request"])
        serving = self.browser.send(issuer, "serve_admission", False)
        assert member.call("request", path=str(path))["stage"] == "Complete"
        assert self.browser.wait(serving) == "Complete"
        return member

    def make_browser(self, actor, name, size):
        assert self.browser.call(actor, "file_make", name, size) == dict(name=name, size=size)
        expected = self.browser.call(actor, "file_hash", name, True)
        assert expected["size"] == size
        return name, expected

    def send_mixed(self, source, target, path):
        if isinstance(source, str):
            return self.browser.call(source, "file_send_source", self.identity(target), path)
        return source.call("file_send", address=self.address_of(target), path=path)["task"]

    def confirm(self, provider, task, approve):
        if isinstance(provider, str):
            return self.browser.call(provider, "approve_file" if approve else "reject_file", task)
        result = provider.call("file_approve" if approve else "file_reject", task=task)
        return result["approved" if approve else "rejected"]

    def empty(self, provider, directory=None):
        return self.browser.call(provider, "file_list") == [] if isinstance(provider, str) else not self.listing(directory, False)

    def saved_mixed(self, source, target, task, expected, directory=None, retain=False):
        sent, received = self.wait_task(source, task), self.wait_task(target, task)
        assert sent["state"] == received["state"] == "complete", (sent, received)
        assert sent["receipt"] == received["receipt"]
        receipt = received["receipt"]
        assert dict(size=receipt["size"], sha256=receipt["sha256"]) == expected
        if isinstance(target, str):
            assert self.browser.call(target, "file_hash", receipt["saved_name"], False) == expected
            if not retain:
                self.browser.call(target, "file_remove", receipt["saved_name"])
        else:
            destination = directory + "/" + receipt["saved_name"]
            assert self.hash_file(destination, False) == expected
            if not retain:
                self.remove(destination, False)
        self.qualifications.append(dict(source=self.identity(source), receiver=self.identity(target), **expected,
            receipt=receipt, actualOPFS=isinstance(target, str), transport=sent["transport"]))
        return receipt

    def stopped(self, source, provider, task, cause, directory=None):
        sent, received = self.wait_task(source, task), self.wait_task(provider, task)
        assert sent["state"] in {"failed", "cancelled"} and sent["receipt"] is None, sent
        assert received["state"] in {"failed", "cancelled"} and received["receipt"] is None, received
        assert 0 < received["bytes"] < received["size"], received
        assert self.empty(provider, directory)
        self.record(self.identity(provider), cause, dict(bytesBeforeStop=received["bytes"], total=received["size"],
            noReceipt=True, noSavedOrPartial=True, transport=received["transport"]))

    def wait_progress(self, source, provider, task):
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            sent, received = self.task(source, task), self.task(provider, task)
            if sent and sent["state"] in {"complete", "failed", "cancelled"}:
                raise AssertionError(dict(stage="real_progress_prerequisite", sender=sent, receiver=received))
            if received and received["state"] == "transferring" and received["bytes"] > 0:
                return received
            time.sleep(0.02)
        raise AssertionError("no real file bytes observed")

    def guest_browser(self, issuer, provider, name, seconds=180):
        if isinstance(issuer, str):
            invitation = self.browser.call(issuer, "issue_guest", json.dumps(self.address_of(provider)), int(time.time()) + seconds)
        else:
            filename = name + "-invite.json"
            issuer.call("guest_issue", path=self.path(filename, False), provider=self.address_of(provider), seconds=seconds)
            invitation = Path(self.path(filename, False)).read_text()
        status = self.browser.start(name, "join_guest", invitation)
        self.browser_nodes[name] = status["id"]
        result = self.browser.call(name, "redeem_guest")
        assert result["stage"] == "GuestActive"
        if not isinstance(issuer, str):
            issuer.call("service_sync", address=self.address_of(provider))
        return name, result["scope"], invitation

    def native_admin_scenario(self):
        admin = self.native_creator("native-admin")
        provider = self.browser_member(admin, "wasm-provider")
        caller = self.member(admin, "native-caller")
        self.browser.call(provider, "enable_files")
        admin.call("service_sync", address=self.address_of(provider))
        caller.call("service_sync", address=admin.address)
        source, expected = self.generate(caller, "member-to-opfs.bin", 11)
        self.saved_mixed(caller, provider, self.send_mixed(caller, provider, source), expected)
        rule = dict(members=True, guests=True, allowed=[], denied=[caller.node])
        caller.call("service_rule", ok=False, provider=self.identity(provider), service="files", previous=None, rule=rule)
        revision = admin.call("service_rule", provider=self.identity(provider), service="files", previous=None, rule=rule)["revision"]
        admin.call("service_sync", address=self.address_of(provider))
        effective = self.browser.call(provider, "service_status")["services"][0]["policy"]
        assert effective["revision"] == revision and effective["rule"] == rule
        task = self.send_mixed(caller, provider, source)
        failed = self.wait_task(caller, task)
        assert self.task(provider, task) is None and self.empty(provider)
        assert failed["state"] in {"failed", "cancelled"} and failed["receipt"] is None, failed
        assert self.browser.call(provider, "file_status")["enabled"]
        restored = self.browser.call(provider, "set_service_rule", self.identity(provider), "files", revision,
            json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
        self.browser.call(provider, "sync_services", admin.node)
        time.sleep(2)
        self.saved_mixed(caller, provider, self.send_mixed(caller, provider, source), expected)
        self.record(provider, "real_wasm_single_acl", dict(remoteAndLocalSameRule=True, providerEnforced=True, unauthorizedChangeNoEffect=True))
        guest, scope, invitation = self.guest_browser(admin, provider, "wasm-guest")
        second = self.browser.start("wasm-second-identity", "join_guest", invitation)
        self.browser_nodes["wasm-second-identity"] = second["id"]
        self.browser.call("wasm-second-identity", "redeem_guest", ok=False)
        source, expected = self.make_browser(guest, "guest-confirmed.bin", 4 * 1024**2)
        self.browser.call(guest, "file_send_source", admin.node, source, ok=False)
        task = self.send_mixed(guest, provider, source)
        self.pending(provider, task)
        assert self.empty(provider) and self.task(guest, task)["receipt"] is None
        assert self.confirm(provider, task, False)
        failed = self.wait_task(guest, task)
        assert failed["state"] == "failed" and failed["error"] == "rejected" and failed["receipt"] is None
        assert self.empty(provider)
        task = self.send_mixed(guest, provider, source)
        self.pending(provider, task)
        assert self.empty(provider)
        assert self.confirm(provider, task, True)
        receipt = self.saved_mixed(guest, provider, task, expected, retain=True)
        identity = self.browser_nodes[provider]
        assert self.browser.reopen(provider)["id"] == identity
        assert not self.browser.call(provider, "file_status")["enabled"]
        assert self.browser.call(provider, "service_status")["services"][0]["policy"]["revision"] == restored
        assert self.browser.call(provider, "file_hash", receipt["saved_name"], False) == expected
        self.browser.call(provider, "file_remove", receipt["saved_name"])
        self.browser.call(provider, "enable_files")
        self.browser.call(provider, "sync_services", admin.node)
        assert self.browser.reopen(guest)["id"] == self.browser_nodes[guest]
        assert self.browser.call(guest, "redeem_guest")["scope"]["expires"] == scope["expires"]
        source, _ = self.make_browser(guest, "guest-confirmed.bin", 4 * 1024**2)
        task = self.send_mixed(guest, provider, source)
        self.pending(provider, task)
        self.confirm(provider, task, True)
        self.saved_mixed(guest, provider, task, expected)
        result = self.browser.call(guest, "end_guest")
        assert result["ended_locally"] and all(item["saved"] for item in result["delivery"]), result
        assert self.browser.call(guest, "guest_status")["stage"] == "GuestEnded"
        self.browser.call(guest, "file_send_source", self.identity(provider), source, ok=False)
        self.record(guest, "actual_wasm_guest_confirmation", dict(oneIdentity=True, pendingNoOPFSFile=True,
            rejectionNoSave=True, approvalActualSave=True, reloadIdentityAndDeadline=True, exitDelivered=True))
        if self.focus == "full":
            self.browser_lifecycle(admin, provider, caller)
        admin.stop()
        caller.stop()

    def browser_lifecycle(self, admin, provider, caller):
        source, _ = self.generate(caller, "live-native-to-wasm.bin", 256 * 1024**2)
        task = self.send_mixed(caller, provider, source)
        self.wait_progress(caller, provider, task)
        previous = self.browser.call(provider, "service_status")["services"][0]["policy"]["revision"]
        revision = admin.call("service_rule", provider=self.identity(provider), service="files", previous=previous,
            rule=dict(members=True, guests=True, allowed=[], denied=[caller.node]))["revision"]
        admin.call("service_sync", address=self.address_of(provider))
        self.stopped(caller, provider, task, "received_acl_stops_real_OPFS_transfer")
        self.remove(source, False)
        self.browser.call(provider, "set_service_rule", self.identity(provider), "files", revision,
            json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
        self.browser.call(provider, "sync_services", admin.node)
        guest, scope, _ = self.guest_browser(admin, provider, "wasm-live-revoked-guest")
        source, _ = self.make_browser(guest, "revoked-browser-file.bin", 128 * 1024**2)
        task = self.send_mixed(guest, provider, source)
        self.pending(provider, task)
        self.confirm(provider, task, True)
        self.wait_progress(guest, provider, task)
        admin.call("guest_revoke", invitation=scope["invitation"])
        admin.call("service_sync", address=self.address_of(provider))
        self.stopped(guest, provider, task, "received_guest_revocation_stops_real_OPFS_transfer")
        expiring, scope, _ = self.guest_browser(provider, provider, "wasm-expiring-guest", 90)
        source, _ = self.make_browser(expiring, "expiring-browser-file.bin", 256 * 1024**2)
        # Source generation, independent hashing and the SDK hash happen
        # before confirmation. Keep its wait below the protocol's 60s bound,
        # then allow real OPFS progress before the unchanged creation deadline.
        while (remaining := scope["created"] + 30 - time.time()) > 0:
            time.sleep(min(0.1, remaining))
        task = self.send_mixed(expiring, provider, source)
        self.pending(provider, task, expiring)
        while (remaining := scope["expires"] - time.time() - 10) > 0:
            time.sleep(min(0.1, remaining))
        assert time.time() < scope["expires"] - 2
        assert self.confirm(provider, task, True)
        self.wait_progress(expiring, provider, task)
        self.stopped(expiring, provider, task, "original_guest_deadline_stops_real_OPFS_transfer")
        assert self.browser.call(expiring, "guest_status")["stage"] == "GuestExpired"
        self.browser.call(expiring, "file_send_source", self.identity(provider), source, ok=False)

    def browser_admin_scenario(self):
        admin = self.browser_creator("wasm-admin")
        provider = self.native_member(admin, "native-provider")
        caller = self.native_member(admin, "native-peer")
        directory = self.enable(provider, "native-provider")
        self.browser.call(admin, "sync_services", provider.node)
        self.browser.call(admin, "sync_services", caller.node)
        source, expected = self.generate(caller, "browser-admin-member.bin", 11)
        self.saved_mixed(caller, provider, self.send_mixed(caller, provider, source), expected, directory)
        revision = self.browser.call(admin, "set_service_rule", provider.node, "files", None,
            json.dumps(dict(members=True, guests=True, allowed=[], denied=[caller.node])))
        self.browser.call(admin, "sync_services", provider.node)
        task = self.send_mixed(caller, provider, source)
        failed = self.wait_task(caller, task)
        assert failed["state"] == "failed" and failed["receipt"] is None
        assert self.task(provider, task) is None and self.empty(provider, directory)
        provider.call("service_rule", provider=provider.node, service="files", previous=revision,
            rule=dict(members=True, guests=True, allowed=[], denied=[]))
        provider.call("service_sync", address=self.address_of(admin))
        invitation = self.browser.call(admin, "issue_guest", json.dumps(provider.address), int(time.time()) + 180)
        path = Path(self.path("browser-issued-guest-invite.json", False))
        path.write_text(invitation)
        path.chmod(0o600)
        guest = self.start("native-guest", False, "join-guest", str(path))
        status = guest.call("guest_redeem")
        assert status["stage"] == "GuestActive"
        self.browser.call(admin, "sync_services", provider.node)
        source, expected = self.generate(guest, "wasm-issued-confirmed.bin", 4 * 1024**2)
        task = self.send_mixed(guest, provider, source)
        self.pending(provider, task)
        assert self.empty(provider, directory)
        self.confirm(provider, task, True)
        self.saved_mixed(guest, provider, task, expected, directory)
        self.browser.reopen(admin)
        assert guest.call("guest_redeem")["scope"]["expires"] == status["scope"]["expires"]
        if self.focus == "full":
            source, _ = self.generate(guest, "native-exit-live.bin", 256 * 1024**2)
            task = self.send_mixed(guest, provider, source)
            self.pending(provider, task)
            self.confirm(provider, task, True)
            self.wait_progress(guest, provider, task)
            result = guest.call("guest_end")
            self.stopped(guest, provider, task, "active_guest_exit_stops_real_native_transfer", directory)
        else:
            result = guest.call("guest_end")
        assert result["ended_locally"] and all(item["saved"] for item in result["delivery"]), result
        source, expected = self.make_browser(admin, "browser-admin-after-cleanup.bin", 11)
        self.saved_mixed(admin, provider, self.send_mixed(admin, provider, source), expected, directory)
        self.record(admin, "actual_wasm_administrator_parity", dict(remoteACLNativeEnforced=True, invitationBoundAndDurable=True,
            nativeGuestConfirmedFile=True, noGenesisForDailyACL=True, noGuestAdmissionSecret=True))
        for host in (guest, caller, provider):
            host.stop()

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.browser = BrowserControl(self, self.args)
            self.native_admin_scenario()
            self.browser_admin_scenario()
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
                browserCleanup=browser_cleanup, cleanup=cleanup, controllerSourceDrift=drift,
                wasmPackage=self.args.package_sha, noPrototypeHooks=True, independentProfiles=True,
                businessData="real Native/WASM and WASM/WASM SDK streams; independent saved reads"), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--focus", choices=["smoke", "full"], default="full")
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    args.harness = Path(__file__).resolve().parents[1] / "browser-probe/harness/service-policy.mjs"
    build = json.loads(args.build_receipt.read_text())
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
    actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in args.package.rglob("*") if path.is_file()}
    assert actual == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only = True, False
    BrowserServiceRunner(args).run()


if __name__ == "__main__":
    main()
