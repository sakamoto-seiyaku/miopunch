#!/usr/bin/env python3
"""An already-enabled receiver admits newly approved members automatically."""
import argparse
from pathlib import Path

from check_service_acl import ServiceRunner


class AdmissionRunner(ServiceRunner):
    def scenario(self, prefix, creator_remote=False):
        for mode in ["precode", "manual"]:
            name = prefix + "-" + mode
            provider = self.start(name + "-provider", creator_remote)
            provider.call("export_recovery_card", path=self.path(name + "-card.json", creator_remote))
            provider.call("acknowledge_recovery_saved")
            directory = self.enable(provider, name)
            member_remote = not creator_remote if not self.local_only else False
            invitation_name = name + "-invite.json"
            if mode == "precode":
                provider.call("issue_precode", path=self.path(invitation_name, creator_remote), seconds=3600, max_uses=1)
            else:
                provider.call("invite", path=self.path(invitation_name, creator_remote))
            invitation = self.transfer(invitation_name, creator_remote, member_remote)
            member = self.start(name + "-new-member", member_remote, "join", invitation)
            if mode == "manual":
                assert member.call("request", path=invitation)["stage"] == "AwaitingApproval"
                pending = provider.call("requests")["requests"]
                assert len(pending) == 1 and pending[0]["applicant"] == member.node
                provider.call("approve", **pending[0])
            assert member.call("request", path=invitation)["stage"] == "Complete"
            assert member.node in [n["node"] for n in provider.call("status")["members"]]
            assert provider.call("files_status")["enabled"]
            assert provider.call("service_status")["services"][0]["policy"]["rule"]["members"]
            assert member.call("ping", address=provider.address)["reply"] == "PONG"
            source, expected = self.generate(member, name + "-new-member.bin", 11)
            # No service_sync or receiver re-enable between admission and send.
            self.delivered(member, provider, source, expected, directory)
            self.record(provider.label, "enabled_receiver_tracks_new_members", dict(mode=mode,
                membershipVerified=True, memberBusinessWorked=True, noServiceSyncOrReenable=True, actualSavedHash=expected))
            member.stop()
            provider.stop()


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
    AdmissionRunner(args).run()


if __name__ == "__main__":
    main()
