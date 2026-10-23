#!/usr/bin/env python3
"""Public file/ACL acceptance with real SDK nodes and independent saved reads.

Never builds. Sources and imported controllers are frozen by FileRunner before
any operation. SSH is deployment/control only; file bytes use miopunch.
"""
import argparse
import json
from pathlib import Path
import time

from check_file_transfer import FileRunner


class ServiceRunner(FileRunner):
    def member(self, administrator, label, remote=False):
        name = label + "-invite.json"
        administrator.call("issue_precode", path=self.path(name, administrator.remote), seconds=3600, max_uses=1)
        invitation = self.transfer(name, administrator.remote, remote)
        member = self.start(label, remote, "join", invitation)
        assert member.call("request", path=invitation)["stage"] == "Complete"
        return member

    def scenario(self, prefix, creator_remote=False):
        admin, provider = self.paired(prefix, creator_remote)
        caller = self.member(admin, prefix + "-caller", creator_remote)
        directory = self.enable(provider, prefix)
        source, expected = self.generate(admin, prefix + "-baseline.bin", 11)
        # Establish the genuine file prerequisite before exercising a new rule.
        self.delivered(admin, provider, source, expected, directory)
        admin.call("service_sync", address=provider.address)
        declarations = admin.call("service_status")["declarations"]
        assert any(d["provider"] == provider.node and d["service"] == "files" and d["capable"] and d["enabled"] for d in declarations)
        caller.call("service_sync", address=admin.address)
        source, expected = self.generate(caller, prefix + "-caller.bin", 4 * 1024**2)
        self.delivered(caller, provider, source, expected, directory)
        rule = dict(members=True, guests=True, allowed=[], denied=[caller.node])
        before = provider.call("service_status")
        caller.call("service_rule", ok=False, provider=provider.node, service="files", previous=None, rule=rule)
        assert provider.call("service_status") == before
        changed = admin.call("service_rule", provider=provider.node, service="files", previous=None, rule=rule)
        admin.call("service_sync", address=provider.address)
        effective = provider.call("service_status")["services"][0]["policy"]
        assert effective["revision"] == changed["revision"] and effective["rule"] == rule
        # Caller deliberately has its previously valid view. The actual
        # provider must reject it, with no file or completed receipt.
        task = caller.call("file_send", address=provider.address, path=source)["task"]
        failed = self.wait_task(caller, task)
        assert failed["state"] == "failed" and failed["receipt"] is None, failed
        assert not self.listing(directory, provider.remote)
        assert self.task(provider, task) is None
        self.record(provider.label, "effective_acl_real_denial", dict(noSavedFile=True, noReceiveTask=True,
                    callerKeptEarlierView=True, revision=changed["revision"]))
        default = dict(members=True, guests=True, allowed=[], denied=[])
        local = provider.call("service_rule", provider=provider.node, service="files", previous=changed["revision"], rule=default)
        assert local["revision"] != changed["revision"]
        provider.call("service_sync", address=admin.address)
        forwarded = [p for p in admin.call("service_status")["policies"] if p["provider"] == provider.node and p["service"] == "files"]
        assert len(forwarded) == 1 and forwarded[0]["revision"] == local["revision"] and forwarded[0]["rule"] == default
        # The same original file operation and independent hash must now pass.
        time.sleep(2)  # Existing bounded contact backoff after the real denial.
        self.delivered(caller, provider, source, expected, directory)
        selected = dict(members=False, guests=True, allowed=[caller.node], denied=[])
        selected_revision = admin.call("service_rule", provider=provider.node, service="files", previous=local["revision"], rule=selected)["revision"]
        admin.call("service_sync", address=provider.address)
        self.delivered(caller, provider, source, expected, directory)
        admin.call("file_send", ok=False, address=provider.address, path=self.path(prefix + "-baseline.bin", admin.remote))
        local = provider.call("service_rule", provider=provider.node, service="files", previous=selected_revision, rule=selected)
        provider.call("service_sync", address=admin.address)
        provider = self.reopen(provider)
        reopened = provider.call("service_status")["services"][0]
        assert reopened["policy"]["revision"] == local["revision"]
        assert reopened["policy"]["rule"] == selected and reopened["enabled"] is False
        provider.call("files_enable", directory=directory)
        assert provider.call("service_status")["services"][0]["policy"]["rule"] == selected
        self.delivered(caller, provider, source, expected, directory)
        self.record(provider.label, "one_durable_acl", dict(remoteThenLocalSameRevision=True,
                    dailyOperationWithoutGenesis=True, reopenedRevision=local["revision"], selectedMemberAllowed=True,
                    defaultMemberDenied=True, explicitRestrictionSurvivesEnable=True))
        for host in (caller, provider, admin):
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
    ServiceRunner(args).run()


if __name__ == "__main__":
    main()
