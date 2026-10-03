"""Owned transparent CONNECT tunnel for public-relay failure injection.

TLS terminates at the real public service. The fixture logs only addresses and
byte counts and never handles QUIC/relay protocol semantics or returns data.
"""
import socket
import select
import threading
import time


class PublicRelayProxy:
    def __init__(self, hosts):
        self.hosts = {host.lower().rstrip(".") for host in hosts}
        self.cut = threading.Event()
        self.stopped = threading.Event()
        self.lock = threading.Lock()
        self.records = []
        self.sockets = []
        self.threads = []
        self.slots = threading.BoundedSemaphore(16)
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(16)
        self.listener.settimeout(0.2)
        self.url = "http://127.0.0.1:" + str(self.listener.getsockname()[1])
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stopped.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            if not self.slots.acquire(blocking=False):
                client.close()
                continue
            thread = threading.Thread(target=self.connect, args=(client,), daemon=True)
            self.threads.append(thread)
            thread.start()

    def connect(self, client):
        upstream = None
        record = None
        try:
            client.settimeout(3)
            header = bytearray()
            while not header.endswith(b"\r\n\r\n"):
                if len(header) >= 4096:
                    raise ValueError("proxy header limit")
                byte = client.recv(1)
                if not byte:
                    raise EOFError("proxy header ended")
                header.extend(byte)
            method, authority, _ = bytes(header).split(b"\r\n", 1)[0].decode("ascii").split(" ")
            host, port = authority.rsplit(":", 1)
            if method != "CONNECT" or host.lower().rstrip(".") not in self.hosts or port != "443":
                client.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                return
            record = {"host": host, "port": int(port), "started_unix_ms": int(time.time() * 1000),
                      "forwarded_to_public": 0, "forwarded_from_public": 0, "dropped": 0}
            with self.lock:
                self.records.append(record)
                self.sockets.append(client)
            if self.cut.is_set():
                client.sendall(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                record["blocked_reconnect"] = True
                return
            upstream = socket.create_connection((host, int(port)), timeout=5)
            record["actual_public_peer"] = list(upstream.getpeername())
            with self.lock:
                self.sockets.append(upstream)
            client.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            client.settimeout(0.2)
            upstream.settimeout(0.2)
            # Small fixed buffers keep a slow public server from creating an
            # unbounded user-space packet queue. Poll both directions fairly.
            while not self.stopped.is_set():
                readable, _, _ = select.select([client, upstream], [], [], 0.2)
                for source in readable:
                    target, direction = (upstream, "forwarded_to_public") if source is client else (client, "forwarded_from_public")
                    try:
                        data = source.recv(16384)
                    except socket.timeout:
                        continue
                    if not data:
                        return
                    with self.lock:
                        if self.cut.is_set():
                            record["dropped"] += len(data)
                            continue
                    target.sendall(data)
                    with self.lock:
                        record[direction] += len(data)
        except (OSError, ValueError, EOFError) as error:
            if record is not None:
                with self.lock:
                    record["ended"] = type(error).__name__
        finally:
            client.close()
            if upstream is not None:
                upstream.close()
            self.slots.release()

    def snapshot(self):
        with self.lock:
            return [dict(record) for record in self.records]

    def close(self):
        self.stopped.set()
        self.listener.close()
        with self.lock:
            sockets = list(self.sockets)
        for stream in sockets:
            try:
                stream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            stream.close()
        self.thread.join(timeout=2)
        for thread in self.threads:
            thread.join(timeout=1)
