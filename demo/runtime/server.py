#!/usr/bin/env python3
"""HTTP/static/relay supervisor for the explicitly enabled private demo.

Application traffic travels through Browser WASM -> encrypted member stream ->
Rust provider. HTTP here delivers assets and a bearer-protected invitation. The
other origin has no config or node-control endpoint. The HTTP mailbox carries
content requests back to the browser node; it does not fetch target websites.
"""
import base64
import functools
import gzip
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import mimetypes
import os
from pathlib import Path
import secrets
import select
import signal
import socket
import socketserver
import ssl
import subprocess
import sys
import threading
import time
from urllib.parse import parse_qs, unquote, urlsplit
from web_queue import WebQueue

APP = Path('/app')
STATE = Path('/state')
STATE.mkdir(mode=0o700, parents=True, exist_ok=True)
os.chown(STATE, 0, 0)
os.chmod(STATE, 0o700)
HOST = os.environ.get('DEMO_HOST', '192.168.4.14')
PORT = int(os.environ.get('DEMO_PORT', '18443'))
CONTENT_PORT = int(os.environ.get('DEMO_CONTENT_PORT', '18444'))
SCHEME = os.environ.get('DEMO_SCHEME', 'http')
if SCHEME not in ('http', 'https'):
    raise RuntimeError('unsupported demo URL scheme')
RELAY = f'{SCHEME}://{HOST}:{PORT}'
CONTENT_ORIGIN = f'{SCHEME}://{HOST}:{CONTENT_PORT}'
RTC_ICE = json.loads(os.environ.get('DEMO_RTC_ICE', '{"stun_servers":["stun:162.159.207.0:3478"]}'))
os.environ['DEMO_RTC_ICE'] = json.dumps(RTC_ICE)
WEB_QUEUE = WebQueue()
CONFIG = None
children = []
servers = []


@functools.lru_cache(maxsize=1)
def compressed_wasm(path, modified_ns):
    # The immutable SDK stays byte-for-byte intact on disk. Compression is only
    # the HTTP representation, cached once for the large WASM module.
    return gzip.compress(path.read_bytes(), compresslevel=6, mtime=0)


token_file = STATE / 'access-token'
if not token_file.exists():
    token_file.write_text(secrets.token_urlsafe(32))
    token_file.chmod(0o600)
TOKEN = token_file.read_text().strip()


def certificate():
    cert, key = STATE / 'tls-cert.pem', STATE / 'tls-key.pem'
    if not cert.exists():
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-sha256', '-nodes', '-days', '90', '-keyout', str(key), '-out', str(cert), '-subj', '/CN=miopunch-private-demo', '-addext', f'subjectAltName=IP:{HOST},IP:127.0.0.1,DNS:localhost', '-addext', 'basicConstraints=critical,CA:FALSE', '-addext', 'extendedKeyUsage=serverAuth'], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        key.chmod(0o600)
    subprocess.run(['openssl', 'x509', '-in', str(cert), '-outform', 'DER', '-out', str(STATE / 'tls-cert.der')], check=True)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(cert, key)
    return context


