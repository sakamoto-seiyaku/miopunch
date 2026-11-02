#!/usr/bin/env python3
"""Observe an owned fixture loop and settle only its registered servers.

This records callback failures that do not escape run_forever/run_until_complete.
It retains default reporting and never changes loop policy or ignores resets.
"""
import asyncio
import threading
import time


class OwnedLoopObserver:
    def __init__(self, loop, owner, errors, emit):
        if loop.get_exception_handler() is not None:
            raise RuntimeError("owned fixture loop already has an exception handler")
        self.loop, self.owner, self.errors, self.emit = loop, owner, errors, emit
        self.events, self.servers = [], []
        self.failures, self.overflow = 0, 0
        self.current_phase = "created"
        loop.set_exception_handler(self.failed)
        self.phase("created")

    def server(self, name, server):
        if server.get_loop() is not self.loop or len(self.servers) >= 4:
            raise RuntimeError("server is not within this bounded owned loop")
        # Holding the original server does not hold its weak attached clients.
        # No transport object is retained by this observer.
        self.servers.append((name, server))
        self.phase("server_registered")

    def snapshot(self):
        servers = []
        for name, server in self.servers:
            clients = getattr(server, "_clients", None)
            # Pinned CPython3.13 has a WeakSet; 3.12 exposes only a count.
            # These private fields are read-only diagnostic provenance.
            count = len(clients) if clients is not None else getattr(server, "_active_count", None)
            count = count if isinstance(count, int) else None
            servers.append(dict(name=name, server_id=id(server), serving=server.is_serving(),
                                listeners=len(server.sockets or ()), attached_clients=count,
                                client_set_available=clients is not None))
        return dict(loop_id=id(self.loop), loop_class=type(self.loop).__name__,
                    loop_running=self.loop.is_running(), loop_closed=self.loop.is_closed(),
                    servers=servers)

    def save(self, kind, **fields):
        row = dict(tag="M119-OWNED-ASYNCIO-v1", owner=self.owner, kind=kind,
                   phase=self.current_phase, monotonic=time.monotonic(),
                   thread=threading.get_ident(), **self.snapshot(), **fields)
        if len(self.events) < 128:
            self.events.append(row)
            try:
                self.emit(row)
            except BaseException as failure:
                self.errors.append("OwnedLoopRecord:" + type(failure).__name__)
        else:
            self.overflow += 1
            if self.overflow == 1:
                self.errors.append("OwnedLoopObservationBound")

    def phase(self, phase):
        self.current_phase = phase
        self.save("phase")

    async def finish_servers(self):
        if asyncio.get_running_loop() is not self.loop:
            raise RuntimeError("server closure is not running on its original owned loop")
        self.phase("closing_owned_servers")
        for name, server in self.servers:
            server.close()
            close_clients = getattr(server, "close_clients", None)
            if close_clients is not None:
                close_clients()
            elif getattr(server, "_active_count", None) != 0:
                # Older fixture runtimes have no public client-close API. An
                # active client is unqualified there, never silently clean.
                self.errors.append("OwnedServerPublicClientCloseUnavailable")
                raise RuntimeError("fixture runtime cannot close its active server clients")
        await asyncio.gather(*(server.wait_closed() for name, server in self.servers))
        self.phase("owned_servers_closed")
        if any(server["attached_clients"] != 0 or server["listeners"] != 0
               for server in self.snapshot()["servers"]):
            self.errors.append("OwnedServerClosureIncomplete")
            raise RuntimeError("owned server listeners or attached clients remain")

    @staticmethod
    def transport_state(transport):
        if not isinstance(transport, asyncio.BaseTransport):
            return None
        value = dict(transport_id=id(transport), transport_class=type(transport).__name__)
        try:
            closing = transport.is_closing()
            value["closing"] = closing if isinstance(closing, bool) else None
            called = getattr(transport, "_called_connection_lost", None)
            value["connection_lost_finished"] = called if isinstance(called, bool) else None
            value["server_id"] = id(transport._server) if getattr(transport, "_server", None) is not None else None
            if hasattr(transport, "_sock"):
                sock = transport._sock
                value["socket_released"] = sock is None
                fileno = sock.fileno() if sock is not None else None
                value["socket_fileno"] = fileno if isinstance(fileno, int) else None
        except BaseException as failure:
            value["metadata_error"] = type(failure).__name__
        return value

    @staticmethod
    def error_number(exception, name):
        value = getattr(exception, name, None)
        return value if isinstance(value, int) else None

    @staticmethod
    def callback_name(callback, name):
        value = getattr(callback, name, None)
        return value[:200] if isinstance(value, str) else None

    def failed(self, loop, context):
        self.failures += 1
        if self.failures == 1:
            self.errors.append("OwnedLoopCallbackError")
        exception = context.get("exception")
        callback = getattr(context.get("handle"), "_callback", None)
        transport = context.get("transport")
        if transport is None:
            transport = getattr(callback, "__self__", None)
        try:
            self.save("callback_failure", same_original_loop=loop is self.loop,
                      exception_type=type(exception).__name__ if exception is not None else None,
                      errno=self.error_number(exception, "errno"), winerror=self.error_number(exception, "winerror"),
                      callback_module=self.callback_name(callback, "__module__"),
                      callback_name=self.callback_name(callback, "__qualname__"),
                      transport=self.transport_state(transport))
        except BaseException as failure:
            self.errors.append("OwnedLoopObserver:" + type(failure).__name__)
        finally:
            # Keep the original exception and traceback visible. No reset code
            # is suppressed or treated as an expected successful close.
            loop.default_exception_handler(context)

    def summary(self):
        return dict(owner=self.owner, failures=self.failures, overflow=self.overflow,
                    events=list(self.events), **self.snapshot())
