#!/usr/bin/env python3
"""Actual private Linux WebKit + public NativeClient Web service acceptance.

No compiler, SSH, host display, clipboard, external relay or management HTTP
RPC. Data travels through the selected provider and its current Web policy.
"""
import argparse
import asyncio
import http.client
import json
import ssl
import shutil
import os
from pathlib import Path
import threading
import time

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_content_gateway import ContentGateway
from web_content_runtime import PrivateBus, PrivateDesktop, WebKitHost
from web_process_owner import register_process
from urllib.parse import urlsplit
from urllib.request import HTTPSHandler, ProxyHandler, build_opener


class ContainerRunner(WebRunner):
    def __init__(self, args):
        super().__init__(args)
        self.desktops, self.buses, self.hosts = [], [], []
        for name in ["web_content_gateway.py", "web_content_runtime.py", "check_web_service.py"]:
            self.freeze_source(Path(__file__).with_name(name))

    wait_seconds = 20
    poll_seconds = 0.05

    def scenario(self):
        retired, desktop = PrivateDesktop(self, "preflight-retired"), PrivateDesktop(self, "content-desktop")
        assert retired.env["DISPLAY"] != desktop.env["DISPLAY"]
        wrong_authority = retired.root / "wrong-authority"
        shutil.copyfile(desktop.env["XAUTHORITY"], wrong_authority)
        os.chmod(wrong_authority, 0o600)
        wrong = dict(retired.env, XAUTHORITY=str(wrong_authority))
        assert not retired.probe(wrong), "private X11 accepted another display's authority"
        retired.close()
        assert not retired.probe(retired.env), "closed owned desktop fell back to a global display"
        assert desktop.probe(desktop.env, private_network=True), "sandbox-like private net namespace cannot access owned named X11"
        self.record("controller", "private_desktop_preflight", {"wrong_owned_auth_rejected": True,
                    "retired_display_unavailable": True, "other_owned_display_live": True, "clipboard_accessed": False})
        if self.args.case == "desktop-preflight":
            return
        PrivateBus(self, desktop)
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self, tls=self.args.case == "https")
        verification = ssl.create_default_context(cafile=target.certificate) if target.certificate else ssl.create_default_context()
        with build_opener(ProxyHandler({}), HTTPSHandler(context=verification)).open(target.url, timeout=3) as response:
            assert response.read() == b"owned M119 web target"
        asyncio.run(target.socket_prerequisite())
        assert target.observed == [("text", "M119 direct target prerequisite")]
        self.record("controller", "independent_protocol_precondition", {"http_exact": True, "target_tls": bool(target.tls),
                    "websocket_handshake_and_bidirectional_message": True, "normal_certificate_validation": True})
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        if target.trust_root_pem:
            provider.call("web_enable", targets=[target.url])
            before = len(target.requests)
            consumer.call("web_open", address=provider.address, url=target.url, ok=False)
            assert len(target.requests) == before, "untrusted TLS request reached HTTP application"
            self.record("controller", "untrusted_https_no_effect", {"public_request_failed": True, "target_http_unchanged": True})
            time.sleep(2.05)
        provider.call("web_enable", targets=[target.url], trust_roots_pem=[target.trust_root_pem] if target.trust_root_pem else [])
        lock = threading.Lock()

        def request(arguments):
            with lock:
                return consumer.call("web_open", address=provider.address, **arguments)

        def socket_call(operation, **arguments):
            with lock:
                return consumer.call(operation, **arguments)

        callbacks = {
            "open": lambda url, headers: socket_call("web_socket_open", address=provider.address, url=url, headers=headers),
            "send": lambda identifier, message: socket_call("web_socket_send", socket=identifier, message=message),
            "read": lambda identifier: socket_call("web_socket_read", socket=identifier, timeout_ms=100),
            "close": lambda identifier: socket_call("web_socket_close", socket=identifier),
        }
        http_callbacks = {
            "begin": lambda arguments: socket_call("web_begin", address=provider.address, **arguments),
            "result": lambda identifier: socket_call("web_result", job=identifier),
            "cancel": lambda identifier: socket_call("web_cancel", job=identifier),
        }
        gateway = ContentGateway(target.url + "page", request, self.private, self.nodes, socket_callbacks=callbacks,
                                 http_callbacks=http_callbacks)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        self.gateway_safety(gateway, target)
        host = WebKitHost(self, desktop, gateway)
        try:
            self.exercise_content(host, target, gateway, lambda: consumer.call("web_status")["active_flows"] == 0
                                  and provider.call("web_status")["active_flows"] == 0)
            self.exercise_isolation(host, gateway, provider)
            self.exercise_pending_close(host, gateway, provider, consumer, target)
        except BaseException:
            failures = []
            self.record("controller", "gateway_phase_diagnostic", {"phases": dict(gateway.phases),
                        "last_denial": gateway.last_denial, "independent_target_requests": len(target.requests)})
            try:
                state = host.evaluate("({ready:document.readyState,bootstrap:location.pathname.startsWith('/__miopunch_open/'),contentType:document.contentType,hasResource:!!document.getElementById('resource')})")
                self.record("controller", "actual_content_diagnostic", state)
            except BaseException as failure:
                failures.append(type(failure).__name__)
            self.record("controller", "diagnostic_failures", {"failures": failures})
            raise

    def exercise_isolation(self, host, gateway, provider):
        escape = HttpTarget(self)
        canary = self.private / "owned-management-canary"
        canary.write_text("owned M119 private management canary")
        before = provider.call("web_status")
        arguments = json.dumps({"file": canary.as_uri(), "http": escape.url, "socket": escape.socket_url})
        host.evaluate("""((arguments)=>{
            const result=window.isolationResult={done:false};
            result.noNode=typeof window.node==='undefined';
            result.noNativeIPC=typeof window.__TAURI__==='undefined'&&typeof window.__TAURI_INTERNALS__==='undefined';
            try{window.webkit.messageHandlers.miopunch.postMessage({op:'disable_web'});result.nativeCallDenied=false;}
            catch{result.nativeCallDenied=true;}
            try{result.parentNodeDenied=typeof window.parent.node==='undefined';}catch{result.parentNodeDenied=true;}
            try{result.parentStorageDenied=window.parent===window?true:!window.parent.localStorage.getItem('M119-management-canary');}
            catch{result.parentStorageDenied=true;}
            result.managementStorageAbsent=localStorage.getItem('M119-management-canary')===null;
            result.managementCookieAbsent=!document.cookie.includes('M119-management-cookie=');
            window.parent.postMessage({op:'disable_web',id:'M119-content-attempt'},'*');
            const denied=url=>fetch(url,{credentials:'include'}).then(()=>false,()=>true);
            const idb=new Promise(resolve=>{
                const request=indexedDB.open('M119-management-private',1);
                request.onerror=()=>resolve(false);
                request.onsuccess=()=>{result.managementIndexedDBAbsent=!request.result.objectStoreNames.contains('secret');request.result.close();resolve(true);};
            });
            const socket=new Promise(resolve=>{try{const value=new WebSocket(arguments.socket);value.onopen=()=>{value.close();resolve(false);};value.onerror=()=>resolve(true);}catch{resolve(true);}});
            const frame=document.createElement('iframe');frame.src=arguments.http;document.body.append(frame);
            const image=document.createElement('img');image.src=arguments.http+'pixel.svg';document.body.append(image);
            Promise.all([denied(arguments.file),denied(arguments.http),socket,idb,
                fetch('/__miopunch_manage',{method:'POST',body:'{"op":"disable_web"}'}).then(r=>r.status===404)
            ]).then(values=>{[result.fileDenied,result.externalFetchDenied,result.externalSocketDenied,result.contentIDBWorked,result.managementRPCDenied]=values;result.done=true;});
            return true;
        })(""" + arguments + ")")
        self.until(lambda: (host.evaluate("window.isolationResult||null") or {}).get("done"), "actual renderer isolation attempts did not settle")
        result = host.evaluate("window.isolationResult")
        assert all(result.values()), result
        assert escape.requests == [] and escape.connections == 0
        assert canary.read_text() == "owned M119 private management canary"
        after = provider.call("web_status")
        assert after["enabled"] and before["origins"] == after["origins"]
        self.record("controller", "actual_content_isolation", {"platform": host.platform, "attempts": result,
                    "fresh_management_file_unchanged": True, "direct_http_ws_frame_image_no_target_effect": True,
                    "provider_configuration_unchanged": True})

    def exercise_pending_close(self, host, gateway, provider, consumer, target):
        host.evaluate("(window.pendingFetchOutcome=null,fetch('/pending').then(r=>{window.pendingFetchOutcome=!r.ok;},()=>{window.pendingFetchOutcome=true;}),true)")
        self.until(target.pending_started.is_set, "real content did not reach pending upstream HTTP")
        started = time.monotonic()
        cleanup = gateway.close()
        self.record("controller", "content_gateway_early_close", cleanup)
        self.nodes.remove(gateway)
        assert time.monotonic() - started < 5, "pending content close exceeded owner bound"
        self.until(lambda: consumer.call("web_status")["active_flows"] == 0 and provider.call("web_status")["active_flows"] == 0,
                   "content close left module owners")
        self.until(lambda: host.evaluate("window.pendingFetchOutcome===true"), "content received false success after pending close")
        self.record("controller", "actual_content_pending_close", {"platform": host.platform,
                    "target_observed_pending": True, "close_seconds": time.monotonic()-started, "all_module_owners_joined": True})

    def gateway_safety(self, gateway, target):
        address = urlsplit(gateway.address)

        def call(method, path, headers=None):
            connection = http.client.HTTPSConnection(address.hostname, address.port, timeout=5,
                                                     context=ssl.create_default_context(cafile=gateway.certificate))
            try:
                connection.request(method, path, headers=headers or {})
                response = connection.getresponse()
                status = response.status
                response.read(4096)
                return status
            finally:
                connection.close()

        # This is the actual absolute-form/Host shape observed from WebKit.
        assert call("GET", gateway.bootstrap, {"Host": address.hostname}) == 303
        before = len(target.requests)
        cookie = "__miopunch_content_session=" + gateway.token
        outcomes = []
        for name, method, path, headers, expected in [
            ("missing_cookie", "GET", "/page", {}, 403),
            ("wrong_host", "GET", "/page", {"Host": "unowned.invalid", "Cookie": cookie}, 403),
            ("other_absolute_origin", "GET", target.url + "page", {"Host": address.hostname, "Cookie": cookie}, 403),
            ("wrong_origin", "GET", "/page", {"Cookie": cookie, "Origin": "http://unowned.invalid"}, 403),
            ("opaque_origin", "POST", "/login", {"Cookie": cookie, "Origin": "null"}, 403),
            ("management_rpc", "GET", "/__miopunch_manage", {"Cookie": cookie}, 404),
            ("connect", "CONNECT", address.netloc, {"Cookie": cookie}, 403),
        ]:
            outcomes.append(dict(case=name, actual=call(method, path, headers), expected=expected))
        self.record("controller", "gateway_negative_outcomes", {"outcomes": outcomes, "target_requests_before": before,
                    "target_requests_after": len(target.requests)})
        assert all(row["actual"] == row["expected"] for row in outcomes), outcomes
        assert len(target.requests) == before, "denied gateway entry reached provider target"
        self.record("controller", "actual_gateway_boundaries", {"absolute_form_host_without_port": True,
                    "missing_cookie_denied": True, "wrong_host_denied": True, "other_absolute_origin_denied": True,
                    "wrong_origin_denied": True, "no_management_rpc": True, "connect_denied": True, "target_no_effect": True})

    def exercise_content(self, host, target, gateway, modules_settled):
        def loaded():
            state = host.evaluate("({ready:document.readyState,fetch:window.fixtureFetch||null})")
            return state is not None and state.get("fetch") == "owned M119 fetched resource"
        self.until(loaded, "actual content script/fetch did not complete")
        result = host.evaluate("({css:getComputedStyle(document.getElementById('resource')).color,image:document.getElementById('image').naturalWidth})")
        assert result == {"css": "rgb(12, 34, 56)", "image": 2}, result
        assert all(path in target.requests for path in ["/page", "/app.js", "/style.css", "/pixel.svg", "/api"])
        self.record("controller", "actual_content_resources", {"actual_container": host.platform, "script_fetch_exact": True,
                    "stylesheet_applied": True, "image_decoded": True, "provider_target_observed_all": True})
        host.evaluate("(document.getElementById('login').submit(),true)")
        self.until(lambda: host.evaluate("!!document.getElementById('authenticated')"), "real form POST/Cookie/303 navigation failed")
        assert target.authenticated_reads == 1 and "/login" in target.requests and "/private" in target.requests
        host.evaluate("(document.getElementById('next').click(),true)")
        self.until(lambda: host.evaluate("!!document.getElementById('navigated')"), "real navigation did not complete")
        assert "/next" in target.requests
        self.record("controller", "actual_content_login_navigation", {"form_post": True, "cookie_received_by_target": True,
                    "redirect_303": True, "navigation": True})
        host.evaluate("""(()=>{
            window.socketResult={greeting:false,text:false,binary:false,closed:false,error:false};
            const socket=new WebSocket(location.origin.replace(/^http/,'ws')+'/socket?authenticated=1');
            socket.binaryType='arraybuffer';
            socket.onmessage=event=>{
                if(event.data==='owned M119 ready'){window.socketResult.greeting=true;socket.send('M119 container text');}
                else if(event.data==='owned-target:M119 container text'){window.socketResult.text=true;socket.send(Uint8Array.from({length:37},(_,i)=>i));}
                else if(event.data instanceof ArrayBuffer){const bytes=new Uint8Array(event.data);window.socketResult.binary=bytes.length===37&&bytes.every((v,i)=>v===i);socket.close(1000);}
            };
            socket.onerror=()=>window.socketResult.error=true;
            socket.onclose=event=>{window.socketResult.closed=true;window.socketResult.code=event.code;window.socketResult.wasClean=event.wasClean;};
            return true;
        })()""")
        self.until(lambda: host.evaluate("socketResult.greeting&&socketResult.text&&socketResult.binary&&socketResult.closed&&!socketResult.error&&socketResult.code===1000&&socketResult.wasClean"),
                   "actual container WebSocket greeting/text/binary/close failed")
        self.until(lambda: gateway.socket_close_results and modules_settled() and not target.active,
                   "normal content close retained a module or independent target owner")
        assert gateway.socket_close_results == [{"closed":True,"failed":False}], gateway.socket_close_results
        assert target.observed[1:] == [("text", "M119 container text"), ("binary", bytes(range(37)))]
        assert target.authenticated_sockets == 1
        self.record("controller", "actual_content_websocket", {"native_websocket_api": True, "unsolicited_greeting": True,
                    "text_and_binary_exact": True, "login_cookie_reached_socket_target": True, "target_independently_observed": True,
                    "normal_close": True, "content_close_code":1000, "content_wasClean":True,
                    "SDK_failed":False, "module_and_target_owners_settled":True, "target_tls": bool(target.tls)})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="container")
    ContainerRunner(parser.parse_args()).run()
