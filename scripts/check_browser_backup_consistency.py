#!/usr/bin/env python3
"""Real Browser backup ownership, guest history, active I/O and resource policy.

Uses ordinary public APIs in separate Chromium profiles plus a Native peer.
No prototype, storage-fault or clock interposition. Capsules remain private.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

from check_browser_device_management import BrowserDeviceRunner
from check_browser_onboarding import CONFIGURATION
from check_native_client import Host


class BrowserBackupRunner(BrowserDeviceRunner):
    def native_applicant(self, issuer, name, invitation=None, precode=False):
        invitation = invitation or self.browser.call(issuer, "invitation")
        path = Path(self.path(name + "-invite.json", False))
        path.write_text(invitation); path.chmod(0o600)
        applicant = self.start(name, False, "join", str(path))
        serving = self.browser.send(issuer, "serve_admission", precode)
        stage = applicant.call("request", path=str(path))["stage"]
        assert self.browser.wait(serving) == stage
        return applicant, str(path)

    def restore_browser(self, actor, backup):
        self.browser.call(actor, "open")
        status = self.browser.call(actor, "start_replacement", "{}", backup)
        self.browser_nodes[actor] = status["id"]
        assert status["stage"] == "RestoringAuthorization"
        self.browser.call(actor, "enable_files", ok=False)
        return actor

    def portable(self):
        source = "browser-backup-source"
        disabled = "https://use1-1.relay.n0.iroh.link./"
        configuration = dict(CONFIGURATION, disabled=[disabled])
        status = self.browser.start(source, "create", configuration=configuration)
        self.browser_nodes[source] = status["id"]
        self.browser.call(source, "export_recovery_card")
        self.browser.call(source, "acknowledge_recovery_saved")
        peer = self.native_member(source, "browser-backup-native-peer")
        pending, pending_invite = self.native_applicant(source, "browser-pending")
        assert pending.call("status")["stage"] == "AwaitingApproval"
        approved, approved_invite = self.native_applicant(source, "browser-approved")
        rejected, rejected_invite = self.native_applicant(source, "browser-rejected")
        requests = self.browser.call(source, "admission_requests")
        pending_request = next(row for row in requests if row["applicant"] == pending.node)
        approved_request = next(row for row in requests if row["applicant"] == approved.node)
        rejected_request = next(row for row in requests if row["applicant"] == rejected.node)
        original_grant = self.browser.call(source, "approve_admission", approved.node, approved_request["request"])
        serving = self.browser.send(source, "serve_admission", False)
        assert approved.call("request", path=approved_invite, authorization_only=True)["stage"] == "PendingMaterials"
        self.browser.wait(serving, ok=False)  # actual sender closes before secret
        self.browser.call(source, "reject_admission", rejected.node, rejected_request["request"])
        serving = self.browser.send(source, "serve_admission", False)
        assert rejected.call("request", path=rejected_invite)["stage"] == "Rejected"
        assert self.browser.wait(serving) == "Rejected"
        old_code = self.browser.call(source, "precode_invitation", int(time.time()) + 3600, 3)
        consumed, consumed_invite = self.native_applicant(source, "browser-code-consumed", old_code, True)
        assert consumed.call("status")["stage"] == "Complete"
        directory = self.enable(peer, "guest-native-provider")
        guest, scope, _ = self.guest_browser(source, peer, "guest-backup-source", seconds=240)
        guest_identity = self.identity(guest)
        self.sync(source, peer)
        assert any(item["guest"] == guest_identity and item["expires"] == scope["expires"]
            for item in peer.call("service_status")["guests"])
        guest_backup = self.browser.call(guest, "prepare_node_replacement")
        guest_target = self.restore_browser("guest-backup-target", guest_backup)
        assert self.identity(guest_target) == guest_identity
        completed = self.browser.call(guest_target, "complete_node_replacement", peer.node)
        assert completed["ready"]
        assert self.browser.call(guest_target, "guest_status")["scope"]["expires"] == scope["expires"]
        path, expected = self.make_browser(guest_target, "migrated-guest.bin", 11)
        task = self.send_mixed(guest_target, peer, path)
        try:
            self.pending(peer, task)
        except Exception:
            self.record(guest_target, "guest_file_failure_independent_state", dict(
                sender=self.browser.call(guest_target, "file_status"),
                providerFiles=peer.call("files_status"), providerServices=peer.call("service_status")))
            raise
        self.confirm(peer, task, True)
        self.saved_mixed(guest_target, peer, task, expected, directory)
        self.browser.call(guest, "static_call", "finalize_node_replacement", json.dumps(completed["receipt"]))
        guest_second = self.browser.call(guest_target, "prepare_node_replacement")
        guest_path = Path(self.path("guest-browser-native.node-backup", False)); guest_path.write_text(guest_second); guest_path.chmod(0o600)
        native_guest = Host(self, "guest-native-target", False, "restore", self.path("guest-native-state", False), str(guest_path))
        assert native_guest.node == guest_identity and native_guest.ready["status"]["stage"] == "RestoringAuthorization"
        native_guest.call("files_enable", directory=self.path("guest-premature-files", False), ok=False)
        native_guest_complete = native_guest.call("replacement_complete", address=peer.address, receipt_path=self.path("guest-native-receipt.json", False))
        assert native_guest_complete["ready"] and native_guest.call("guest_status")["scope"]["expires"] == scope["expires"]
        self.browser.call(guest_target, "static_call", "finalize_node_replacement", json.dumps(native_guest_complete["receipt"]))
        path, native_guest_expected = self.generate(native_guest, "native-guest-after-migration.bin", 11)
        task = self.send_mixed(native_guest, peer, path)
        self.pending(peer, task); self.confirm(peer, task, True)
        self.saved_mixed(native_guest, peer, task, native_guest_expected, directory)
        guest_third = self.path("guest-native-browser.node-backup", False)
        native_guest.call("replacement_prepare", path=guest_third)
        native_guest.process.stdin.close(); assert native_guest.process.wait(timeout=20) == 0; native_guest.stderr.close()
        guest_final = self.restore_browser("guest-browser-final", Path(guest_third).read_text())
        assert self.identity(guest_final) == guest_identity
        guest_final_complete = self.browser.call(guest_final, "complete_node_replacement", peer.node)
        assert guest_final_complete["ready"] and self.browser.call(guest_final, "guest_status")["scope"]["expires"] == scope["expires"]
        final_receipt = Path(self.path("guest-final-receipt.json", False)); final_receipt.write_text(json.dumps(guest_final_complete["receipt"]))
        finalized = subprocess.run([str(self.linux), "finalize-replacement", native_guest.directory, str(final_receipt)], capture_output=True, text=True, timeout=90)
        assert finalized.returncode == 0 and json.loads(finalized.stdout)["source_retired"]
        assert not json.loads(finalized.stdout)["network_departure_published"]
        self.close_actor(guest_final)
        self.browser.call(source, "revoke_guest", scope["invitation"])
        self.sync(source, peer)
        revoked = self.restore_browser("guest-revoked-old-snapshot", guest_backup)
        self.browser.call(revoked, "complete_node_replacement", peer.node, ok=False)
        assert not self.browser.call(revoked, "node_replacement_status")["ready"]
        self.browser.call(revoked, "enable_files", ok=False)
        self.close_actor(revoked)
        expiring, expiry_scope, _ = self.guest_browser(source, peer, "guest-expiring-source", seconds=45)
        expiring_backup = self.browser.call(expiring, "prepare_node_replacement")
        self.record(source, "guest_backup_scope_and_revocation", dict(identity=guest_identity,
            originalDeadline=scope["expires"], exactIdentityAndScopeRestored=True,
            actualNativeSave=expected, nativeGuestSavedHash=native_guest_expected,
            browserNativeBrowserGuest=True, originalDeadlineUnchanged=True,
            bothSourceTypesRetired=True, currentRevocationDeniedOldCapsule=True))

        # An actual ordinary transfer is in progress before source ownership is
        # joined. No active transport/task is resumed from the identity capsule.
        self.browser.call(source, "enable_files")
        path, retained = self.generate(peer, "before-browser-backup.bin", 11)
        saved = self.saved_mixed(peer, source, self.send_mixed(peer, source, path), retained, retain=True)
        large, _ = self.generate(peer, "active-during-backup.bin", 64 * 1024**2)
        active = self.send_mixed(peer, source, large)
        progress = self.wait_progress(peer, source, active)
        first_backup = self.browser.call(source, "prepare_node_replacement")
        terminal = self.wait_task(peer, active)
        assert terminal["state"] in {"failed", "cancelled"} and terminal["receipt"] is None
        assert self.browser.call(source, "file_list") == [saved["saved_name"]]
        package = json.loads(first_backup)["package"]
        self.browser.call(source, "static_call", "start", "{}", ok=False)
        self.browser.call(source, "static_call", "abort_node_replacement", package["id"], False, ok=False)
        self.browser.call(source, "static_call", "abort_node_replacement", "00" * 32, True, ok=False)
        self.browser.call(source, "static_call", "start", "{}", ok=False)
        self.browser.call(source, "static_call", "abort_node_replacement", package["id"], True)
        assert self.browser.call(source, "start", "start", "{}", None)["id"] == status["id"]
        backup = self.browser.call(source, "prepare_node_replacement")
        target = self.restore_browser("browser-backup-target", backup)
        assert self.identity(target) == status["id"]
        result = self.browser.call(target, "complete_node_replacement", peer.node)
        assert result["ready"]
        assert self.browser.call(target, "file_list") == []
        assert self.browser.call(target, "credential_custody")["holds_genesis"]
        restored_requests = self.browser.call(target, "admission_requests")
        assert any(row["applicant"] == pending.node and not row["approved"] and not row["rejected"] for row in restored_requests)
        assert any(row["applicant"] == rejected.node and row["rejected"] for row in restored_requests)
        assert self.browser.call(target, "approve_admission", approved.node, approved_request["request"]) == original_grant
        self.browser.call(target, "approve_admission", rejected.node, rejected_request["request"], ok=False)
        old_path = Path(self.path("browser-old-new-invite.json", False)); old_path.write_text(old_code); old_path.chmod(0o600)
        old_new = self.start("browser-old-new-attempt", False, "join", str(old_path))
        serving = self.browser.send(target, "serve_admission", True)
        old_new.call("request", path=str(old_path), ok=False)
        self.browser.wait(serving, ok=False)
        wrong = json.loads(json.dumps(result["receipt"]))
        wrong["package"]["id"] = "00" * 32
        self.browser.call(source, "static_call", "finalize_node_replacement", json.dumps(wrong), ok=False)
        self.browser.call(source, "static_call", "start", "{}", ok=False)
        self.browser.call(source, "static_call", "finalize_node_replacement", json.dumps(result["receipt"]))
        again = self.browser.call(source, "static_call", "finalize_node_replacement", json.dumps(result["receipt"]))
        assert again["already_finalized"] and not again["network_departure_published"]
        self.browser.call(source, "static_call", "start", "{}", ok=False)
        self.browser.call(target, "enable_files")
        self.saved_mixed(peer, target, self.send_mixed(peer, target, path), retained)

        # Retained Browser restrictions map to the effective Native allow-list.
        portable = self.browser.call(target, "prepare_node_replacement")
        portable_node = json.loads(portable)["node"]
        assert disabled in portable_node["settings"]["browser"]["disabled"]
        assert portable_node["admission"]["codes"][0]["used"] == 1
        assert portable_node["admission"]["codes"][0]["stopped"]
        capsule = Path(self.path("browser-to-native.node-backup", False))
        capsule.write_text(portable); capsule.chmod(0o600)
        native = Host(self, "browser-backup-native-target", False, "restore", self.path("browser-native-state", False), str(capsule))
        assert native.node == status["id"]
        native_complete = native.call("replacement_complete", address=peer.address, receipt_path=self.path("browser-native-receipt.json", False))
        self.browser.call(target, "static_call", "finalize_node_replacement", json.dumps(native_complete["receipt"]))
        assert native.call("requests")["requests"] == [dict(applicant=pending.node, request_id=pending_request["request"])]
        native.call("approve", applicant=rejected.node, request_id=rejected_request["request"], ok=False)
        records = native.call("status")["records"]
        assert approved.call("request", path=approved_invite)["stage"] == "Complete"
        assert native.call("status")["records"] == records
        native.call("approve", applicant=pending.node, request_id=pending_request["request"])
        assert pending.call("request", path=pending_invite)["stage"] == "Complete"
        assert consumed.call("request", path=consumed_invite)["stage"] == "Complete"
        actual_config = json.loads(Path(native.directory, "resources.json").read_text())
        self.record(native.label, "restored_effective_resource_policy", dict(
            disabledOrigin=disabled, disabledUses=[usage for origin, usage in actual_config["disabled"] if origin == disabled],
            selectedDataRelays=[item["url"] for item in actual_config["replacement"] if "DataRelay" in item["uses"]],
            sourceDataRelays=portable_node["settings"]["contact"]["relays"]))
        assert actual_config["path_selection"] == portable_node["settings"]["path_selection"]
        assert actual_config["maintenance"] == portable_node["settings"]["maintenance"]
        assert {item["url"] for item in actual_config["replacement"] if "DataRelay" in item["uses"]} == set(portable_node["settings"]["contact"]["relays"])
        assert all([disabled, usage] in actual_config["disabled"] for usage in ["Observation", "Lookup", "Signaling", "DataRelay"])
        # Allow real wall time to expire the original signed guest deadline.
        # This waits for the business condition; it never changes product time.
        while int(time.time()) <= expiry_scope["expires"]:
            time.sleep(0.1)
        expired = self.restore_browser("guest-expired-old-snapshot", expiring_backup)
        self.browser.call(expired, "complete_node_replacement", peer.node, ok=False)
        assert not self.browser.call(expired, "node_replacement_status")["ready"]
        self.browser.call(expired, "enable_files", ok=False)
        self.close_actor(expired)
        ordinary = self.browser_member(native, "browser-ordinary-departure")
        self.sync(native, peer)
        name, expected = self.make_browser(ordinary, "before-voluntary-exit.bin", 11)
        self.saved_mixed(ordinary, peer, self.send_mixed(ordinary, peer, name), expected, directory)
        leave = self.browser.call(ordinary, "propose_device_operation", "leave", self.identity(ordinary))["operation"]["id"]
        ended = self.browser.call(ordinary, "execute_device_operation", leave)
        assert not ended["requires_genesis"] and self.browser.call(ordinary, "status")["stage"] == "Removed"
        assert self.browser.call(ordinary, "execute_device_operation", leave)["record"] == ended["record"]
        self.browser.call(ordinary, "file_send_source", peer.node, name, ok=False)
        self.browser.call(ordinary, "enable_files", ok=False)
        native_leave = pending.call("genesis_propose", action="leave", target=pending.node)["operation"]["id"]
        native_ended = pending.call("device_execute", operation=native_leave)
        assert not native_ended["requires_genesis"] and pending.call("status")["stage"] == "Removed"
        assert pending.call("device_execute", operation=native_leave)["record"] == native_ended["record"]
        pending.call("files_enable", directory=self.path("departed-denied", False), ok=False)
        self.record(source, "browser_source_ownership_and_resource_continuity", dict(identity=status["id"],
            realActiveBytesBeforeStop=progress["bytes"], activeTaskStoppedWithoutReceipt=True,
            noPartialFile=True, independentBusinessFilesNotClaimedCopied=True,
            incorrectReceiptRetainedSource=True, abortRequiredExplicitNoTarget=True,
            browserToBrowserSameIdentity=True, signedFinalizationIdempotent=True,
            browserToNativeEffectiveRestrictions=True, expiredOldGuestCapsuleDenied=True,
            pendingAttemptRetainedAcrossBothRuntimes=True, originalApprovedGrant=original_grant,
            originalGrantCompletedWithoutNewRecord=True, rejectionCannotBecomeApproval=True,
            precodeUsedOnceAndStopped=True,
            ordinaryBrowserDeparture=ended["record"], ordinaryNativeDeparture=native_ended["record"],
            departureRetryIdempotentAndBusinessDenied=True,
            originalGuestDeadline=expiry_scope["expires"], actualOPFSSave=retained))
        for host in (native, peer, pending, approved, rejected, consumed, old_new):
            self.close_actor(host)
        self.close_actor(ordinary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    args.harness = Path(__file__).resolve().parents[1] / "browser-probe/harness/device-management.mjs"
    build = json.loads(args.build_receipt.read_text())
    actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in args.package.rglob("*") if path.is_file()}
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"] and actual == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only, args.focus, args.case = True, False, "smoke", "portable"
    BrowserBackupRunner(args).run()


if __name__ == "__main__":
    main()
