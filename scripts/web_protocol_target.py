#!/usr/bin/env python3
"""One owned HTTP(S)/WebSocket protocol sample, independent of the SDK.

Known literals and target-side request/message observations are test fixtures,
not a product site whitelist or an arbitrary-site compatibility promise.
"""
import asyncio
import gzip
import threading

from aiohttp import ClientSession, WSMsgType, web

from web_tls_fixture import server_certificate
from web_process_owner import wait_future


class WebTarget:
    def __init__(self, runner, tls=False):
        self.loop = asyncio.new_event_loop()
        self.record = runner.record
        self.started = threading.Event()
        self.requests, self.observed = [], []
        self.connections, self.closed, self.authenticated_reads = 0, 0, 0
        self.authenticated_sockets = 0
        self.active, self.errors = set(), []
        self.pending_started, self.pending_release = threading.Event(), threading.Event()
        self.pressure_paused = threading.Event()
        self.pressure_release = runner.private / "socket-pressure-release"
        self.pressure_lengths = []
        self.tls, self.certificate, self.trust_root_pem = None, None, None
        if tls:
            self.tls, self.certificate, self.trust_root_pem, self.spki = server_certificate(runner.private, "target")
        self.reader = threading.Thread(target=self.run, daemon=True)
        runner.targets.append(self)
        self.reader.start()
        assert self.started.wait(5) and not self.errors

    def run(self):
        asyncio.set_event_loop(self.loop)
        try:
            self.loop.run_until_complete(self.start())
            self.started.set()
            self.loop.run_forever()
        except BaseException as failure:
            self.errors.append(type(failure).__name__)
            self.started.set()
        finally:
            self.loop.run_until_complete(self.loop.shutdown_asyncgens())
            self.loop.close()

    async def socket(self, request):
        if request.query.get("authenticated") == "1":
            if request.cookies.get("session") != "m119-owned":
                raise web.HTTPForbidden()
            self.authenticated_sockets += 1
        response = web.WebSocketResponse(max_msg_size=32768)
        await response.prepare(request)
        self.active.add(response)
        self.connections += 1
        pressure = request.query.get("pressure") == "1"
        transport = request.transport
        if pressure:
            # This fresh target deliberately leaves its receive window full.
            # It sends no replies to pressure frames and never affects a host
            # socket. The controller releases it after observing explicit close.
            transport.pause_reading()
            self.pressure_paused.set()
        await response.send_str("owned M119 ready")
        try:
            if pressure:
                while not self.pressure_release.exists():
                    await asyncio.sleep(.02)
                transport.resume_reading()
            async for message in response:
                if pressure:
                    if message.type in {WSMsgType.TEXT, WSMsgType.BINARY}:
                        self.pressure_lengths.append(len(message.data))
                    elif message.type == WSMsgType.ERROR:
                        # An aborted pressured connection is an expected
                        # terminal observation, not an echo payload.
                        self.record("controller", "pressure_target_terminal_error", {"type":type(message.data).__name__})
                    continue
                if message.type == WSMsgType.TEXT:
                    self.observed.append(("text", message.data))
                    await response.send_str("owned-target:" + message.data)
                elif message.type == WSMsgType.BINARY:
                    self.observed.append(("binary", bytes(message.data)))
                    await response.send_bytes(message.data)
        finally:
            self.active.discard(response)
            self.closed += 1
        return response

    async def handle(self, request):
        self.requests.append(request.path)
        if request.path == "/pending-stream":
            # A bounded active response lets all queued contact handshakes
            # finish without the fixture's silent-response read timeout.
            response = web.StreamResponse(status=200)
            await response.prepare(request)
            self.pending_started.set()
            for _ in range(60):
                if self.pending_release.is_set() or request.transport is None or request.transport.is_closing():
                    break
                try:
                    await response.write(b".")
                except ConnectionResetError:
                    break
                await asyncio.sleep(1)
            return response
        if request.path == "/pending":
            self.pending_started.set()
            while not self.pending_release.is_set():
                if request.transport is None or request.transport.is_closing():
                    return web.Response(status=499)
                await asyncio.sleep(0.02)
            return web.Response(body=b"owned M119 pending completed")
        if request.path == "/socket" and request.headers.get("Upgrade", "").lower() == "websocket":
            return await self.socket(request)
        if request.method == "POST":
            body = await request.read()
            if request.path != "/login" or body != b"password=owned-password":
                return web.Response(status=400)
            response = web.Response(status=303, headers={"Location": "/private"})
            response.set_cookie("session", "m119-owned", httponly=True, secure=self.tls is not None, samesite="Strict", path="/")
            return response
        if request.path == "/page":
            return web.Response(content_type="text/html", body=b'''<!doctype html><link rel="stylesheet" href="/style.css"><script src="/app.js" defer></script>
                <p id="resource">M119 resource</p><img id="image" src="/pixel.svg">
                <form id="login" method="POST" action="/login"><input name="password" value="owned-password"><button>Login</button></form>''')
        if request.path == "/style.css":
            return web.Response(content_type="text/css", text="#resource { color: rgb(12, 34, 56); }")
        if request.path == "/app.js":
            return web.Response(content_type="text/javascript", text="fetch('/api').then(r=>r.text()).then(t=>window.fixtureFetch=t);")
        if request.path == "/api":
            return web.Response(body=b"owned M119 fetched resource")
        if request.path == "/gzip":
            return web.Response(body=gzip.compress(b"owned M119 compressed resource"), headers={"Content-Encoding": "gzip", "Content-Type": "text/plain"})
        if request.path == "/pixel.svg":
            return web.Response(content_type="image/svg+xml", body=b'<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>')
        if request.path == "/private":
            if request.cookies.get("session") != "m119-owned":
                return web.Response(status=401, text="not authenticated")
            self.authenticated_reads += 1
            return web.Response(content_type="text/html", text='<p id="authenticated">owned authenticated page</p><a id="next" href="/next">next</a>')
        if request.path == "/next":
            return web.Response(content_type="text/html", text='<p id="navigated">owned navigation complete</p>')
        return web.Response(body=b"owned M119 web target")

    async def start(self):
        application = web.Application(client_max_size=1024 * 1024)
        application.router.add_route("*", "/{path:.*}", self.handle)
        async def stop(_application):
            await asyncio.gather(*(socket.close(code=1001) for socket in list(self.active)))
        application.on_shutdown.append(stop)
        # aiohttp gives a busy handler two separate graceful waits. Keep both
        # inside this fixture's unchanged five-second total close assertion.
        self.runner = web.AppRunner(application, access_log=None, shutdown_timeout=1)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, "127.0.0.1", 0, ssl_context=self.tls)
        await self.site.start()
        port = self.site._server.sockets[0].getsockname()[1]
        self.url = ("https" if self.tls else "http") + "://127.0.0.1:%d/" % port
        self.socket_url = ("wss" if self.tls else "ws") + "://127.0.0.1:%d/socket" % port

    async def socket_prerequisite(self):
        import ssl
        verification = ssl.create_default_context(cafile=self.certificate) if self.certificate else None
        async with ClientSession(trust_env=False) as session:
            async with session.ws_connect(self.socket_url, max_msg_size=32768, ssl=verification or True) as socket:
                assert (await socket.receive_str(timeout=2)) == "owned M119 ready"
                await socket.send_str("M119 direct target prerequisite")
                assert (await socket.receive_str(timeout=2)) == "owned-target:M119 direct target prerequisite"

    def close(self):
        self.pending_release.set()
        self.pressure_release.touch()
        async def shutdown():
            self.record("controller", "target_cleanup_begin", {"active_websockets": len(self.active)})
            await self.runner.cleanup()
            await asyncio.sleep(0.25)
            self.record("controller", "target_cleanup_complete", {"active_websockets": len(self.active)})
        future = asyncio.run_coroutine_threadsafe(shutdown(), self.loop)
        wait_future(future, 5)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.reader.join(timeout=5)
        assert not self.reader.is_alive() and not self.errors and not self.active
        return {"owner": "web-protocol-target", "reader_joined": True, "active_websockets": 0, "errors": []}
