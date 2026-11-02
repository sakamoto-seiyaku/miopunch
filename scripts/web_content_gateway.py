#!/usr/bin/env python3
"""Minimal content gateway for an already admitted Web-service consumer.

The callback is bound to one provider and target origin by the host. HTTP never
accepts an operation name, provider identity, node directory or management RPC.
This is technical integration for M119, not a final product UI/server.
"""
import asyncio
import base64
from concurrent.futures import ThreadPoolExecutor
from http.cookies import SimpleCookie
import secrets
import threading
import time
from urllib.parse import urlsplit, urlunsplit

from aiohttp import WSMsgType, web
from web_asyncio_observer import OwnedLoopObserver
from web_tls_fixture import server_certificate
from web_process_owner import wait_future

MAX_BODY = 1024 * 1024
MAX_OPERATIONS = 8
SESSION_COOKIE = "__miopunch_content_session"


def origin(url):
    value = urlsplit(url)
    if value.scheme not in {"http", "https"} or not value.hostname or value.username or value.password:
        raise ValueError("content target requires an HTTP(S) origin")
    port = value.port or (443 if value.scheme == "https" else 80)
    host = "[%s]" % value.hostname if ":" in value.hostname else value.hostname
    return "%s://%s%s" % (value.scheme, host, "" if port == (443 if value.scheme == "https" else 80) else ":%d" % port)