class EchoTCP(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        while True:
            try:
                data = self.request.recv(8192)
                if not data:
                    break
                self.request.sendall(data)
            except OSError:
                break


class EchoUDP(socketserver.BaseRequestHandler):
    def handle(self):
        data, sock = self.request
        sock.sendto(data, self.client_address)


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send_body(self, status, body, content_type='text/html; charset=utf-8', headers=None):
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.startswith('/api/status'):
            self.send_body(200, b'{"provider":"r_proxy","via":"native node"}', 'application/json')
        elif self.path.startswith('/private'):
            ok = 'demo_session=accepted' in self.headers.get('Cookie', '')
            self.send_body(200 if ok else 403, ('<h2 id="private-result">登录成功 · Cookie 保存在节点侧</h2>' if ok else '请先登录').encode())
        elif self.path.startswith('/large'):
            self.send_body(200, ('<!doctype html><h2>大响应验证</h2><p>' + 'm' * 100000 + '</p>').encode())
        elif self.path.startswith('/next'):
            self.send_body(200, '<h2 id="next-page">页面导航成功</h2><a href="/">返回</a>'.encode())
        else:
            self.send_body(200, '''<!doctype html><meta charset="utf-8"><style>body{font:16px system-ui;padding:24px;color:#243248}button,input{padding:9px;margin:5px}</style><h2>Linux 节点内网演示站点</h2><p id="via">这份页面由 Linux 原生节点读取，经成员连接传到浏览器。</p><p id="api">读取节点服务中…</p><a id="next" href="/next">尝试页面导航</a><form method="post" action="/login"><input name="user" placeholder="任意测试用户名"><button id="login">测试登录</button></form><script>fetch('/api/status').then(x=>x.json()).then(x=>document.querySelector('#api').textContent='API: '+x.provider);try{parent.document.body;window.parentReadable=true}catch{window.parentReadable=false}</script>'''.encode())

    def do_POST(self):
        count = int(self.headers.get('Content-Length', '0'))
        if count > 24000:
            return self.send_body(413, b'too large', 'text/plain')
        self.rfile.read(count)
        if self.path == '/login':
            self.send_body(303, b'', headers={'Location': '/private', 'Set-Cookie': 'demo_session=accepted; Path=/; HttpOnly; SameSite=Lax'})
        else:
            self.send_body(404, b'not found', 'text/plain')


def background(server):
    servers.append(server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


class Web(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    content_origin = False
    relay_port = 0

    def log_message(self, *_):
        # Do not log access credentials, invitation bodies, terminal or web data.
        pass

    def body(self, status, data, kind='text/plain; charset=utf-8', extra=None):
        self.send_response(status)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        if extra:
            for name, value in extra.items():
                self.send_header(name, value)
        self.end_headers()
        if self.command != 'HEAD':
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass

    def authorized(self):
        return hmac.compare_digest(self.headers.get('Authorization', ''), 'Bearer ' + TOKEN)

    def relay(self):
        upstream = socket.create_connection(('127.0.0.1', self.relay_port), timeout=10)
        upstream.settimeout(30)
        request = f'{self.command} {self.path} HTTP/1.1\r\n'
        for name, value in self.headers.items():
            if name.lower() not in ('authorization', 'cookie'):
                request += f'{name}: {value}\r\n'
        upstream.sendall((request + '\r\n').encode('latin1'))
        response = b''
        while b'\r\n\r\n' not in response:
            chunk = upstream.recv(1)
            if not chunk or len(response) > 32768:
                upstream.close()
                return self.body(502, b'relay handshake failed')
            response += chunk
        self.connection.sendall(response)
        if b' 101 ' not in response.split(b'\r\n')[0]:
            # Relay probes have small, finite bodies. A normal GET /ping is
            # handled separately; unexpected status ends this HTTP connection.
            upstream.close()
            self.close_connection = True
            return
        self.close_connection = True
        self.connection.settimeout(60)
        upstream.settimeout(60)
        try:
            while True:
                sockets = [self.connection, upstream]
                buffered = isinstance(self.connection, ssl.SSLSocket) and self.connection.pending()
                ready = [self.connection] if buffered else select.select(sockets, [], [], 45)[0]
                if not ready:
                    continue
                for source in ready:
                    data = source.recv(65536)
                    if not data:
                        return
                    (upstream if source is self.connection else self.connection).sendall(data)
        except (OSError, ssl.SSLError):
            pass
        finally:
            upstream.close()

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        parsed = urlsplit(self.path)
        path = parsed.path
        if not self.content_origin and path in ('/relay', '/derp'):
            return self.relay()
        if not self.content_origin and path == '/ping':
            return self.body(200, b'')
        if self.content_origin:
            if path == '/__open':
                params = parse_qs(parsed.query)
                key = params.get('channel', [''])[0]
                destination = params.get('path', ['/'])[0]
                try:
                    WEB_QUEUE.channel(key)
                    if not destination.startswith('/') or destination.startswith('//') or any(ord(x) < 32 for x in destination):
                        raise ValueError('invalid destination path')
                except ValueError as error:
                    return self.body(400, str(error).encode())
                return self.body(303, b'', extra={'Location': destination, 'Set-Cookie': f'miopunch_demo_route={key}; Path=/; HttpOnly; SameSite=Lax'})
            return WEB_QUEUE.serve(self, CONTENT_ORIGIN)
        if path == '/__web/next':
            if not self.authorized():
                return self.body(401, b'access capability required')
            try:
                message = WEB_QUEUE.poll(parse_qs(parsed.query).get('channel', [''])[0])
                return self.body(200, json.dumps(message).encode(), 'application/json')
            except ValueError as error:
                return self.body(410, str(error).encode())
        if path == '/__config':
            if not self.authorized():
                return self.body(401, b'private demo: access link required')
            if CONFIG is None:
                return self.body(503, b'native node is starting')
            try:
                return self.body(200, json.dumps({**CONFIG, 'webChannel': WEB_QUEUE.create()}).encode(), 'application/json')
            except ValueError as error:
                return self.body(503, str(error).encode())
        if path == '/__health':
            ready = CONFIG is not None and all(p.poll() is None for p in children)
            return self.body(200 if ready else 503, json.dumps({'ready': ready, 'native': bool(CONFIG)}).encode(), 'application/json')
        root = APP / ('pkg' if path.startswith('/pkg/') else 'client')
        relative = path[5:] if path.startswith('/pkg/') else path.lstrip('/') or 'index.html'
        file = (root / unquote(relative)).resolve()
        if not file.is_relative_to(root.resolve()) or not file.is_file():
            return self.body(404, b'not found')
        kind = mimetypes.guess_type(str(file))[0] or 'application/octet-stream'
        if file.suffix == '.wasm' and 'gzip' in self.headers.get('Accept-Encoding', ''):
            return self.body(200, compressed_wasm(file, file.stat().st_mtime_ns), kind,
                             {'Content-Encoding': 'gzip', 'Vary': 'Accept-Encoding'})
        return self.body(200, file.read_bytes(), kind)

    def do_POST(self):
        if self.content_origin:
            return WEB_QUEUE.serve(self, CONTENT_ORIGIN)
        parsed = urlsplit(self.path)
        if parsed.path != '/__web/reply' or not self.authorized():
            self.close_connection = True
            return self.body(401, b'access capability required')
        count = int(self.headers.get('Content-Length', '0'))
        if count < 0 or count > 12 * 1024 * 1024:
            self.close_connection = True
            return self.body(413, b'demo web reply exceeds limit')
        try:
            message = json.loads(self.rfile.read(count))
            accepted = WEB_QUEUE.reply(parse_qs(parsed.query).get('channel', [''])[0], message)
            return self.body(200 if accepted else 409, b'{}', 'application/json')
        except (ValueError, KeyError, TypeError) as error:
            return self.body(400, str(error).encode())

    do_PUT = do_POST
    do_PATCH = do_POST
    do_DELETE = do_POST
    do_OPTIONS = do_POST


def main():
    global CONFIG
    tls = certificate() if SCHEME == 'https' else None
    Path('/workspace').mkdir(exist_ok=True)
    os.chown('/workspace', 1000, 1000)
    fixture = background(ThreadingHTTPServer(('127.0.0.1', 0), Fixture))
    tcp = socketserver.ThreadingTCPServer(('127.0.0.1', 0), EchoTCP)
    tcp.daemon_threads = True
    tcp_port = background(tcp)
    udp_port = background(socketserver.ThreadingUDPServer(('127.0.0.1', 0), EchoUDP))
    relay = subprocess.Popen([str(APP / 'browser-probe-native'), 'relay', '0'], stdout=subprocess.PIPE, stderr=sys.stderr, text=True)
    children.append(relay)
    relay_line = relay.stdout.readline().strip()
    if not relay_line.startswith('RELAY '):
        raise RuntimeError('relay failed to start')
    relay_port = int(relay_line.rsplit(':', 1)[1])
    Web.relay_port = relay_port
    main_server = ThreadingHTTPServer(('0.0.0.0', PORT), Web)
    if tls:
        main_server.socket = tls.wrap_socket(main_server.socket, server_side=True)
    background(main_server)
    content_type = type('Content', (Web,), {'content_origin': True})
    content_server = ThreadingHTTPServer(('0.0.0.0', CONTENT_PORT), content_type)
    if tls:
        content_server.socket = tls.wrap_socket(content_server.socket, server_side=True)
    background(content_server)
    native = subprocess.Popen([str(APP / 'miopunch_demo'), str(STATE / 'native'), RELAY, str(STATE / 'tls-cert.der') if tls else '-', str(APP / 'terminal.py')], stdout=subprocess.PIPE, stderr=sys.stderr, text=True)
    children.append(native)
    ready = json.loads(native.stdout.readline())
    if ready.get('event') != 'ready':
        raise RuntimeError('native host failed to start')
    def drain_native_events():
        # The first private ready message is consumed above. Drain subsequent
        # SDK diagnostics so its stdout pipe cannot block the node; only the
        # established public active-path event types enter the rotated log.
        for line in native.stdout:
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get('event') in ('unified-active-result', 'unified-active-committed',
                                      'rtc-active-progress', 'rtc-path-cleanup',
                                      'rtc-candidate-cancelled'):
                print(line, end='', file=sys.stderr, flush=True)
    threading.Thread(target=drain_native_events, daemon=True).start()
    CONFIG = dict(peer=ready['peer'], code=ready['code'], relay=RELAY, contentOrigin=CONTENT_ORIGIN, exampleWeb=f'http://127.0.0.1:{fixture}/', echoTcp=tcp_port, echoUdp=udp_port, rtcIce=RTC_ICE)
    (STATE / 'entry-url').write_text(RELAY + '/#access=' + TOKEN)
    (STATE / 'entry-url').chmod(0o600)
    print('miopunch demo ready; private entry URL is in /state/entry-url', flush=True)
    while all(p.poll() is None for p in children):
        time.sleep(2)
    raise RuntimeError('a demo child exited; supervisor will restart')


def stop(*_):
    raise KeyboardInterrupt


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        main()
    except KeyboardInterrupt:
        pass
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
        for child in reversed(children):
            try:
                child.wait(timeout=8)
            except subprocess.TimeoutExpired:
                child.kill()
        for server in servers:
            server.server_close()
