#!/usr/bin/env python3
"""Real Windows/Linux complete-node replacement through public host operations.

Only the private identity capsule and public cleanup receipt cross SSH as
control material. All membership checks, decisions and business use miopunch.
"""
import argparse
import json
from pathlib import Path
import subprocess

from check_device_management import DeviceRunner
from check_native_client import Host, ps, quote


class ReplacementRunner(DeviceRunner):
    def control_file(self, name, source_remote, destination_remote):
        assert name.endswith((".node-backup", "-receipt.json"))
        if source_remote != destination_remote:
            source = "laptop:" + self.path(name, True) if source_remote else self.path(name, False)
            destination = "laptop:" + self.path(name, True) if destination_remote else self.path(name, False)
            subprocess.run(["scp", "-q", source, destination], check=True)
            if not destination_remote:
                Path(destination).chmod(0o600)
        return self.path(name, destination_remote)

    def static(self, remote, *arguments, ok=True):
        if remote:
            command = ps("& " + quote(self.remote_binary) + " " + " ".join(map(quote, arguments)) + "; exit $LASTEXITCODE")
        else:
            command = [str(self.linux), *arguments]
        result = subprocess.run(command, text=True, capture_output=True, timeout=90)
        assert (result.returncode == 0) == ok, dict(exit=result.returncode)
        if ok:
            return json.loads(result.stdout)
        assert not result.stdout
        return dict(exit=result.returncode, noReady=True)

    def source_paused(self, source):
        value = self.static(source.remote, "open", source.directory, ok=False)
        self.record(source.label, "ordinary_source_open_blocked", value)

    def restore(self, prefix, remote, backup):
        host = Host(self, prefix, remote, "restore", self.path(prefix + "-state", remote), backup)
        assert host.ready["status"]["stage"] == "RestoringAuthorization"
        assert host.ready["status"]["replacement_ready"] is False
        return host

    def scenario(self, prefix, creator_remote=False):
        source, peer = self.paired(prefix, creator_remote)
        source_identity = source.node
        members = source.call("status")["members"]
        rule = dict(members=False, guests=False, allowed=[peer.node], denied=[])
        revision = source.call("service_rule", provider=source.node, service="files", previous=None, rule=rule)["revision"]
        source.call("service_sync", address=peer.address)
        card_name = prefix + "-card.json"
        source.call("issue_precode", path=self.path(prefix + "-old-code-invite.json", source.remote), seconds=3600, max_uses=2)
        name = prefix + ".node-backup"
        prepared = source.call("replacement_prepare", path=self.path(name, source.remote))
        assert prepared["saved"] and prepared["source_stopped"]
        source.process.stdin.close()
        assert source.process.wait(timeout=20) == 0
        source.stderr.close()
        self.source_paused(source)
        snapshot = self.control_file(name, source.remote, peer.remote)
        target = self.restore(prefix + "-target", peer.remote, snapshot)
        assert target.node == source_identity and target.ready["status"]["members"] == members
        assert target.ready["status"]["recovery_saved"]
        policy = target.call("service_status")["services"][0]["policy"]
        assert policy["revision"] == revision and policy["rule"] == rule
        target.call("ping", ok=False, address=peer.address)
        target.call("files_enable", ok=False, directory=self.path(prefix + "-premature-restore-files", target.remote))
        target.call("service_rule", ok=False, provider=target.node, service="files", previous=revision, rule=rule)
        receipt_name = prefix + "-receipt.json"
        completed = target.call("replacement_complete", address=peer.address, receipt_path=self.path(receipt_name, target.remote))
        assert completed["ready"] and completed["receipt"]["package"] == prepared["package"]
        assert target.call("status")["stage"] == "Complete"
        assert target.call("custody_status")["holds_genesis"]
        repeated = target.call("replacement_complete", address=peer.address, receipt_path=self.path(receipt_name, target.remote))
        assert repeated["receipt"] == completed["receipt"] and repeated["reused"]
        self.exchange(target, peer)
        directory = self.enable(target, prefix + "-after-replacement")
        path, expected = self.generate(peer, prefix + "-after-replacement.bin", 11)
        self.delivered(peer, target, path, expected, directory)
        receipt = self.control_file(receipt_name, target.remote, source.remote)
        finalized = self.static(source.remote, "finalize-replacement", source.directory, receipt)
        assert finalized["source_retired"] and finalized["network_departure_published"] is False
        again = self.static(source.remote, "finalize-replacement", source.directory, receipt)
        assert again["source_retired"] and again["already_finalized"]
        self.source_paused(source)
        remaining = self.listing(source.directory, source.remote)
        assert not set(remaining).intersection({"node.key", "genesis.credential", "admission.material", "admission.secret", "management.sqlite"})
        assert peer.call("status")["members"] == members
        self.record(prefix, "complete_node_replacement", dict(identity=source_identity,
            package=prepared["package"], effectiveRuleRevision=revision, sourceStoppedBeforeExport=True,
            restoredRequiredCurrentPeer=True, rootAndRecoveryAcknowledgementRetained=True,
            independentlySavedHash=expected, sourcePrivateMaterialRemoved=True, noLeaveOrRemove=True))

        # Current administration changes after the retained old backup. No old
        # snapshot may use its previous role before a fresh peer exchange.
        appoint = self.propose(target, "appoint-administrator", peer.node)
        target.call("genesis_confirm", operation=appoint)
        target.call("service_sync", address=peer.address)
        demote = self.propose(peer, "demote-administrator", target.node)
        peer.call("genesis_request", operation=demote, address=target.address)
        target.call("genesis_confirm", operation=demote)
        peer.call("genesis_result", operation=demote, address=target.address)
        assert self.role(target, target.node) == "member"
        target.stop()
        stale = self.restore(prefix + "-old-admin-snapshot", peer.remote, snapshot)
        assert stale.node == source_identity
        stale.call("service_rule", ok=False, provider=peer.node, service="files", previous=None, rule=rule)
        stale.call("replacement_complete", address=peer.address, receipt_path=self.path(prefix + "-demoted-receipt.json", stale.remote))
        assert self.role(stale, stale.node) == "member"
        assert stale.call("custody_status")["holds_genesis"]
        stale.call("service_rule", ok=False, provider=peer.node, service="files", previous=None, rule=rule)
        self.exchange(stale, peer)
        stale.stop()
        remove = self.propose(peer, "remove-member", source_identity)
        ordinary = peer.call("device_execute", operation=remove)
        assert ordinary["requires_genesis"] is False and self.role(peer, source_identity) is None
        removed = self.restore(prefix + "-removed-snapshot", peer.remote, snapshot)
        removed.call("replacement_complete", ok=False, address=peer.address, receipt_path=self.path(prefix + "-forbidden-receipt.json", removed.remote))
        assert removed.call("replacement_status")["ready"] is False
        assert removed.call("status")["stage"] == "RestoringAuthorization"
        removed.call("ping", ok=False, address=peer.address)
        removed.call("files_enable", ok=False, directory=self.path(prefix + "-removed-files", removed.remote))
        # Removing membership cannot physically invalidate the retained root.
        root_path = self.path(prefix + "-retained-root-card.json", removed.remote)
        removed.call("export_recovery_card", path=root_path)
        root = removed.call("inspect_recovery_card", path=root_path)
        assert root["network"] == prepared["package"]["network"]["root"]
        self.record(prefix, "old_snapshot_does_not_revive_authority", dict(
            identity=source_identity, demotedSnapshotAdoptedCurrentMemberRole=True,
            removedSnapshotRemainedBlocked=True, ordinaryRemovalWithoutGenesis=True,
            physicallyRetainedRootStillValid=True, originalRuleRevision=revision,
            sourceCardRemainsIndependent=bool(card_name)))
        removed.stop()
        peer.stop()


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
        parser.error("actual Windows pin and hash required")
    ReplacementRunner(args).run()


if __name__ == "__main__":
    main()
