#!/usr/bin/env python3
"""Ordinary public admission/result continuity through complete-node replacement.

Real Native hosts retain pending, approved, rejected and precode attempts across
the same identity's replacement. Private capsules are inspected only inside the
owned runtime directory; evidence contains counts and original public grants.
"""
import argparse
import json
from pathlib import Path
import subprocess

from check_node_replacement import ReplacementRunner


class ConsistencyRunner(ReplacementRunner):
    def refresh_contact_hints(self, target, names, source_remote, applicant_remote):
        """Keep original attempts/codes while explicitly locating the moved node.

        Direct-IP runs disable discovery. Addresses are contact hints, whereas
        the original invitation material and bearer code must remain unchanged.
        """
        for name in names:
            local = Path(self.path(name, False))
            if source_remote:
                subprocess.run(["scp", "-q", "laptop:" + self.path(name, True), str(local)], check=True)
            old = json.loads(local.read_text())
            assert old["address"]["id"] == target.node
            updated = dict(old, address=target.address)
            assert {k: v for k, v in updated.items() if k != "address"} == {
                k: v for k, v in old.items() if k != "address"}
            local.write_text(json.dumps(updated))
            local.chmod(0o600)
            if source_remote or applicant_remote:
                subprocess.run(["scp", "-q", str(local), "laptop:" + self.path(name, True)], check=True)
        self.record(target.label, "replacement_contact_hints_refreshed", dict(
            sameIdentity=True, originalAuthorizationAndCodesUnchanged=True,
            automaticDiscoveryClaimed=False))

    def applicant(self, source, prefix, invitation_name):
        invitation = self.transfer(invitation_name, source.remote, not source.remote if not self.local_only else False)
        host = self.start(prefix, not source.remote if not self.local_only else False, "join", invitation)
        return host, invitation

    def scenario(self, prefix, creator_remote=False):
        source, peer = self.paired(prefix, creator_remote)
        manual_name = prefix + "-manual-invite.json"
        source.call("invite", path=self.path(manual_name, source.remote))
        pending, pending_invite = self.applicant(source, prefix + "-pending", manual_name)
        assert pending.call("request", path=pending_invite)["stage"] == "AwaitingApproval"
        pending_request = next(r for r in source.call("requests")["requests"] if r["applicant"] == pending.node)
        approved, approved_invite = self.applicant(source, prefix + "-approved", manual_name)
        assert approved.call("request", path=approved_invite)["stage"] == "AwaitingApproval"
        approved_request = next(r for r in source.call("requests")["requests"] if r["applicant"] == approved.node)
        source.call("approve", **approved_request)
        assert approved.call("request", path=approved_invite, authorization_only=True)["stage"] == "PendingMaterials"
        rejected, rejected_invite = self.applicant(source, prefix + "-rejected", manual_name)
        assert rejected.call("request", path=rejected_invite)["stage"] == "AwaitingApproval"
        rejected_request = next(r for r in source.call("requests")["requests"] if r["applicant"] == rejected.node)
        source.call("reject", **rejected_request)
        assert rejected.call("request", path=rejected_invite)["stage"] == "Rejected"
        code_name = prefix + "-old-code-invite.json"
        source.call("issue_precode", path=self.path(code_name, source.remote), seconds=3600, max_uses=3)
        consumed, consumed_invite = self.applicant(source, prefix + "-consumed", code_name)
        assert consumed.call("request", path=consumed_invite)["stage"] == "Complete"
        assert source.call("precode_status", path=self.path(code_name, source.remote))["uses"] == 1
        source.call("service_sync", address=peer.address)
        backup_name = prefix + ".node-backup"
        package = source.call("replacement_prepare", path=self.path(backup_name, source.remote))["package"]
        source.process.stdin.close(); assert source.process.wait(timeout=20) == 0; source.stderr.close()
        capsule = self.control_file(backup_name, source.remote, False)
        before = json.loads(Path(capsule).read_text())["node"]["admission"]
        def row(ledger, applicant):
            return next(a for a in ledger["approvals"] if a["applicant"] == applicant.node)
        assert row(before, pending)["authorization"] is None
        original = row(before, approved)["authorization"]
        assert original is not None and row(before, approved)["delivery_phase"] == 1
        assert row(before, rejected)["rejected"]
        assert before["codes"][0]["used"] == 1
        assert before["own_request"] is None  # creator never requested admission
        snapshot = self.control_file(backup_name, False, peer.remote)
        target = self.restore(prefix + "-target", peer.remote, snapshot)
        assert target.node == source.node
        target.call("ping", ok=False, address=peer.address)
        receipt_name = prefix + "-receipt.json"
        target.call("replacement_complete", address=peer.address, receipt_path=self.path(receipt_name, target.remote))
        assert target.call("requests")["requests"] == [pending_request]
        target.call("approve", ok=False, **rejected_request)
        self.refresh_contact_hints(target, [manual_name, code_name], source.remote, approved.remote)
        records = target.call("status")["records"]
        assert approved.call("request", path=approved_invite)["stage"] == "Complete"
        assert target.call("status")["records"] == records
        assert consumed.call("request", path=consumed_invite)["stage"] == "Complete"
        assert target.call("status")["records"] == records
        assert pending.call("request", path=pending_invite)["stage"] == "AwaitingApproval"
        target.call("approve", **pending_request)
        assert pending.call("request", path=pending_invite)["stage"] == "Complete"
        assert rejected.call("status")["stage"] == "Rejected"
        rejected_again = rejected.call("request", path=rejected_invite)
        assert rejected_again == {"stage": "Rejected", "automatic_retry": False}
        # An unconsumed allowance in an old backup is never resurrected.
        old_new, old_invite = self.applicant(source, prefix + "-new-old-code", code_name)
        old_new.call("request", path=old_invite, ok=False)
        assert target.call("requests")["requests"] == []
        fresh_name = prefix + "-fresh-code-invite.json"
        target.call("issue_precode", path=self.path(fresh_name, target.remote), seconds=3600, max_uses=1)
        fresh, fresh_invite = self.applicant(target, prefix + "-fresh", fresh_name)
        assert fresh.call("request", path=fresh_invite)["stage"] == "Complete"
        target.call("service_sync", address=fresh.address)
        directory = self.enable(target, prefix + "-actual-save")
        path, expected = self.generate(fresh, prefix + "-fresh.bin", 11)
        self.delivered(fresh, target, path, expected, directory)
        final_name = prefix + "-final.node-backup"
        target.call("replacement_prepare", path=self.path(final_name, target.remote))
        target.process.stdin.close(); assert target.process.wait(timeout=20) == 0; target.stderr.close()
        final = json.loads(Path(self.control_file(final_name, target.remote, False)).read_text())["node"]["admission"]
        assert row(final, approved)["authorization"] == original and row(final, approved)["complete"]
        assert row(final, pending)["complete"] and row(final, rejected)["rejected"]
        old = next(code for code in final["codes"] if code["token"] == before["codes"][0]["token"])
        assert old["used"] == 1 and old["stopped"]
        # Expected generations belong to the applicant that received material,
        # rather than to its creator/provider. Inspect that actual node too.
        peer_name = prefix + "-peer.node-backup"
        peer.call("replacement_prepare", path=self.path(peer_name, peer.remote))
        peer.process.stdin.close(); assert peer.process.wait(timeout=20) == 0; peer.stderr.close()
        peer_node = json.loads(Path(self.control_file(peer_name, peer.remote, False)).read_text())["node"]
        material_rows = peer_node["admission"]["material_generations"]
        assert len(material_rows) > 0
        assert any(row["request"] == peer_node["admission"]["own_request"]["request"]
            and row["generation"] == peer_node["generation"] for row in material_rows)
        self.record(prefix, "portable_admission_control_continuity", dict(package=package,
            pendingAttemptRetained=True, approvedGrant=original, originalGrantCompletedWithoutNewRecord=True,
            rejectionRetainedAndCannotApprove=True, consumedCount=1, restoredOldCodeStopped=True,
            originalConsumedResultRetry=True, freshCodeNewAdmission=True,
            materialGenerationRows=len(material_rows), actualSavedHash=expected))
        for host in (peer, pending, approved, rejected, consumed, old_new, fresh):
            host.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--local-only", action="store_true")
    args = parser.parse_args()
    args.focus, args.failure_only = "smoke", False
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("actual Windows pin/hash required")
    ConsistencyRunner(args).run()


if __name__ == "__main__":
    main()
