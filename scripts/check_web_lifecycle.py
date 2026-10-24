#!/usr/bin/env python3
"""Owned pending HTTP cancellation through the ordinary public Web entry."""
import argparse
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


class LifecycleRunner(WebRunner):

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("web_enable", targets=[target.url])
        assert consumer.call("web_open", address=provider.address, url=target.url)["status"] == 200
        assert target.requests == ["/"]
        self.record("controller", "pending_web_preconditions", {"real_http_public_entry": True, "owned_target_exact": True})
        job = consumer.call("web_begin", address=provider.address, url=target.url + "pending")
        self.until(target.pending_started.is_set, "target never observed pending HTTP")
        assert consumer.call("web_result", job=job["id"])["pending"]
        started = time.monotonic()
        consumer.call("web_cancel", job=job["id"])
        assert time.monotonic() - started < 5
        consumer.call("web_result", job=job["id"], ok=False)
        self.until(lambda: provider.call("web_status")["active_flows"] == 0, "provider HTTP owner remained after cancel")
        assert consumer.call("web_status")["active_flows"] == 0
        self.record("controller", "pending_web_cancel", {"target_observed": True, "cancel_seconds": time.monotonic() - started,
                    "provider_and_consumer_joined": True, "no_false_success_result": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="lifecycle")
    LifecycleRunner(parser.parse_args()).run()
