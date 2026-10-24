#!/usr/bin/env python3
"""Breadth of the observed admission/contact class at ordinary public entries.

No fault hooks, route changes, build retries or added readiness sleeps. Each
case is a distinct manual/precode entry and OS role, with its own negative.
"""
import argparse
import json
from pathlib import Path
import time

from check_file_transfer import FileRunner


class AdmissionBreadth(FileRunner):
    def scenario(self, prefix, creator_remote=False):
        for kind in ("manual", "precode"):
            name = prefix + "-" + kind
            creator = self.start(name + "-creator", creator_remote)
            creator.call("export_recovery_card", path=self.path(name + "-card.json", creator_remote))
            creator.call("acknowledge_recovery_saved")
            invite_name = name + "-invite.json"
            if kind == "manual":
                creator.call("invite", path=self.path(invite_name, creator_remote))
            else:
                creator.call("issue_precode", path=self.path(invite_name, creator_remote), seconds=3600, max_uses=1)
            remote = not creator_remote if not self.local_only else False
            invitation = self.transfer(invite_name, creator_remote, remote)
            member = self.start(name + "-member", remote, "join", invitation)
            member.call("files_enable", ok=False, directory=self.path(name + "-premature", remote))
            began = time.monotonic()
            try:
                result = member.call("request", path=invitation)
                if kind == "manual":
                    assert result["stage"] == "AwaitingApproval"
                    pending = creator.call("requests")["requests"]
                    assert len(pending) == 1 and pending[0]["applicant"] == member.node
                    assert member.call("request", path=invitation)["stage"] == "AwaitingApproval"
                    assert creator.call("requests")["requests"] == pending
                    creator.call("approve", **pending[0])
                    assert member.call("request", path=invitation)["stage"] == "Complete"
                else:
                    assert result["stage"] == "Complete"
                    assert creator.call("precode_status", path=self.path(invite_name, creator_remote))["uses"] == 1
                self.exchange(creator, member)
                outsider = self.start(name + "-negative", remote, "join", invitation)
                if kind == "manual":
                    assert outsider.call("request", path=invitation)["stage"] == "AwaitingApproval"
                    pending = creator.call("requests")["requests"]
                    request = next(r for r in pending if r["applicant"] == outsider.node)
                    creator.call("reject", **request)
                    assert outsider.call("request", path=invitation)["stage"] == "Rejected"
                else:
                    outsider.call("request", ok=False, path=invitation)
                    assert outsider.call("status")["stage"] != "Complete"
                    assert creator.call("precode_status", path=self.path(invite_name, creator_remote))["uses"] == 1
                outsider.call("ping", ok=False, address=creator.address)
                self.record(name, "admission_class_public_breadth", dict(kind=kind,
                    creatorWindows=creator_remote, sourceIdentity=creator.node, destinationIdentity=member.node,
                    sourceState=creator.call("status"), destinationState=member.call("status"),
                    seconds=time.monotonic()-began, noReadinessSleep=True, positiveAndNegative=True))
                outsider.stop()
            except Exception:
                self.record(name, "failure_independent_state", dict(source=creator.call("status"),
                    address=creator.call("address"), pending=creator.call("requests"), applicant=member.call("status"),
                    started=began, observed=time.monotonic()))
                raise
            finally:
                member.stop()
                creator.stop()


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
    AdmissionBreadth(args).run()


if __name__ == "__main__":
    main()
