#!/usr/bin/env python3
"""Ordinary public guest/file operations on immutable Native SDK pins."""
import argparse
import json
from pathlib import Path
import time

from check_service_acl import ServiceRunner


class GuestRunner(ServiceRunner):
    def pending(self, host, identifier, sender=None, preparation_timeout=45):
        deadline = time.monotonic() + preparation_timeout
        while time.monotonic() < deadline:
            task = self.task(host, identifier)
            if task and task["state"] == "awaiting_approval":
                assert task["bytes"] == 0 and task["receipt"] is None
                return task
            if task and task["state"] in {"complete", "failed", "cancelled"}:
                raise AssertionError(("guest request ended before confirmation", task))
            if sender is not None:
                source = self.task(sender, identifier)
                if source and source["state"] in {"complete", "failed", "cancelled"}:
                    raise AssertionError(("source ended before the pending prerequisite", source, task))
            time.sleep(0.05)
        raise AssertionError("guest file did not expose its public pending confirmation")

    def saved_task(self, sender, receiver, task, expected, directory):
        sent = self.wait_task(sender, task)
        received = self.wait_task(receiver, task)
        assert sent["state"] == received["state"] == "complete", (sent, received)
        assert sent["receipt"] == received["receipt"]
        receipt = received["receipt"]
        assert receipt["size"] == expected["size"] and receipt["sha256"] == expected["sha256"]
        destination = directory + "/" + receipt["saved_name"]
        assert self.hash_file(destination, receiver.remote) == expected
        assert not any(name.endswith(".partial") for name in self.listing(directory, receiver.remote))
        self.qualifications.append(dict(source=sender.label, receiver=receiver.label, **expected,
            receipt=receipt, transport=sent["transport"], guestConfirmed=True))
        self.remove(destination, receiver.remote)

    def scenario(self, prefix, creator_remote=False):
        other, provider = self.paired(prefix, creator_remote)
        other_dir = self.enable(other, other.label)
        source, expected = self.generate(provider, prefix + "-baseline.bin", 11)
        self.delivered(provider, other, source, expected, other_dir)
        directory = self.enable(provider, provider.label)
        name = prefix + "-guest-invite.json"
        provider.call("guest_issue", path=self.path(name, provider.remote), provider=provider.address, seconds=180)
        guest_remote = not provider.remote if not self.local_only else False
        invitation = self.transfer(name, provider.remote, guest_remote)
        guest = self.start(prefix + "-guest", guest_remote, "join-guest", invitation)
        source, expected = self.generate(guest, prefix + "-guest.bin", 4 * 1024**2)
        guest.call("file_send", ok=False, address=provider.address, path=source)
        assert guest.call("guest_redeem")["stage"] == "GuestActive"
        status = guest.call("guest_status")
        assert status["role"] == "guest" and status["scope"]["guest"] == guest.node
        deadline = status["scope"]["expires"]
        assert status["scope"]["provider"] == provider.node and status["scope"]["service"] == "files"
        assert guest.node not in [m["node"] for m in provider.call("status")["members"]]
        second = self.start(prefix + "-second-identity", guest_remote, "join-guest", invitation)
        second.call("guest_redeem", ok=False)
        second.call("file_send", ok=False, address=provider.address, path=source)
        assert len(provider.call("service_status")["guests"]) == 1
        # The invited service on another node remains unavailable.
        guest.call("file_send", ok=False, address=other.address, path=source)
        assert other.call("files_status")["tasks"][-1]["state"] == "complete"
        assert not self.listing(other_dir, other.remote)
        # No sink, saved bytes, or completion before the public decision.
        task = guest.call("file_send", address=provider.address, path=source)["task"]
        self.pending(provider, task)
        assert self.task(guest, task)["state"] != "complete"
        assert not self.listing(directory, provider.remote)
        assert provider.call("file_reject", task=task)["rejected"] is True
        denied = self.wait_task(guest, task)
        assert denied["state"] == "failed" and denied["error"] == "rejected" and denied["receipt"] is None
        assert not self.listing(directory, provider.remote)
        self.record(provider.label, "guest_file_reject", dict(noSavedFile=True, noPreApprovalSink=True))
        task = guest.call("file_send", address=provider.address, path=source)["task"]
        self.pending(provider, task)
        assert not self.listing(directory, provider.remote)
        assert provider.call("file_approve", task=task)["approved"] is True
        assert provider.call("file_approve", task=task)["approved"] is False
        self.saved_task(guest, provider, task, expected, directory)
        guest = self.reopen(guest)
        assert guest.call("guest_redeem")["stage"] == "GuestActive"
        assert guest.call("guest_status")["scope"]["expires"] == deadline
        # A fresh transfer still needs confirmation after reconnect.
        task = guest.call("file_send", address=provider.address, path=source)["task"]
        self.pending(provider, task)
        provider.call("file_approve", task=task)
        self.saved_task(guest, provider, task, expected, directory)
        guest.call("ping", ok=False, address=provider.address)
        time.sleep(2)
        result = guest.call("guest_end")
        assert result["ended_locally"] and all(x["saved"] for x in result["delivery"]), result
        assert guest.call("guest_status")["stage"] == "GuestEnded"
        guest.call("file_send", ok=False, address=provider.address, path=source)
        assert provider.call("service_status")["guests"][0]["ended"] is True
        guest.call("guest_redeem", ok=False)
        self.record(guest.label, "guest_identity_scope_reconnect_exit", dict(oneIdentity=True,
            scopeEnforced=True, deadlinePreserved=deadline, ordinaryMembershipNotGranted=True,
            realSavesAfterConfirmation=2, exitDelivered=True, accessStopped=True))
        other.call("service_sync", address=provider.address)
        previous = provider.call("service_status")["services"][0]["policy"]["revision"]
        other.call("service_rule", provider=provider.node, service="files", previous=previous,
            rule=dict(members=True, guests=False, allowed=[], denied=[]))
        other.call("service_sync", address=provider.address)
        before = len(provider.call("service_status")["guests"])
        provider.call("guest_issue", ok=False, path=self.path(prefix + "-restricted-guest-invite.json", provider.remote),
            provider=provider.address, seconds=180)
        assert len(provider.call("service_status")["guests"]) == before
        self.record(provider.label, "ordinary_provider_sharing_obeys_remote_acl", dict(ordinarySelfIssuer=True,
            remoteRestrictionBlocksNewShare=True, sameEffectiveRule=True))
        for host in (guest, second, other, provider):
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
    args.focus = "smoke"
    args.failure_only = False
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("actual Windows pin and hash required")
    GuestRunner(args).run()


if __name__ == "__main__":
    main()
