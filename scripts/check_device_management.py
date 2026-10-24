#!/usr/bin/env python3
"""Ordinary public device/genesis operations on real Windows/Linux SDK hosts.

Never compiles. SSH deploys/controls hosts and delivers private offline cards;
management requests, confirmations, history and business use miopunch itself.
"""
import argparse
import json
from pathlib import Path
import subprocess

from check_file_transfer import FileRunner
from check_native_client import quote


class DeviceRunner(FileRunner):
    def role(self, host, target):
        members = host.call("status")["members"]
        return next((m["role"] for m in members if m["node"] == target), None)

    def propose(self, host, action, target):
        result = host.call("genesis_propose", action=action, target=target)
        assert result["stage"] == "awaiting_confirmation"
        assert result["operation"]["description"]["action"] == action
        assert result["operation"]["description"]["target"] == target
        assert result["effective_locally"] is False
        return result["operation"]["id"]

    def private_card(self, prefix, source, destination):
        name = prefix + "-card.json"
        if source.remote != destination.remote:
            src = "laptop:" + self.path(name, True) if source.remote else self.path(name, False)
            dst = "laptop:" + self.path(name, True) if destination.remote else self.path(name, False)
            subprocess.run(["scp", "-q", src, dst], check=True)
            if not destination.remote:
                Path(dst).chmod(0o600)
        return self.path(name, destination.remote)

    def scenario(self, prefix, creator_remote=False):
        custodian, initiator = self.paired(prefix, creator_remote)
        self.exchange(custodian, initiator)
        assert not initiator.call("device_operations")["holds_genesis"]
        before = initiator.call("status")["records"]
        operation = self.propose(initiator, "appoint-administrator", initiator.node)
        initiator.call("device_execute", ok=False, operation=operation)
        initiator.call("genesis_confirm", ok=False, operation=operation)
        assert initiator.call("status")["records"] == before
        for _ in range(2):
            pending = initiator.call("genesis_request", operation=operation, address=custodian.address)
            assert pending["stage"] == "awaiting_confirmation"
        requests = custodian.call("device_operations")["operations"]
        assert len(requests) == 1 and requests[0]["operation"]["id"] == operation
        confirmed = custodian.call("genesis_confirm", operation=operation)
        assert confirmed["stage"] == "confirmed" and confirmed["effective_locally"]
        assert self.role(custodian, initiator.node) == "admin"
        received = initiator.call("genesis_request", operation=operation, address=custodian.address)
        assert received["record"] == confirmed["record"] and received["effective_locally"]
        assert self.role(initiator, initiator.node) == "admin"
        assert not initiator.call("device_operations")["holds_genesis"]
        saved_count = initiator.call("status")["records"]
        retry = initiator.call("genesis_request", operation=operation, address=custodian.address)
        assert retry["record"] == confirmed["record"]
        assert initiator.call("status")["records"] == saved_count
        # This independent action cannot borrow the first confirmation.
        demotion = self.propose(initiator, "demote-administrator", initiator.node)
        initiator.call("device_execute", ok=False, operation=demotion)
        initiator.call("genesis_confirm", ok=False, operation=demotion)
        assert self.role(initiator, initiator.node) == "admin"
        # Explicit offline fallback uses the same card for only this action.
        card = self.private_card(prefix, custodian, initiator)
        demoted = initiator.call("genesis_confirm", operation=demotion, card_path=card)
        assert demoted["stage"] == "confirmed" and self.role(initiator, initiator.node) == "member"
        assert not initiator.call("device_operations")["holds_genesis"]
        assert initiator.call("genesis_confirm", operation=demotion)["record"] == demoted["record"]
        # Demotion retains real ordinary business participation.
        initiator.call("service_sync", address=custodian.address)
        assert self.role(custodian, initiator.node) == "member"
        directory = self.enable(custodian, prefix + "-post-demotion")
        path, expected = self.generate(initiator, prefix + "-post-demotion.bin", 11)
        self.delivered(initiator, custodian, path, expected, directory)
        self.record(prefix, "specific_genesis_confirmations", dict(remoteSavedRecord=confirmed["record"],
            retryRecord=retry["record"], noRootCopied=True, secondActionRequiredFreshConfirmation=True,
            offlineDemotionRecord=demoted["record"], ordinaryMembershipAndSavedBusinessRetained=True))
        for host in (initiator, custodian):
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
        parser.error("actual Windows pin and hash required")
    DeviceRunner(args).run()


if __name__ == "__main__":
    main()
