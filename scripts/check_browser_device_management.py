#!/usr/bin/env python3
"""Ordinary public Native/WASM device operations in independent Chromium nodes.

No injected storage faults, prototype changes or clock changes. Private cards
and complete-node capsules stay in the owned runtime directory or RPC pipe.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess

from check_browser_onboarding import BrowserControl, PRIVATE_OPERATIONS, CONFIGURATION
from check_browser_services import BrowserServiceRunner
from check_native_client import Host


class BrowserDeviceRunner(BrowserServiceRunner):
    def device(self, host, browser_method, native_method, *values, ok=True, **arguments):
        if isinstance(host, str):
            return self.browser.call(host, browser_method, *values, ok=ok)
        return host.call(native_method, ok=ok, **arguments)

    def custody(self, host):
        return self.device(host, "credential_custody", "custody_status")

    def sync(self, host, peer):
        return self.device(host, "sync_services", "service_sync", self.identity(peer), address=self.address_of(peer))

    def close_actor(self, host):
        return self.browser.call(host, "close_node") if isinstance(host, str) else host.stop()

    def custody_lifecycle(self, browser_creator):
        """Ordinary stop/reopen/fresh-consent flow; never redeliver old material."""
        prefix = "browser-source-lifecycle" if browser_creator else "native-source-lifecycle"
        source = self.browser_creator(prefix) if browser_creator else self.native_creator(prefix)
        recipient = self.native_member(source, prefix + "-recipient") if browser_creator else self.browser_member(source, prefix + "-recipient")
        identity = self.identity(recipient)

        def entry(identifier):
            return next(e for e in self.custody(recipient)["offers"] if e["offer"]["id"] == identifier)

        def propose():
            proposed = self.device(source, "propose_credential_custody", "custody_propose", identity, recipient=identity)
            identifier = proposed["offer"]["id"]
            delivered = self.device(source, "transfer_credential_custody", "custody_transfer", identifier, identity,
                offer=identifier, address=self.address_of(recipient))
            assert delivered["stage"] == "awaiting_recipient_confirmation"
            self.device(recipient, "accept_credential_custody", "custody_accept", identifier, offer=identifier)
            return identifier

        def stop():
            checked = self.device(recipient, "check_other_credential_custodians", "custody_check",
                addresses=[self.address_of(source)])
            assert checked["confirmed_other_member"]
            stopped = self.device(recipient, "stop_credential_custody", "custody_stop", checked["check"], False,
                check=checked["check"])
            assert stopped["deleted_locally"] and not stopped["forced_without_confirmation"]
            assert not self.custody(recipient)["holds_genesis"]

        def reopen():
            nonlocal recipient
            if isinstance(recipient, str):
                assert self.browser.reopen(recipient)["id"] == identity
            else:
                recipient = self.reopen(recipient)
            assert not self.custody(recipient)["holds_genesis"]

        # Stop before the first credential save: the outstanding local consent
        # must not be presented as ready after a normal storage reopen.
        pending = propose()
        assert not self.custody(recipient)["holds_genesis"]
        stop()
        reopen()
        assert entry(pending)["stage"] == "stopped_locally" and not entry(pending)["recipient_confirmed"]
        assert entry(pending)["saved_receipt"] is None
        self.device(recipient, "accept_credential_custody", "custody_accept", pending, offer=pending, ok=False)

        # A different source-confirmed offer, explicitly accepted by the
        # recipient, still performs an ordinary first delivery and durable save.
        fresh = propose()
        assert fresh != pending
        saved = self.device(source, "transfer_credential_custody", "custody_transfer", fresh, identity,
            offer=fresh, address=self.address_of(recipient))
        assert saved["stage"] == "saved" and saved["saved_receipt"]
        assert self.custody(source)["holds_genesis"] and self.custody(recipient)["holds_genesis"]
        stop()
        reopen()
        historical = entry(fresh)
        assert historical["stage"] == "stopped_locally" and historical["saved_receipt"] == saved["saved_receipt"]
        self.device(recipient, "accept_credential_custody", "custody_accept", fresh, offer=fresh, ok=False)
        # No transfer of either old offer is attempted after stopping custody.
        final = propose()
        assert final not in (pending, fresh)
        saved_again = self.device(source, "transfer_credential_custody", "custody_transfer", final, identity,
            offer=final, address=self.address_of(recipient))
        assert saved_again["stage"] == "saved" and self.custody(recipient)["holds_genesis"]
        stop()
        reopen()

        # Ordinary self-departure has an existing signed result. Confirmation
        # queries must return that same result in both adapters without a key.
        leave = self.device(recipient, "propose_device_operation", "genesis_propose", "leave", identity,
            action="leave", target=identity)["operation"]["id"]
        ended = self.device(recipient, "execute_device_operation", "device_execute", leave, operation=leave)
        assert not ended["requires_genesis"]
        before = self.device(recipient, "status", "status")
        assert before["stage"] == "Removed"
        queried = self.device(recipient, "confirm_device_operation", "genesis_confirm", leave, operation=leave)
        assert queried["record"] == ended["record"]
        assert self.device(recipient, "status", "status") == before
        self.record(prefix, "ordinary_custody_lifecycle", dict(recipientRuntime="native" if browser_creator else "browser",
            pendingConsentStopped=True, stopRetainedOnReopen=True, stoppedConsentCannotBeReaccepted=True,
            historicalReceiptRetained=True, freshBothEndConsentSavedTwice=True, oldCredentialRedeliveryAttempted=False,
            ordinaryLeaveConfirmationRecord=queried["record"], confirmationQueryDidNotChangeStatus=True))
        self.close_actor(recipient)
        self.close_actor(source)

    def custody_recovery(self, browser_creator):
        prefix = "wasm-root" if browser_creator else "native-root-custody"
        root = self.browser_creator(prefix) if browser_creator else self.native_creator(prefix)
        member = self.native_member(root, prefix + "-ordinary") if browser_creator else self.browser_member(root, prefix + "-ordinary")
        if browser_creator:
            card = self.browser.call(root, "export_recovery_card")
        else:
            card = Path(self.path(prefix + "-card.json", False)).read_text()
        identity = self.identity(member)
        offered = self.device(root, "propose_credential_custody", "custody_propose", identity, recipient=identity)
        offer = offered["offer"]["id"]
        self.device(root, "accept_credential_custody", "custody_accept", offer, offer=offer, ok=False)
        def transfer():
            return self.device(root, "transfer_credential_custody", "custody_transfer", offer, identity, offer=offer, address=self.address_of(member))
        assert transfer()["stage"] == "awaiting_recipient_confirmation"
        assert not self.custody(member)["holds_genesis"]
        self.device(member, "accept_credential_custody", "custody_accept", offer, offer=offer)
        saved = transfer()
        assert saved["stage"] == "saved" and saved["saved_receipt"]
        assert self.custody(member)["holds_genesis"] and self.custody(root)["holds_genesis"]
        if isinstance(member, str):
            assert self.browser.reopen(member)["id"] == identity
        else:
            member = self.reopen(member)
        assert self.custody(member)["holds_genesis"]
        assert transfer()["saved_receipt"] == saved["saved_receipt"]
        check = self.device(root, "check_other_credential_custodians", "custody_check", addresses=[self.address_of(member)])
        assert check["confirmed_other_member"]
        removed = self.device(root, "stop_credential_custody", "custody_stop", check["check"], False, check=check["check"])
        assert removed["deleted_locally"] and not removed["forced_without_confirmation"]
        check = self.device(member, "check_other_credential_custodians", "custody_check", addresses=[self.address_of(root)])
        assert not check["confirmed_other_member"]
        self.device(member, "stop_credential_custody", "custody_stop", check["check"], False, check=check["check"], ok=False)
        forced = self.device(member, "stop_credential_custody", "custody_stop", check["check"], True, check=check["check"], force_without_confirmed_copy=True)
        assert forced["deleted_locally"] and forced["forced_without_confirmation"]
        assert not self.custody(member)["holds_genesis"]
        self.close_actor(root)
        for as_browser in (not browser_creator, browser_creator):
            name = prefix + ("-recovered-wasm" if as_browser else "-recovered-native")
            if as_browser:
                self.browser.call(name, "open")
                status = self.browser.call(name, "start_recovery", json.dumps(CONFIGURATION), card, None, identity)
                self.browser_nodes[name] = status["id"]
                recovered = name
            else:
                card_path = Path(self.path(name + "-card.json", False)); card_path.write_text(card); card_path.chmod(0o600)
                address_path = Path(self.path(name + "-contact.json", False)); address_path.write_text(json.dumps(self.address_of(member)))
                recovered = Host(self, name, False, "recover", self.path(name + "-state", False), str(card_path), str(address_path))
            if as_browser:
                self.browser.call(recovered, "enable_files", ok=False)
                self.browser.call(recovered, "propose_device_operation", "appoint-administrator", identity, ok=False)
            else:
                recovered.call("files_enable", directory=self.path(name + "-premature", False), ok=False)
            if as_browser:
                self.browser.call(recovered, "restore_management_authority", self.identity(root), ok=False)
            result = self.device(recovered, "restore_management_authority", "management_recover", identity, address=self.address_of(member))
            assert result["stage"] == "Complete" and result["saved_locally"]
            assert not result["old_administrators_automatically_revoked"]
            repeated = self.device(recovered, "restore_management_authority", "management_recover", identity, address=self.address_of(member))
            assert repeated["authorization"] == result["authorization"]
            self.sync(member, recovered)
            assert self.identity(recovered) not in (self.identity(root), identity)
            operation = self.device(recovered, "propose_device_operation", "genesis_propose", "demote-administrator", self.identity(recovered), action="demote-administrator", target=self.identity(recovered))["operation"]["id"]
            demoted = self.device(recovered, "confirm_device_operation", "genesis_confirm", operation, operation=operation)
            assert demoted["stage"] == "confirmed"
            self.sync(recovered, member)
            # The TLS/root proof session stays alive through initial secret save,
            # but its original grant cannot re-promote a subsequently demoted node.
            self.device(recovered, "restore_management_authority", "management_recover", identity, address=self.address_of(member), ok=False)
            if as_browser:
                self.browser.call(recovered, "enable_files")
                if isinstance(member, str):
                    source, expected = self.make_browser(member, name + "-real.bin", 11)
                else:
                    source, expected = self.generate(member, name + "-real.bin", 11)
                self.saved_mixed(member, recovered, self.send_mixed(member, recovered, source), expected)
            else:
                directory = self.enable(recovered, name + "-files")
                if isinstance(member, str):
                    source, expected = self.make_browser(member, name + "-real.bin", 11)
                else:
                    source, expected = self.generate(member, name + "-real.bin", 11)
                self.saved_mixed(member, recovered, self.send_mixed(member, recovered, source), expected, directory)
            self.record(prefix, "real_cross_runtime_custody_and_recovery", dict(savedReceipt=saved["saved_receipt"],
                recipientReopened=True, normalThenExplicitRiskDeletion=True, oldAdministratorStopped=True,
                newRecoveryIdentity=self.identity(recovered), contactRuntime="wasm" if isinstance(member, str) else "native",
                recoveredRuntime="wasm" if as_browser else "native", exactGrantRetried=result["authorization"],
                beforeProofBusinessDenied=True, oldGrantAfterDemotionDenied=True, actualSavedHash=expected))
            self.close_actor(recovered)
        self.close_actor(member)

    def portable(self):
        source = self.native_creator("portable-native")
        peer = self.browser_member(source, "portable-peer")
        identity = source.node
        rule = dict(members=False, guests=False, allowed=[self.identity(peer)], denied=[])
        revision = source.call("service_rule", provider=identity, service="files", previous=None, rule=rule)["revision"]
        self.sync(source, peer)
        # Retain an unfinished exact confirmation and a two-end custody offer.
        pending = source.call("genesis_propose", action="appoint-administrator", target=self.identity(peer))["operation"]["id"]
        custody = source.call("custody_propose", recipient=self.identity(peer))["offer"]["id"]
        source.call("custody_transfer", offer=custody, address=self.address_of(peer))
        self.browser.call(peer, "accept_credential_custody", custody)
        name = "portable.node-backup"
        prepared = source.call("replacement_prepare", path=self.path(name, False))
        source.process.stdin.close(); assert source.process.wait(timeout=20) == 0; source.stderr.close()
        backup = Path(self.path(name, False)).read_text()
        target = "portable-browser"
        self.browser.call(target, "open")
        status = self.browser.call(target, "start_replacement", "{}", backup)
        self.browser_nodes[target] = status["id"]
        assert status["id"] == identity and status["stage"] == "RestoringAuthorization"
        self.browser.call(target, "enable_files", ok=False)
        self.browser.call(target, "set_service_rule", identity, "files", revision, json.dumps(rule), ok=False)
        complete = self.browser.call(target, "complete_node_replacement", self.identity(peer))
        assert complete["ready"] and complete["receipt"]["package"] == prepared["package"]
        assert self.browser.call(target, "credential_custody")["holds_genesis"]
        assert self.browser.call(target, "service_status")["services"][0]["policy"]["revision"] == revision
        assert any(o["operation"]["id"] == pending for o in self.browser.call(target, "device_operations")["operations"])
        assert self.browser.call(target, "transfer_credential_custody", custody, self.identity(peer))["stage"] == "saved"
        confirmed = self.browser.call(target, "confirm_device_operation", pending)
        assert confirmed["stage"] == "confirmed"
        self.sync(target, peer)
        self.browser.call(target, "enable_files")
        path, expected = self.make_browser(peer, "portable-opfs.bin", 11)
        self.saved_mixed(peer, target, self.send_mixed(peer, target, path), expected)
        # The next capsule is produced by the real Browser lease/CAS owner.
        second_backup = self.browser.call(target, "prepare_node_replacement")
        self.browser.call(target, "static_call", "start", "{}", ok=False)
        second_path = Path(self.path("portable-browser.node-backup", False)); second_path.write_text(second_backup); second_path.chmod(0o600)
        restored = Host(self, "portable-native-return", False, "restore", self.path("portable-return-state", False), str(second_path))
        assert restored.node == identity and restored.ready["status"]["stage"] == "RestoringAuthorization"
        restored.call("files_enable", ok=False, directory=self.path("portable-premature", False))
        receipt_path = self.path("portable-browser-receipt.json", False)
        returned = restored.call("replacement_complete", address=self.address_of(peer), receipt_path=receipt_path)
        assert returned["ready"] and restored.call("custody_status")["holds_genesis"]
        assert restored.call("device_operations")["operations"][0]["record"] == confirmed["record"]
        assert restored.call("service_status")["services"][0]["policy"]["revision"] == revision
        self.browser.call(target, "static_call", "finalize_node_replacement", json.dumps(returned["receipt"]))
        self.browser.call(target, "static_call", "start", "{}", ok=False)
        directory = self.enable(restored, "portable-return-business")
        self.saved_mixed(peer, restored, self.send_mixed(peer, restored, path), expected, directory)
        # Current peer changes defeat an old capsule before it can serve.
        operation = self.browser.call(peer, "propose_device_operation", "demote-administrator", identity)["operation"]["id"]
        self.browser.call(peer, "request_device_confirmation", operation, identity)
        restored.call("genesis_confirm", operation=operation)
        self.browser.call(peer, "collect_device_result", operation, identity)
        restored.stop()
        stale = "portable-old-snapshot"
        self.browser.call(stale, "open")
        status = self.browser.call(stale, "start_replacement", "{}", backup); self.browser_nodes[stale] = status["id"]
        self.browser.call(stale, "complete_node_replacement", self.identity(peer))
        self.browser.call(stale, "set_service_rule", self.identity(peer), "files", None, json.dumps(rule), ok=False)
        self.browser.call(stale, "close_node")
        remove = self.browser.call(peer, "propose_device_operation", "remove-member", identity)["operation"]["id"]
        assert not self.browser.call(peer, "execute_device_operation", remove)["requires_genesis"]
        removed = "portable-removed-snapshot"; self.browser.call(removed, "open")
        status = self.browser.call(removed, "start_replacement", "{}", backup); self.browser_nodes[removed] = status["id"]
        self.browser.call(removed, "complete_node_replacement", self.identity(peer), ok=False)
        self.browser.call(removed, "enable_files", ok=False)
        assert self.browser.call(removed, "node_replacement_status")["ready"] is False
        self.browser.call(removed, "export_recovery_card")
        self.record(target, "native_browser_native_portable", dict(identity=identity, effectiveRule=revision,
            unfinishedConfirmationRetained=True, twoEndCustodyRetained=True, exactResultRetained=confirmed["record"],
            sourcePaused=True, targetQuarantine=True, browserSourceRetired=True, actualSavedHash=expected,
            oldDemotionAdopted=True, oldRemovalBlocked=True, physicalRootStillExportable=True))
        self.close_actor(removed); self.close_actor(peer)

    def recovery_portable(self):
        root = self.native_creator("recovery-portable-root")
        contact = self.browser_member(root, "recovery-portable-contact")
        card = Path(self.path("recovery-portable-root-card.json", False)).read_text()
        root.stop()
        source = "recovery-portable-wasm-source"
        self.browser.call(source, "open")
        status = self.browser.call(source, "start_recovery", json.dumps(CONFIGURATION), card, None, self.identity(contact))
        self.browser_nodes[source] = status["id"]
        first = self.browser.call(source, "restore_management_authority", self.identity(contact))
        assert first["stage"] == "Complete"
        identity, grant = status["id"], first["authorization"]
        first_record_ids = json.loads(self.browser.call(source, "admission_material_receipt"))["records"]
        first_records = len(first_record_ids)
        backup = self.browser.call(source, "prepare_node_replacement")
        original_signed_records = json.loads(backup)["node"]["records"]
        path = Path(self.path("recovery-portable-first.node-backup", False)); path.write_text(backup); path.chmod(0o600)
        target = Host(self, "recovery-portable-native-target", False, "restore", self.path("recovery-portable-native-state", False), str(path))
        assert target.node == identity
        assert target.ready["status"]["stage"] == "RestoringAuthorization"
        assert target.call("management_recovery_status")["authorization"] == grant
        target.call("management_recover", address=self.address_of(contact), ok=False)
        receipt_path = self.path("recovery-portable-first-receipt.json", False)
        qualified = target.call("replacement_complete", address=self.address_of(contact), receipt_path=receipt_path)
        assert qualified["ready"]
        assert target.call("management_recovery_status")["stage"] == "Complete"
        repeated = target.call("management_recover", address=self.address_of(contact))
        assert repeated["stage"] == "Complete" and repeated["authorization"] == grant
        assert target.call("status")["records"] == first_records
        self.browser.call(source, "static_call", "finalize_node_replacement", json.dumps(qualified["receipt"]))
        second_path = self.path("recovery-portable-second.node-backup", False)
        target.call("replacement_prepare", path=second_path)
        target.process.stdin.close(); assert target.process.wait(timeout=20) == 0; target.stderr.close()
        second = Path(second_path).read_text()
        assert json.loads(second)["node"]["records"] == original_signed_records
        final = "recovery-portable-wasm-final"
        self.browser.call(final, "open")
        restored = self.browser.call(final, "start_replacement", "{}", second)
        self.browser_nodes[final] = restored["id"]
        assert restored["id"] == identity and restored["stage"] == "RestoringAuthorization"
        self.browser.call(final, "enable_files", ok=False)
        assert self.browser.call(final, "management_recovery_status")["authorization"] == grant
        self.browser.call(final, "restore_management_authority", self.identity(contact), ok=False)
        completed = self.browser.call(final, "complete_node_replacement", self.identity(contact))
        assert completed["ready"]
        resumed = self.browser.call(final, "restore_management_authority", self.identity(contact))
        assert resumed["stage"] == "Complete" and resumed["authorization"] == grant
        assert json.loads(self.browser.call(final, "admission_material_receipt"))["records"] == first_record_ids
        receipt = Path(self.path("recovery-portable-second-receipt.json", False)); receipt.write_text(json.dumps(completed["receipt"]))
        retired = subprocess.run([str(self.linux), "finalize-replacement", target.directory, str(receipt)], capture_output=True, text=True, timeout=90)
        assert retired.returncode == 0 and json.loads(retired.stdout)["source_retired"]
        assert not json.loads(retired.stdout)["network_departure_published"]
        assert not Path(target.directory, "node.key").exists() and not Path(target.directory, "genesis.credential").exists()
        self.browser.call(contact, "enable_files")
        filename, expected = self.make_browser(final, "recovered-after-two-migrations.bin", 11)
        self.saved_mixed(final, contact, self.send_mixed(final, contact, filename), expected)
        self.record(final, "management_recovery_portable_original_result", dict(identity=identity,
            originalGrant=grant, records=first_records, browserNativeBrowser=True,
            exactAttemptAndCompletedProofRetained=True, freshTLSRetrySameGrant=True,
            sourceBrowserAndNativeRetired=True, actualOPFSSavedHash=expected))
        self.close_actor(final); self.close_actor(contact)

    def role(self, host, identity):
        if isinstance(host, str):
            self.browser.call(host, "refresh_service_state")
            # Canonical service authorization and management operations verify
            # this state; status contains only public scope/identity fields.
            return self.browser.call(host, "status")
        return next((m["role"] for m in host.call("status")["members"] if m["node"] == identity), None)

    def confirmations(self):
        root = self.native_creator("native-root")
        browser = self.browser_member(root, "wasm-initiator")
        identity = self.identity(browser)
        assert not self.browser.call(browser, "device_operations")["holds_genesis"]
        proposed = self.browser.call(browser, "propose_device_operation", "appoint-administrator", identity)
        operation = proposed["operation"]["id"]
        self.browser.call(browser, "execute_device_operation", operation, ok=False)
        self.browser.call(browser, "confirm_device_operation", operation, ok=False)
        for _ in range(2):
            assert self.browser.call(browser, "request_device_confirmation", operation, root.node)["stage"] == "awaiting_confirmation"
        requests = root.call("device_operations")["operations"]
        assert len(requests) == 1 and requests[0]["operation"]["id"] == operation
        confirmed = root.call("genesis_confirm", operation=operation)
        result = self.browser.call(browser, "collect_device_result", operation, root.node)
        assert result["record"] == confirmed["record"] and result["effective_locally"]
        assert not self.browser.call(browser, "device_operations")["holds_genesis"]
        count = json.loads(self.browser.call(browser, "admission_material_receipt"))["records"]
        assert self.browser.call(browser, "collect_device_result", operation, root.node)["record"] == result["record"]
        assert json.loads(self.browser.call(browser, "admission_material_receipt"))["records"] == count
        self.browser.call(browser, "set_service_rule", root.node, "files", None,
            json.dumps(dict(members=True, guests=True, allowed=[], denied=[])))
        self.browser.call(browser, "sync_services", root.node)
        demotion = self.browser.call(browser, "propose_device_operation", "demote-administrator", identity)["operation"]["id"]
        self.browser.call(browser, "execute_device_operation", demotion, ok=False)
        self.browser.call(browser, "confirm_device_operation", demotion, ok=False)
        card = Path(self.path("native-root-card.json", False)).read_text()
        demoted = self.browser.call(browser, "confirm_device_operation_offline", demotion, card, None)
        assert demoted["stage"] == "confirmed" and demoted["effective_locally"]
        assert not self.browser.call(browser, "device_operations")["holds_genesis"]
        self.browser.call(browser, "sync_services", root.node)
        assert self.role(root, identity) == "member"
        session = self.browser.call(browser, "begin_management_session")
        seed = base64.urlsafe_b64decode(card.split(":1:", 1)[1] + "==")[:32].hex()
        self.browser.call(browser, "elevate_genesis", session, seed, root.ready["status"]["genesis"], 10000, ok=False)
        self.browser.call(browser, "end_management_session", session)
        directory = self.enable(root, "wasm-to-native")
        source, expected = self.make_browser(browser, "post-demotion.bin", 11)
        self.saved_mixed(browser, root, self.send_mixed(browser, root, source), expected, directory)
        self.browser.call(browser, "enable_files")
        source, expected = self.generate(root, "native-to-wasm.bin", 11)
        self.saved_mixed(root, browser, self.send_mixed(root, browser, source), expected)
        self.record(browser, "ordinary_browser_specific_genesis", dict(remoteRecord=result["record"],
            offlineDemotionRecord=demoted["record"], noRootCopied=True, sameResultRetried=True,
            generalWindowRejected=True, actualFilesInBothDirections=True))
        self.browser.call(browser, "close_node")
        root.stop()

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.browser = BrowserControl(self, self.args)
            self.browser.private_operations = PRIVATE_OPERATIONS | {
                "issue_precode", "prepare_node_replacement", "retry_node_replacement_backup",
                "prepare_management_recovery", "restore_node_replacement", "static_call",
                "start_recovery", "start_replacement"}
            if self.args.case in ("all", "confirmations"):
                self.confirmations()
            if self.args.case in ("all", "custody"):
                self.custody_recovery(False)
                self.custody_recovery(True)
            if self.args.case in ("all", "portable"):
                self.portable()
            if self.args.case in ("all", "recovery-portable"):
                self.recovery_portable()
            if self.args.case in ("all", "custody-lifecycle"):
                self.custody_lifecycle(False)
                self.custody_lifecycle(True)
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
                qualifications=self.qualifications, browserCleanup=browser_cleanup, cleanup=cleanup,
                controllerSourceDrift=drift, wasmPackage=self.args.package_sha,
                noPrototypeHooks=True, noClockChange=True, ssh_business_transfer=False), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["all", "confirmations", "custody", "portable", "recovery-portable", "custody-lifecycle"], default="all")
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    args.harness = root / "browser-probe/harness/device-management.mjs"
    build = json.loads(args.build_receipt.read_text())
    actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in args.package.rglob("*") if path.is_file()}
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"] and actual == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only, args.focus = True, False, "smoke"
    BrowserDeviceRunner(args).run()


if __name__ == "__main__":
    main()
