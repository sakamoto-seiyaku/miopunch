#!/usr/bin/env python3
"""Ordinary owned WebSocket bidirectional acceptance via the public Web module.

The protocol sample is newly created for M119. It is not a forwarding, UDP,
large-reply, old credential, remote relay or Windows probe.
"""
import argparse
import asyncio
import base64
from pathlib import Path

from web_protocol_target import WebTarget

from check_web_service import OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process




class SocketRunner(WebRunner):
    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = WebTarget(self)
        asyncio.run(target.socket_prerequisite())
        assert target.observed == [("text", "M119 direct target prerequisite")]
        self.record("controller", "independent_websocket_precondition", {"real_handshake": True, "unsolicited_greeting": True,
                    "actual_bidirectional_text": True})
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        provider.call("web_enable", targets=[target.url])
        opened = consumer.call("web_socket_open", address=provider.address, url=target.socket_url)
        identifier = opened["id"]
        greeting = consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)
        assert greeting["message"] == {"type": "text", "text": "owned M119 ready"}
        consumer.call("web_socket_send", socket=identifier, message={"type": "text", "text": "M119 SDK text"})
        assert consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type": "text", "text": "owned-target:M119 SDK text"}
        payload = bytes(range(37))
        message = {"type": "binary", "data": base64.b64encode(payload).decode()}
        consumer.call("web_socket_send", socket=identifier, message=message)
        assert consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == message
        assert target.observed[1:] == [("text", "M119 SDK text"), ("binary", payload)]
        closed = consumer.call("web_socket_close", socket=identifier)
        assert closed == {"closed":True, "failed":False}, closed
        self.until(lambda: consumer.call("web_status")["active_flows"] == 0
                   and provider.call("web_status")["active_flows"] == 0 and not target.active,
                   "normal SDK close retained a module or independent target owner")
        self.record("controller", "public_websocket_delivery", {"unsolicited_message": True, "text_exact": True,
                    "binary_exact": True, "target_independently_observed": True,
                    "normal_close":True, "SDK_failed":False, "module_and_target_owners_settled":True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="socket")
    SocketRunner(parser.parse_args()).run()
