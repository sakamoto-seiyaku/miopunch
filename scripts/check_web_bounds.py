#!/usr/bin/env python3
"""Bounded Native Web handles and current ACL at full real socket capacity."""
import argparse
import base64
import time
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


class BoundsRunner(WebRunner):
    wait_seconds = 10

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("web_enable", targets=[target.url])
        jobs = [consumer.call("web_begin", address=provider.address, url=target.url)["id"] for _ in range(16)]
        self.until(lambda: consumer.call("web_status")["active_flows"] == 0, "HTTP results did not complete")
        assert len(target.requests) == 16
        status = consumer.call("web_status")
        assert status["request_handles"] == status["max_request_handles"] == 16
        consumer.call("web_begin", address=provider.address, url=target.url, ok=False)
        assert len(target.requests) == 16
        for job in jobs:
            result = consumer.call("web_result", job=job)
            assert not result["pending"] and result["response"]["status"] == 200
            assert base64.b64decode(result["response"]["body"]) == b"owned M119 web target"
        assert consumer.call("web_status")["request_handles"] == 0
        sockets = []
        for _ in range(16):
            identifier = consumer.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
            assert consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned M119 ready"}
            sockets.append(identifier)
        assert len(target.active) == provider.call("web_status")["active_flows"] == 16
        before = len(target.requests)
        consumer.call("web_socket_open", address=provider.address, url=target.socket_url, ok=False)
        assert len(target.requests) == before
        consumer.call("web_socket_send", socket=sockets[0], message={"type": "text", "text": "M119 at full capacity"})
        assert consumer.call("web_socket_read", socket=sockets[0], timeout_ms=2000)["message"] == {"type": "text", "text": "owned-target:M119 at full capacity"}
        rule = {"members": False, "guests": False, "allowed": [], "denied": []}
        provider.call("service_rule", provider=provider.identity, service="web", previous=None, rule=rule)
        self.until(lambda: not target.active and provider.call("web_status")["active_flows"] == 0,
                   "current ACL could not close full-capacity provider owners")
        for identifier in sockets:
            result = consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)
            assert result["closed"] and result.get("failed"), result
            assert consumer.call("web_socket_close", socket=identifier)["failed"]
        assert consumer.call("web_status")["active_flows"] == 0
        time.sleep(2.05)
        consumer.call("web_open", address=provider.address, url=target.url, ok=False)
        assert len(target.requests) == before
        self.record("controller", "bounded_web_handles_and_policy_ingress", {"completed_http_results": 16,
                    "extra_http_handle_no_effect": True, "live_socket_flows": 16, "extra_socket_no_effect": True,
                    "duplex_at_capacity": True, "current_acl_ingress_closed_all": True, "no_effect_after": True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="bounds")
    BoundsRunner(parser.parse_args()).run()
