#!/usr/bin/env python3
"""Actual Windows renderer assertions derived from the retained container checks."""
import base64
import http.client
import json
import secrets
import ssl
import time
from urllib.parse import urlsplit

from web_content_gateway import ContentGateway
from web_protocol_target import WebTarget
from web_windows_content_runtime import WindowsContentHost


class WindowsContentChecks:
    def __init__(self, runner):
        self.runner = runner
        self.record, self.until = runner.record, runner.until
        self.private, self.targets, self.nodes = runner.private, runner.targets, runner.gateways

    def exercise(self, host, target, gateway, provider, consumer):
        self.gateway_safety(gateway, target)
        self.exercise_content(host, target, gateway, lambda: consumer.call("web_status")["active_flows"] == 0
                              and provider.call("web_status")["active_flows"] == 0)
        self.exercise_isolation(host, gateway, provider)
        self.exercise_pending_close(host, gateway, provider, consumer, target)

    def exercise_isolation(self, host, gateway, provider):
        # Seed actual separate renderer storage before testing its absence.
        # These are fresh synthetic controls, never a user's management data.
        canary = "M119-owned-management-" + secrets.token_hex(12)
        management_file = self.private / "owned-management-canary"
        management_file.write_text(canary)
        control_private = self.private / "management-control-gateway"
        control_private.mkdir()
        attempts = []
        page = b"<!doctype html><title>M119 owned management control</title><p id='control'>owned control</p>"

        def management_request(arguments):
            if arguments["url"].endswith("/attempt"):
                attempts.append(arguments["method"])
            return dict(status=200, headers=[["Content-Type", "text/html"]],
                        body=base64.b64encode(page).decode())

        management = ContentGateway("https://127.0.0.1:9/control", management_request,
                                    control_private, self.runner.gateways)
        control = WindowsContentHost(self.runner, management, label="management-control")
        self.until(lambda: control.evaluate("!!document.getElementById('control')"),
                   "owned management control page did not load")
        assert control.profile != host.profile and management.address != gateway.address
        seed = json.dumps(canary)
        control.evaluate("""((value)=>{
            window.managementControlReady=false;
            localStorage.setItem('M119-management-canary',value);
            document.cookie='M119-management-cookie='+value+';Secure;SameSite=Strict;Path=/';
            const open=indexedDB.open('M119-management-private',1);
            open.onupgradeneeded=()=>open.result.createObjectStore('secret');
            open.onsuccess=()=>{
                const db=open.result;
                const transaction=db.transaction('secret','readwrite');
                transaction.objectStore('secret').put(value,'control');
                transaction.oncomplete=()=>{
                    const read=db.transaction('secret').objectStore('secret').get('control');
                    read.onsuccess=()=>{window.managementControlValue=read.result;window.managementControlReady=true;db.close();};
                };
            };
            return true;
        })(""" + seed + ")")
        self.until(lambda: control.evaluate("window.managementControlReady===true"),
                   "positive management IndexedDB control not seeded")
        control_expression = "({local:localStorage.getItem('M119-management-canary'),cookie:document.cookie.includes('M119-management-cookie=' + " + seed + "),idb:window.managementControlValue})"
        assert control.evaluate(control_expression) == dict(local=canary, cookie=True, idb=canary)
        self.record("controller", "positive_management_storage_controls", dict(separate_actual_renderer=True,
            different_origin=True, different_profile=True, localStorage_seeded=True, Cookie_seeded=True,
            IndexedDB_seeded_and_read=True, synthetic_file_present=True))
        escape = WebTarget(self.runner)
        before = provider.call("web_status")
        service_before = provider.call("service_status")
        arguments = json.dumps(dict(file=management_file.as_uri(), http=escape.url,
                                    socket=escape.socket_url, management=management.address + "/attempt"))
        host.evaluate("""((arguments)=>{
            const result=window.isolationResult={done:false};
            result.noNode=typeof window.node==='undefined';
            result.noTauriIPC=typeof window.__TAURI__==='undefined'&&typeof window.__TAURI_INTERNALS__==='undefined';
            result.genericWebViewObject=!!window.chrome?.webview;
            result.nativeIPCError=false;
            try{window.chrome.webview.postMessage({op:'web_disable',id:'M119-owned-content-attempt'});}catch{result.nativeIPCError=true;}
            result.nativeIPCAttemptCompleted=true;
            result.managementHostObjectDenied=false;
            try{const object=window.chrome.webview.hostObjects.sync.miopunch;object.web_disable();}
            catch{result.managementHostObjectDenied=true;}
            try{result.parentNodeDenied=typeof window.parent.node==='undefined';}catch{result.parentNodeDenied=true;}
            result.managementStorageAbsent=localStorage.getItem('M119-management-canary')===null;
            result.managementCookieAbsent=!document.cookie.includes('M119-management-cookie=');
            window.parent.postMessage({op:'web_disable',id:'M119-owned-parent-attempt'},'*');
            const denied=url=>fetch(url,{credentials:'include'}).then(()=>false,()=>true);
            const idb=new Promise(resolve=>{
                const request=indexedDB.open('M119-management-private',1);
                request.onerror=()=>resolve(false);
                request.onsuccess=()=>{result.managementIndexedDBAbsent=!request.result.objectStoreNames.contains('secret');request.result.close();resolve(true);};
            });
            const socket=new Promise(resolve=>{try{const value=new WebSocket(arguments.socket);value.onopen=()=>{value.close();resolve(false);};value.onerror=()=>resolve(true);}catch{resolve(true);}});
            const frame=document.createElement('iframe');frame.src=arguments.http;document.body.append(frame);
            const image=document.createElement('img');image.src=arguments.http+'pixel.svg';document.body.append(image);
            Promise.all([denied(arguments.file),denied(arguments.http),denied(arguments.management),socket,idb,
                fetch('/__miopunch_manage',{method:'POST',body:'{"op":"web_disable"}'}).then(r=>r.status===404)
            ]).then(values=>{[result.fileDenied,result.externalFetchDenied,result.managementOriginDenied,
                result.externalSocketDenied,result.contentIDBWorked,result.managementRPCDenied]=values;result.done=true;});
            return true;
        })(""" + arguments + ")")
        self.until(lambda: (host.evaluate("window.isolationResult||null") or {}).get("done"),
                   "actual Windows content isolation attempts did not settle")
        result = host.evaluate("window.isolationResult")
        required = ["done", "noNode", "noTauriIPC", "nativeIPCAttemptCompleted", "managementHostObjectDenied",
                    "parentNodeDenied", "managementStorageAbsent", "managementCookieAbsent", "managementIndexedDBAbsent",
                    "fileDenied", "externalFetchDenied", "managementOriginDenied", "externalSocketDenied",
                    "contentIDBWorked", "managementRPCDenied"]
        assert all(result.get(name) is True for name in required), result
        # Generic chrome.webview plumbing may exist. Its presence or whether
        # postMessage throws is not a management permission oracle.
        assert escape.requests == [] and escape.connections == 0 and attempts == []
        assert management_file.read_text() == canary
        control.evaluate("""(()=>{
            window.managementControlReady=false;
            const open=indexedDB.open('M119-management-private',1);
            open.onsuccess=()=>{
                const db=open.result;
                const read=db.transaction('secret').objectStore('secret').get('control');
                read.onsuccess=()=>{window.managementControlValue=read.result;window.managementControlReady=true;db.close();};
            };
            return true;
        })()""")
        self.until(lambda: control.evaluate("window.managementControlReady===true"),
                   "positive management IndexedDB control did not reread after content attempts")
        assert control.evaluate(control_expression) == dict(local=canary, cookie=True, idb=canary)
        after = provider.call("web_status")
        assert after["enabled"] and before["origins"] == after["origins"]
        assert provider.call("service_status") == service_before
        self.record("controller", "actual_Windows_content_isolation", dict(attempts=result,
            positive_management_controls_still_intact=True, synthetic_file_unchanged=True,
            direct_owned_http_ws_frame_image_no_effect=True, management_origin_no_effect=True,
            provider_configuration_unchanged=True, actual_native_handlers_declared=False))
        self.record("controller", "management_control_private_close", control.close())

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
