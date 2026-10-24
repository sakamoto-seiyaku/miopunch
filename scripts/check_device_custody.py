#!/usr/bin/env python3
"""Actual Native custody, recovery and administrator departure operations.

Both OS roles use the public JSON-lines host. Private cards use SSH delivery
only for the explicitly chosen offline recovery path; protocol bytes do not.
"""
import argparse
import base64
import json
from pathlib import Path

from check_device_management import DeviceRunner
from check_native_client import Host, quote


class CustodyRunner(DeviceRunner):
    def paired(self, prefix, creator_remote=False):
        creator = self.start(prefix + "-creator", creator_remote)
        creator.call("export_recovery_card", path=self.path(prefix + "-card.json", creator_remote))
        creator.call("acknowledge_recovery_saved")
        name = prefix + "-invite.json"
        creator.call("invite", path=self.path(name, creator_remote))
        remote = not creator_remote if not self.local_only else False
        invitation = self.transfer(name, creator_remote, remote)
        member = self.start(prefix + "-member", remote, "join", invitation)
        member.call("files_enable", ok=False, directory=self.path(prefix + "-premature-receive", remote))
        source, _ = self.generate(member, prefix + "-premature-source.bin", 11)
        member.call("file_send", ok=False, address=creator.address, path=source)
        assert member.call("files_status")["tasks"] == []
        self.remove(source, remote)
        self.record(prefix, "admission_prerequisites", dict(creatorStatus=creator.call("status"),
            creatorAddress=creator.call("address"), applicantStatus=member.call("status")))
        if creator.remote:
            snapshot = self.remote("$exe=[IO.Path]::GetFullPath(" + quote(self.remote_binary) + "); "
                + "$owned=@(Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -eq $exe}); "
                + "@{processes=$owned.Count; udp=@(Get-NetUDPEndpoint | Where-Object {$owned.ProcessId -contains $_.OwningProcess} | Select-Object LocalAddress,LocalPort)} | ConvertTo-Json -Depth 4 -Compress")
            self.record(prefix, "owned_windows_listener_snapshot", json.loads(snapshot))
        try:
            assert member.call("request", path=invitation)["stage"] == "AwaitingApproval"
        except Exception:
            self.record(prefix, "admission_failure_independent_state", dict(
                creatorStatus=creator.call("status"), creatorAddress=creator.call("address"),
                pending=creator.call("requests"), applicantStatus=member.call("status")))
            raise
        pending = creator.call("requests")["requests"]
        assert len(pending) == 1 and pending[0]["applicant"] == member.node
        creator.call("approve", **pending[0])
        assert member.call("request", path=invitation)["stage"] == "Complete"
        return creator, member

    def write_json(self, name, value, remote):
        path = self.path(name, remote)
        content = json.dumps(value).encode()
        if remote:
            self.remote("[IO.File]::WriteAllBytes(" + quote(path) + ",[Convert]::FromBase64String(" + quote(base64.b64encode(content).decode()) + "))")
        else:
            Path(path).write_bytes(content)
        return path

    def third_member(self, admin, prefix, remote):
        name = prefix + "-invite.json"
        admin.call("issue_precode", path=self.path(name, admin.remote), seconds=3600, max_uses=1)
        invitation = self.transfer(name, admin.remote, remote)
        host = self.start(prefix, remote, "join", invitation)
        assert host.call("request", path=invitation)["stage"] == "Complete"
        return host

    def scenario(self, prefix, creator_remote=False):
        source, recipient = self.paired(prefix, creator_remote)
        members = source.call("status")["members"]
        offered = source.call("custody_propose", recipient=recipient.node)
        offer = offered["offer"]["id"]
        assert offered["stage"] == "awaiting_recipient_confirmation"
        source.call("custody_accept", ok=False, offer=offer)
        pending = source.call("custody_transfer", offer=offer, address=recipient.address)
        assert pending["stage"] == "awaiting_recipient_confirmation"
        assert not recipient.call("custody_status")["holds_genesis"]
        assert recipient.call("custody_status")["offers"][0]["offer"]["id"] == offer
        assert recipient.call("status")["members"] == members
        accepted = recipient.call("custody_accept", offer=offer)
        assert accepted["stage"] == "ready_to_deliver"
        saved = source.call("custody_transfer", offer=offer, address=recipient.address)
        assert saved["stage"] == "saved" and saved["saved_receipt"]
        assert source.call("custody_status")["holds_genesis"]
        assert recipient.call("custody_status")["holds_genesis"]
        assert self.role(recipient, recipient.node) == "member"
        recipient = self.reopen(recipient)
        assert recipient.call("custody_status")["holds_genesis"]
        repeated = source.call("custody_transfer", offer=offer, address=recipient.address)
        assert repeated["saved_receipt"] == saved["saved_receipt"]
        check = source.call("custody_check", addresses=[recipient.address])
        assert check["confirmed_other_member"]
        assert check["observations"] == [dict(member=recipient.node, status="confirmed_holding")]
        before = source.call("status")
        stopped = source.call("custody_stop", check=check["check"])
        assert stopped["deleted_locally"] and stopped["confirmed_other_member"] and not stopped["forced_without_confirmation"]
        assert source.call("status") == before
        assert not source.call("custody_status")["holds_genesis"]
        source = self.reopen(source)
        self.exchange(source, recipient)
        # The last online holder cannot mistake a card or rootless member for
        # another member's saved copy. The explicit risk path remains usable.
        last_check = recipient.call("custody_check", addresses=[source.address])
        assert not last_check["confirmed_other_member"]
        assert last_check["observations"] == [dict(member=source.node, status="currently_not_stored")]
        recipient.call("custody_stop", ok=False, check=last_check["check"])
        assert recipient.call("custody_status")["holds_genesis"]
        forced = recipient.call("custody_stop", check=last_check["check"], force_without_confirmed_copy=True)
        assert forced["deleted_locally"] and forced["forced_without_confirmation"]
        assert not recipient.call("custody_status")["holds_genesis"]
        recipient = self.reopen(recipient)
        self.exchange(source, recipient)
        self.record(prefix, "custody_actual_saved_and_deleted", dict(offer=offer,
            sourceAndRecipientConfirmed=True, actualSavedReceipt=saved["saved_receipt"],
            noAdministratorAutomaticallyGranted=True, rootlessCreatorReopened=True,
            normalDeletionConfirmedCurrentMember=True, lastOnlineCopyRequiredExplicitForce=True))

        # A plain ordinary member remains available while all old admin hosts
        # are stopped. The original card recovers a new node, not their IDs.
        contact = self.third_member(source, prefix + "-ordinary-contact", recipient.remote)
        source.call("service_sync", address=recipient.address)
        contact.call("service_sync", address=source.address)
        source.stop()
        recipient.stop()
        remote = creator_remote
        card = self.path(prefix + "-card.json", remote)
        address_file = self.write_json(prefix + "-recovery-contact.json", contact.address, remote)
        recovered = Host(self, prefix + "-recovered", remote, "recover",
            self.path(prefix + "-recovered-state", remote), card, address_file)
        assert recovered.node not in (source.node, recipient.node, contact.node)
        assert recovered.ready["status"]["records"] == 0
        recovery = recovered.call("management_recover", address=contact.address)
        assert recovery["stage"] == "Complete" and recovery["saved_locally"]
        assert recovery["old_administrators_automatically_revoked"] is False
        assert self.role(recovered, recovered.node) == "admin"
        assert self.role(recovered, source.node) == "admin"
        count = recovered.call("status")["records"]
        resumed = recovered.call("management_recover", address=contact.address)
        assert resumed["authorization"] == recovery["authorization"]
        assert recovered.call("status")["records"] == count
        recovered = self.reopen(recovered)
        self.exchange(recovered, contact)
        directory = self.enable(contact, prefix + "-recovered-business")
        path, expected = self.generate(recovered, prefix + "-recovery-business.bin", 11)
        self.delivered(recovered, contact, path, expected, directory)

        # Promote the ordinary contact, then let it explicitly leave through a
        # remote custodian. Receipt collection must work after its revocation.
        operation = self.propose(contact, "appoint-administrator", contact.node)
        contact.call("genesis_request", operation=operation, address=recovered.address)
        recovered.call("genesis_confirm", operation=operation)
        contact.call("genesis_result", operation=operation, address=recovered.address)
        assert self.role(contact, contact.node) == "admin"
        leave = self.propose(contact, "leave", contact.node)
        contact.call("device_execute", ok=False, operation=leave)
        contact.call("genesis_request", operation=leave, address=recovered.address)
        confirmed = recovered.call("genesis_confirm", operation=leave)
        ended = contact.call("genesis_result", operation=leave, address=recovered.address)
        assert ended["record"] == confirmed["record"]
        assert contact.call("status")["stage"] == "Removed"
        count = contact.call("status")["records"]
        assert contact.call("genesis_result", operation=leave, address=recovered.address)["record"] == ended["record"]
        assert contact.call("status")["records"] == count
        contact.call("ping", ok=False, address=recovered.address)
        self.record(prefix, "recovery_and_remote_administrator_exit", dict(
            originalNetworkRecoveredWithoutOldAdmins=True, newIndependentIdentity=recovered.node,
            originalGrantRetried=recovery["authorization"], oldAdminsNotAutomaticallyRevoked=True,
            postRecoverySavedHash=expected, remoteAdministratorDepartureRecord=ended["record"],
            resultOnlyScopeWorksAfterDeparture=True, businessDeniedAfterDeparture=True))
        for host in (contact, recovered):
            host.stop()

    def run(self):
        if self.direction == "both":
            return super().run()
        passed = False
        try:
            self.deploy()
            self.scenario(self.direction + "-creates", self.direction == "windows")
            passed = True
        finally:
            cleanup = self.cleanup()
            drift = self.controller_drift()
            passed = passed and not drift and all("error" not in item for item in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed,
                direction=self.direction, qualifications=self.qualifications, cleanup=cleanup,
                controllerSourceDrift=drift, ssh_business_transfer=False,
                originalWholeRunReclassified=False), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--local-only", action="store_true")
    parser.add_argument("--direction", choices=["both", "linux", "windows"], default="both")
    args = parser.parse_args()
    args.focus, args.failure_only = "smoke", False
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("actual Windows pin and hash required")
    runner = CustodyRunner(args)
    runner.direction = args.direction
    runner.run()


if __name__ == "__main__":
    main()
