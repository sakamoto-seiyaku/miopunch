#!/usr/bin/env python3
"""Actual Web off/target replacement/redirect/single-ACL negative effects."""
import argparse
import base64
import json
import time
from pathlib import Path

from aiohttp import web
from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


class RedirectTarget(HttpTarget):
    destination = None
    async def handle(self, request):
        if request.path == "/redirect":
            self.requests.append(request.path)
            return web.Response(status=302, headers={"Location": self.destination})
        return await super().handle(request)


class PolicyRunner(WebRunner):

    def denied(self, consumer, provider, target):
        before = len(target.requests)
        consumer.call("web_open", address=provider.address, url=target.url, ok=False)
        consumer.call("web_socket_open", address=provider.address, url=target.socket_url, ok=False)
        assert len(target.requests) == before
        time.sleep(2.05)

    def positive(self, consumer, provider, target):
        before = len(target.requests)
        response = consumer.call("web_open", address=provider.address, url=target.url)
        assert response["status"] == 200 and base64.b64decode(response["body"]) == b"owned M119 web target"
        assert len(target.requests) == before+1
        identifier = consumer.call("web_socket_open", address=provider.address, url=target.socket_url)["id"]
        assert consumer.call("web_socket_read", socket=identifier, timeout_ms=2000)["message"] == {"type":"text","text":"owned M119 ready"}
        return identifier

    def closed(self, consumer, provider, target, identifier):
        self.until(lambda: not target.active and provider.call("web_status")["active_flows"] == 0,
                   "current Web target/policy change left provider effect")
        self.until(lambda: consumer.call("web_status")["active_flows"] == 0,
                   "current Web target/policy change left consumer owner")
        assert consumer.call("web_socket_close", socket=identifier)["failed"]

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = RedirectTarget(self)
        replacement = HttpTarget(self)
        target.destination = replacement.url
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        outsider = OwnedNode(self, "outsider")
        declared = next(row for row in provider.call("service_status")["services"] if row["service"] == "web")
        assert declared["capable"] and not declared["enabled"]
        self.denied(consumer, provider, target)
        provider.call("web_enable", targets=[target.url])
        identifier = self.positive(consumer, provider, target)
        self.denied(outsider, provider, target)
        before = len(replacement.requests)
        response = consumer.call("web_open", address=provider.address, url=target.url+"redirect")
        assert response["status"] == 302 and [value for name,value in response["headers"] if name.lower()=="location"] == [replacement.url]
        assert len(replacement.requests) == before, "provider automatically followed an unauthorized redirect"
        self.denied(consumer, provider, replacement)
        provider.call("web_enable", targets=[replacement.url])
        self.closed(consumer, provider, target, identifier)
        self.denied(consumer, provider, target)
        identifier = self.positive(consumer, provider, replacement)
        provider.call("web_disable")
        self.closed(consumer, provider, replacement, identifier)
        self.denied(consumer, provider, replacement)
        assert not provider.call("web_status")["enabled"]
        provider.call("web_enable", targets=[replacement.url])
        rule = {"members":False,"guests":True,"allowed":[],"denied":[]}
        revision = provider.call("service_rule", provider=provider.identity, service="web", previous=None, rule=rule)["revision"]
        self.denied(consumer, provider, replacement)
        rule["allowed"] = [consumer.identity]
        revision = provider.call("service_rule", provider=provider.identity, service="web", previous=revision, rule=rule)["revision"]
        identifier = self.positive(consumer, provider, replacement)
        rule["denied"] = [consumer.identity]
        provider.call("service_rule", provider=provider.identity, service="web", previous=revision, rule=rule)
        self.closed(consumer, provider, replacement, identifier)
        self.denied(consumer, provider, replacement)
        self.record("controller", "actual_web_effect_time_authorization", {"initial_off_no_http_ws_effect":True,
                    "outsider_denied":True,"redirect_returned_without_following":True,"redirect_target_denied":True,
                    "target_replacement_closes_old_socket":True,"disable_closes_socket":True,
                    "member_default_denied_then_explicit_allowed":True,"explicit_deny_wins_and_closes_socket":True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,required=True)
    parser.add_argument("--sha256",required=True)
    parser.add_argument("--source-receipt",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--relay-binary",type=Path,required=True)
    parser.add_argument("--relay-receipt",type=Path,required=True)
    parser.add_argument("--case",default="policy")
    PolicyRunner(parser.parse_args()).run()
