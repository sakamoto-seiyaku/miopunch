#!/usr/bin/env python3
"""Real Windows content effects under current Web target and single ACL changes."""
import json
import time

from aiohttp import web

from web_protocol_target import WebTarget


class PolicyTarget(WebTarget):
    destination = None

    async def handle(self, request):
        if request.path == "/redirect-other":
            self.requests.append(request.path)
            return web.Response(status=302, headers={"Location": self.destination})
        return await super().handle(request)


class WindowsPolicyChecks:
    def __init__(self, runner, host, gateway, provider, consumer, target):
        self.runner, self.host, self.gateway = runner, host, gateway
        self.provider, self.consumer, self.target = provider, consumer, target
        self.record, self.until = runner.record, runner.until
        self.sequence = 0

    def fetch(self, expected, path="/api"):
        self.sequence += 1
        argument = json.dumps(path + ("&" if "?" in path else "?") + "owned-policy=" + str(self.sequence))
        before = len(self.target.requests)
        self.host.evaluate("""((path)=>{
            window.policyFetch={done:false};
            fetch(path,{cache:'no-store'}).then(async response=>{
                window.policyFetch={done:true,ok:response.ok,status:response.status,body:await response.text()};
            },()=>{window.policyFetch={done:true,ok:false,status:0};});
            return true;
        })(""" + argument + ")")
        self.until(lambda: (self.host.evaluate("window.policyFetch") or {}).get("done"),
                   "actual content current-policy fetch did not settle")
        result = self.host.evaluate("window.policyFetch")
        assert result["ok"] is expected, result
        if expected:
            assert result["status"] == 200 and result["body"] == "owned M119 fetched resource", result
            assert len(self.target.requests) == before + 1
        elif path == "/api":
            assert len(self.target.requests) == before, "current denied content fetch reached provider target"
        return result

    def live_socket(self):
        self.socket_closes_before = len(self.gateway.socket_close_results)
        self.host.evaluate("""(()=>{
            window.policySocketState={greeting:false,opened:false,closed:false,error:false};
            const socket=window.policySocket=new WebSocket(location.origin.replace(/^http/,'ws')+'/socket');
            socket.onopen=()=>window.policySocketState.opened=true;
            socket.onmessage=event=>{if(event.data==='owned M119 ready')window.policySocketState.greeting=true;};
            socket.onerror=()=>window.policySocketState.error=true;
            socket.onclose=event=>{window.policySocketState.closed=true;window.policySocketState.code=event.code;};
            return true;
        })()""")
        self.until(lambda: self.host.evaluate("policySocketState.opened&&policySocketState.greeting&&!policySocketState.closed&&!policySocketState.error"),
                   "current authorized actual content socket did not reach independent target")
        self.until(lambda: len(self.target.active) == 1 and self.provider.call("web_status")["active_flows"] == 1
                   and self.consumer.call("web_status")["active_flows"] == 1,
                   "actual content socket provider/consumer ownership premise did not settle")

    def closed(self, label):
        self.until(lambda: not self.target.active and self.provider.call("web_status")["active_flows"] == 0
                   and self.consumer.call("web_status")["active_flows"] == 0,
                   "current " + label + " retained an actual content/provider/consumer socket owner", seconds=10)
        self.until(lambda: self.host.evaluate("policySocketState.closed===true"),
                   "actual content did not observe closure after current " + label, seconds=10)
        self.until(lambda: len(self.gateway.socket_close_results) == self.socket_closes_before + 1,
                   "gateway original socket close callback did not settle after current " + label, seconds=10)
        assert self.gateway.socket_close_results[-1]["closed"] is True
        assert self.gateway.socket_close_results[-1]["failed"] is True
        self.record("controller", "actual_content_policy_socket_closed", dict(cause=label,
            actual_target_socket_gone=True, provider_consumer_active_flows=0,
            content=self.host.evaluate("policySocketState"), SDK_closed=True, SDK_failed=True))

    def denied(self, label):
        before, connections = len(self.target.requests), self.target.connections
        self.fetch(False)
        self.host.evaluate("""(()=>{
            window.deniedSocket={opened:false,closed:false,error:false};
            const socket=new WebSocket(location.origin.replace(/^http/,'ws')+'/socket');
            socket.onopen=()=>{window.deniedSocket.opened=true;socket.close();};
            socket.onerror=()=>window.deniedSocket.error=true;
            socket.onclose=()=>window.deniedSocket.closed=true;
            return true;
        })()""")
        self.until(lambda: self.host.evaluate("deniedSocket.closed&&deniedSocket.error&&!deniedSocket.opened"),
                   "current denied content WebSocket did not fail before opening")
        assert len(self.target.requests) == before and self.target.connections == connections
        assert not self.target.active
        self.record("controller", "actual_content_policy_no_effect", dict(cause=label,
            HTTP_denied=True, native_WebSocket_denied=True, target_requests_and_connections_unchanged=True))
        time.sleep(2.05)  # Retain the existing refused-contact backoff.

    def exercise(self):
        replacement = WebTarget(self.runner)
        self.target.destination = replacement.url
        before = len(replacement.requests)
        redirect = self.fetch(False, "/redirect-other")
        assert redirect["status"] == 502 and len(replacement.requests) == before
        self.record("controller", "actual_content_cross_origin_redirect_denied", dict(
            original_target_returned_redirect=True, other_owned_target_not_contacted=True))
        self.live_socket()
        self.provider.call("web_enable", targets=[replacement.url])
        self.closed("target_replacement")
        self.denied("old_target_replaced")
        self.provider.call("web_enable", targets=[self.target.url], trust_roots_pem=[self.target.trust_root_pem])
        self.fetch(True)
        self.live_socket()
        self.provider.call("web_disable")
        self.closed("service_disable")
        self.denied("service_off")
        self.provider.call("web_enable", targets=[self.target.url], trust_roots_pem=[self.target.trust_root_pem])
        rule = dict(members=False, guests=True, allowed=[], denied=[])
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="web", previous=None, rule=rule)["revision"]
        self.denied("current_member_default_denied")
        rule["allowed"] = [self.consumer.identity]
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="web", previous=revision, rule=rule)["revision"]
        self.fetch(True)
        self.live_socket()
        rule["denied"] = [self.consumer.identity]
        self.provider.call("service_rule", provider=self.provider.identity, service="web", previous=revision, rule=rule)
        self.closed("explicit_deny_overrides_allow")
        self.denied("explicit_deny_overrides_allow")
        self.record("controller", "actual_Windows_content_current_single_ACL", dict(
            target_replacement_closed_live_content_socket=True, service_off_closed_live_content_socket=True,
            member_default_denied=True, explicit_allow_restored_real_content_HTTP_WS=True,
            explicit_deny_wins_and_closes_live_content_socket=True,
            no_HTTP_or_WS_target_effect_after_each_denial=True, cross_origin_redirect_not_followed=True))