class ContentGateway:
    def __init__(self, target, request, owner_directory, owners, socket_callbacks=None, browser_origins=False, http_callbacks=None):
        self.target, self.target_origin, self.request = target, origin(target), request
        self.private = owner_directory
        self.socket_callbacks = socket_callbacks
        self.http_callbacks, self.http_jobs, self.stopping = http_callbacks, set(), False
        realm = secrets.token_hex(8)
        self.content_host = "content-" + realm + ".m119.test" if browser_origins else "127.0.0.1"
        self.management_host = "management-" + realm + ".m119.test" if browser_origins else None
        self.key_path = self.private / "content-gateway-key.pem"
        self.tls, self.certificate, self.trust_root_pem, self.spki = server_certificate(self.private, "content-gateway",
            [self.content_host, self.management_host] if browser_origins else [])
        self.token = secrets.token_urlsafe(32)
        self.errors, self.requests = [], 0
        self.socket_close_results = []
        self.record = lambda operation, result: None
        self.cleanup_phase = "running"
        self.phases = {}
        self.last_denial = None
        self.loop = asyncio.new_event_loop()
        self.observer = OwnedLoopObserver(self.loop, "gateway:" + self.private.name, self.errors,
                                          lambda row: self.record("owned_asyncio", row))
        self.executor = ThreadPoolExecutor(max_workers=MAX_OPERATIONS, thread_name_prefix="m119-web-callback")
        self.loop.set_default_executor(self.executor)
        self.started = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.closed = False
        self.cleanup, self.close_error, self.close_started = None, None, False
        owners.append(self)
        self.thread.start()
        assert self.started.wait(10) and not self.errors, "content gateway failed to start"

    def run(self):
        asyncio.set_event_loop(self.loop)
        try:
            self.observer.phase("starting")
            self.loop.run_until_complete(self.start())
            self.started.set()
            self.observer.phase("running")
            self.loop.run_forever()
        except BaseException as failure:
            self.errors.append(type(failure).__name__)
            self.started.set()
        finally:
            self.observer.phase("shutdown_asyncgens")
            self.loop.run_until_complete(self.loop.shutdown_asyncgens())
            self.observer.phase("loop_close")
            self.loop.close()
            self.observer.phase("loop_closed")

    async def start(self):
        self.capacity = asyncio.Semaphore(MAX_OPERATIONS)
        @web.middleware
        async def no_tunnel(request, handler):
            # CONNECT uses authority-form rather than a normal route path.
            # Reject it before routing can turn it into an unrelated 404.
            if request.method == "CONNECT":
                raise web.HTTPForbidden()
            return await handler(request)
        application = web.Application(client_max_size=MAX_BODY, middlewares=[no_tunnel],
                                      handler_args={"max_line_size": 8192, "max_field_size": 8192})
        application.router.add_route("*", "/{path:.*}", self.handle)
        self.runner = web.AppRunner(application, access_log=None, shutdown_timeout=1)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, "127.0.0.1", 0, ssl_context=self.tls)
        await self.site.start()
        self.observer.server("https", self.site._server)
        self.address = "https://%s:%d" % (self.content_host, self.site._server.sockets[0].getsockname()[1])
        self.proxy_site = web.TCPSite(self.runner, "127.0.0.1", 0)
        await self.proxy_site.start()
        self.observer.server("http-proxy", self.proxy_site._server)
        self.proxy_address = "http://127.0.0.1:%d" % self.proxy_site._server.sockets[0].getsockname()[1]
        self.bootstrap = self.address + "/__miopunch_open/" + self.token

    def authenticated(self, request):
        return secrets.compare_digest(request.cookies.get(SESSION_COOKIE, ""), self.token)

    async def handle(self, request):
        def phase(name):
            self.phases[name] = self.phases.get(name, 0) + 1
        phase("received")
        # WebKit sends absolute-form HTTP proxy requests with Host containing
        # the IP alone. Validate the complete request-target authority as well
        # as Host; never reinterpret an absolute request for another origin.
        expected = urlsplit(self.address)
        try:
            host = urlsplit("//" + request.host)
            absolute = urlsplit(request.raw_path)
            valid_host = host.hostname == expected.hostname and host.port in {None, expected.port} and not host.username and not host.password
            valid_target = not absolute.scheme and not absolute.netloc or origin(request.raw_path) == self.address
        except ValueError:
            valid_host, valid_target = False, False
        if not valid_host or not valid_target or request.method == "CONNECT":
            phase("host_or_connect_denied")
            self.last_denial = {"method": request.method, "host": request.host,
                                "expected_host": urlsplit(self.address).netloc,
                                "absolute_http_request": request.raw_path.startswith(("http://", "https://"))}
            raise web.HTTPForbidden()
        if request.path.startswith("/__miopunch_open/"):
            if request.path != "/__miopunch_open/" + self.token or request.method != "GET":
                raise web.HTTPForbidden()
            phase("bootstrap_accepted")
            path = urlsplit(self.target)
            response = web.HTTPSeeOther(path.path + ("?" + path.query if path.query else ""))
            response.set_cookie(SESSION_COOKIE, self.token, httponly=True, secure=True, samesite="Strict", path="/")
            response.headers["Cache-Control"] = "no-store"
            raise response
        if not self.authenticated(request):
            phase("session_cookie_denied")
            raise web.HTTPForbidden()
        if request.headers.get("Origin", self.address) != self.address:
            phase("origin_denied")
            raise web.HTTPForbidden()
        if request.path.startswith("/__miopunch_"):
            # No generic host RPC is made available to opened content.
            raise web.HTTPNotFound()
        if self.capacity.locked():
            raise web.HTTPServiceUnavailable()
        async with self.capacity:
            if request.headers.get("Upgrade", "").lower() == "websocket":
                return await self.socket(request)
            body = await request.read()
            if len(body) > MAX_BODY:
                raise web.HTTPRequestEntityTooLarge(max_size=MAX_BODY, actual_size=len(body))
            headers = []
            for name, value in request.headers.items():
                lower = name.lower()
                if lower in {"host", "content-length", "connection", "origin", "referer", "accept-encoding"} or lower.startswith("sec-"):
                    continue
                if lower == "cookie":
                    cookies = SimpleCookie()
                    cookies.load(value)
                    cookies.pop(SESSION_COOKIE, None)
                    value = "; ".join(item.OutputString() for item in cookies.values())
                    if not value:
                        continue
                headers.append((name, value))
            if "Origin" in request.headers:
                headers.append(("Origin", self.target_origin))
            target = self.target_origin + request.path_qs
            self.requests += 1
            phase("callback_started")
            try:
                result = await self.fetch(dict(
                    url=target, method=request.method, headers=headers, body=base64.b64encode(body).decode()))
                payload = base64.b64decode(result["body"], validate=True)
                if len(payload) > 8 * 1024 * 1024:
                    raise ValueError("Web response exceeds module limit")
            except BaseException:
                phase("callback_failed")
                # Upstream errors may contain URLs or credentials. The content
                # gets a stable failure, and no exception text enters a log.
                raise web.HTTPBadGateway(text="Web service request failed") from None
            response = web.Response(status=result["status"], body=payload)
            for name, value in result["headers"]:
                lower = name.lower()
                if lower in {"content-length", "connection", "transfer-encoding", "content-security-policy", "content-security-policy-report-only"}:
                    continue
                if lower == "location":
                    destination = urlsplit(value)
                    if destination.scheme or destination.netloc:
                        if origin(value) != self.target_origin:
                            raise web.HTTPBadGateway(text="Open the other origin in a separate Web session")
                        value = urlunsplit(("", "", destination.path or "/", destination.query, destination.fragment))
                    elif value.startswith("//"):
                        raise web.HTTPBadGateway(text="Cross-origin redirect requires a separate Web session")
                if lower == "set-cookie":
                    cookies = SimpleCookie()
                    cookies.load(value)
                    for key, cookie in cookies.items():
                        if key == SESSION_COOKIE:
                            continue
                        cookie["domain"] = ""
                        # HTTPS content hosting retains upstream Secure.
                        response.headers.add("Set-Cookie", cookie.OutputString())
                    continue
                response.headers.add(name, value)
            # An opened page cannot escape to direct network resources or
            # frames. Navigation is additionally restricted by the real host.
            response.headers["Content-Security-Policy"] = (
                "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; "
                "style-src 'self' 'unsafe-inline'; connect-src 'self'; "
                "form-action 'self'; frame-src 'self'; object-src 'none'; base-uri 'self'")
            # no-referrer makes navigate-mode POST Origin:null under Fetch,
            # which correctly fails our strict Origin check. strict-origin
            # retains the same-origin form Origin without disclosing a path.
            response.headers["Referrer-Policy"] = "strict-origin"
            response.headers["X-Content-Type-Options"] = "nosniff"
            response.headers["Cache-Control"] = "no-store"
            phase("delivered")
            return response

    async def invoke(self, callback, *arguments):
        return await asyncio.get_running_loop().run_in_executor(self.executor, callback, *arguments)

    async def fetch(self, arguments):
        if self.stopping:
            raise RuntimeError("content session is stopping")
        if self.http_callbacks is None:
            return await self.invoke(self.request, arguments)
        callbacks = self.http_callbacks
        identifier = (await self.invoke(callbacks["begin"], arguments))["id"]
        self.http_jobs.add(identifier)
        try:
            while not self.stopping:
                result = await self.invoke(callbacks["result"], identifier)
                if not result["pending"]:
                    self.http_jobs.discard(identifier)
                    return result["response"]
                await asyncio.sleep(0.02)
            raise RuntimeError("content session stopped")
        finally:
            if identifier in self.http_jobs:
                self.http_jobs.discard(identifier)
                await self.invoke(callbacks["cancel"], identifier)

    async def shutdown(self):
        self.cleanup_phase = "cancelling_requests"
        self.observer.phase(self.cleanup_phase)
        self.stopping = True
        failures = []
        # Cancel module owners before joining HTTP handlers/executor threads.
        # Only handles created by this gateway are present in this set.
        for identifier in list(self.http_jobs):
            self.http_jobs.discard(identifier)
            try:
                await self.invoke(self.http_callbacks["cancel"], identifier)
            except Exception as failure:
                failures.append(type(failure).__name__)
        self.cleanup_phase = "app_runner_cleanup"
        self.observer.phase(self.cleanup_phase)
        await self.runner.cleanup()
        await self.observer.finish_servers()
        self.cleanup_phase = "ssl_callback_drain"
        self.observer.phase(self.cleanup_phase)
        # Let SSL protocol connection_lost callbacks retire their transports
        # while this private loop is still alive, before loop.close().
        await asyncio.sleep(0.25)
        self.cleanup_phase = "shutdown_complete"
        self.observer.phase(self.cleanup_phase)
        if failures:
            raise RuntimeError("content request cancellation failed")

    async def socket(self, request):
        if self.socket_callbacks is None:
            raise web.HTTPServiceUnavailable()
        callbacks = self.socket_callbacks
        target = ("wss" if self.target_origin.startswith("https:") else "ws") + self.target_origin[self.target_origin.index(":"):] + request.path_qs
        headers = []
        for name, value in request.headers.items():
            if name.lower() == "cookie":
                cookies = SimpleCookie()
                cookies.load(value)
                cookies.pop(SESSION_COOKIE, None)
                value = "; ".join(item.OutputString() for item in cookies.values())
                if value:
                    headers.append(("Cookie", value))
            elif name.lower() == "sec-websocket-protocol":
                headers.append((name, value))
        headers.append(("Origin", self.target_origin))
        identifier = None
        front = None
        tasks = []
        try:
            opened = await self.invoke(callbacks["open"], target, headers)
            identifier = opened["id"]
            protocols = [value for name, value in opened.get("headers", []) if name.lower() == "sec-websocket-protocol"]
            front = web.WebSocketResponse(protocols=protocols, max_msg_size=32768)
            await front.prepare(request)

            async def to_provider():
                async for message in front:
                    if message.type == WSMsgType.TEXT:
                        await self.invoke(callbacks["send"], identifier, {"type": "text", "text": message.data})
                    elif message.type == WSMsgType.BINARY:
                        await self.invoke(callbacks["send"], identifier, {"type": "binary", "data": base64.b64encode(message.data).decode()})

            async def to_content():
                while not front.closed:
                    result = await self.invoke(callbacks["read"], identifier)
                    if result.get("pending"):
                        continue
                    message = result.get("message")
                    if message and message["type"] == "text":
                        await front.send_str(message["text"])
                    elif message and message["type"] == "binary":
                        await front.send_bytes(base64.b64decode(message["data"], validate=True))
                    elif result.get("closed"):
                        code = message.get("code", 1000) if message else (1011 if result.get("failed", True) else 1000)
                        if code in {1004, 1005, 1006, 1015}:
                            code = 1000
                        await front.close(code=code)
                        return
            tasks = [asyncio.create_task(to_provider()), asyncio.create_task(to_content())]
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in tasks:
                if task.done():
                    task.result()
        except Exception:
            if front is None:
                raise web.HTTPBadGateway(text="WebSocket service opening failed") from None
            await front.close(code=1011)
        finally:
            for task in tasks:
                if not task.done():
                    task.cancel()
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
            if identifier is not None:
                try:
                    result = await self.invoke(callbacks["close"], identifier)
                    self.socket_close_results.append(result)
                    self.record("SDK_socket_closed", result)
                except Exception:
                    self.errors.append("WebSocketOwnerClose")
        return front

    def close(self):
        if self.close_started:
            if self.close_error is not None:
                raise self.close_error
            if self.cleanup is not None:
                return self.cleanup
            raise RuntimeError("original content gateway close outcome is unsettled; no rescheduling")
        self.close_started = True
        failures = []
        future = None
        started, wall_started = time.monotonic(), time.time()
        try:
            if self.loop.is_closed():
                raise RuntimeError("original content gateway loop already closed before shutdown")
            coroutine = self.shutdown()
            try:
                future = asyncio.run_coroutine_threadsafe(coroutine, self.loop)
            except BaseException:
                coroutine.close()
                raise
            wait_future(future, 10)
        except BaseException as failure:
            failures.append(type(failure).__name__)
        try:
            self.record("gateway_shutdown_future", {"phase": self.cleanup_phase,
                        "wait_elapsed":time.monotonic()-started, "wall_elapsed":time.time()-wall_started, "wait_timeout":10,
                        "future_done": future.done() if future else None, "future_cancelled": future.cancelled() if future else None,
                        "errors": list(failures), "loop_running": self.loop.is_running(), "loop_closed": self.loop.is_closed()})
        except BaseException as failure:
            failures.append("CloseRecord:" + type(failure).__name__)
        try:
            if not self.loop.is_closed():
                self.loop.call_soon_threadsafe(self.loop.stop)
        except BaseException as failure:
            failures.append("LoopStop:" + type(failure).__name__)
        self.thread.join(timeout=5)
        self.executor.shutdown(wait=True, cancel_futures=True)
        self.cleanup = {"owner": "content-gateway", "phase": self.cleanup_phase, "thread_joined": not self.thread.is_alive(),
                        "errors": failures + self.errors, "loop_closed": self.loop.is_closed(),
                        "executor_threads_alive": sum(thread.is_alive() for thread in self.executor._threads),
                        "owned_asyncio": self.observer.summary()}
        if self.thread.is_alive() or failures or self.errors:
            self.close_error = RuntimeError("content gateway cleanup failed")
            raise self.close_error
        self.closed = True
        return self.cleanup

    @property
    def reader(self):
        return self.thread

    def terminal(self):
        return self.closed and not self.thread.is_alive()
