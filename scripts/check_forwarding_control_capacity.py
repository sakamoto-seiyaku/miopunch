#!/usr/bin/env python3
"""Remote current ACL must remain usable at the forwarding service flow bound."""
import argparse
from contextlib import ExitStack
from pathlib import Path
from check_forwarding import Node, Target
from check_forwarding_matrix import MatrixRunner
from forwarding_local_resources import configure_local_resources


class CapacityRunner(MatrixRunner):
    def __init__(self, args):
        super().__init__(args)
        self.args = args

    def scenario(self):
        self.freeze_source(Path(__file__))
        self.freeze_source(Path(__file__).with_name("check_forwarding_matrix.py"))
        configure_local_resources(self, self.args)
        admin = Node(self, "admin")
        provider = self.member(admin, "provider")
        target = Target(self, "udp", prefix=b"")
        self.target_precondition(target)
        provider.call("forward_enable", protocol="udp", targets=[target.address])
        admin.call("service_sync", address=provider.address)
        forwarding = self.start_listener(admin, provider, "udp", target)
        with ExitStack() as stack:
            for index in range(16):
                client = stack.enter_context(self.application(forwarding))
                payload = bytes([index]) * 17
                assert self.exchange(client, "udp", payload) == payload
            before = provider.call("forward_status")
            assert before["active_flows"] == before["max_flows"] == 16
            assert admin.call("forward_status")["active_flows"] == 16
            revision = admin.call("service_rule", provider=provider.identity, service="udp_forward", previous=None,
                                  rule=dict(members=True, guests=True, allowed=[], denied=[admin.identity]))["revision"]
            self.record("controller", "capacity_acl_prerequisites", {"active_real_datagram_associations": 16,
                        "target_live_exact_echo": True, "current_authorized_admin_revision_saved": revision})
            result = admin.call("service_sync", address=provider.address)
            assert result["saved_locally"], "remote ACL synchronization must be available at the normal flow bound"
            current = next(policy for policy in provider.call("service_status")["policies"]
                           if policy["provider"] == provider.identity and policy["service"] == "udp_forward")
            assert current["revision"] == revision
            self.until(lambda: provider.call("forward_status")["active_flows"] == 0,
                       "remote current ACL closes all sixteen actual associations")
        admin.call("forward_stop", task=forwarding["id"])
        self.record("controller", "remote_acl_at_full_capacity", {"saved_at_capacity": True, "exact_revision_enforced": True, "all_provider_flows_zero": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--loopback", default="::1")
    parser.add_argument("--case", default="capacity")
    CapacityRunner(parser.parse_args()).run()
