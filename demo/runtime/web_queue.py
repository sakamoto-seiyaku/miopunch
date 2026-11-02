"""Bounded HTTP request mailbox; the browser node performs every target fetch.

This server never connects to a target website. An authenticated management
page consumes HTTP-shaped requests, uses its WASM member connection, and returns
the resulting bytes. Content pages have no management capability.
"""
import base64
from http.cookies import SimpleCookie
import json
import queue
import secrets
import threading
import time
from urllib.parse import urlsplit


class WebQueue:
    def __init__(self):
        self.lock = threading.Lock()
        self.channels = {}

    def create(self):
        with self.lock:
            now = time.monotonic()
            stale = [key for key, item in self.channels.items() if now - item['touched'] > 90]
            for key in stale:
                del self.channels[key]
            if len(self.channels) >= 16:
                raise ValueError('演示网页通道已满，请关闭不再使用的页面，稍后重试')
            key = secrets.token_urlsafe(24)
            self.channels[key] = {'queue': queue.Queue(maxsize=32), 'pending': {}, 'touched': now}
            return key

    def channel(self, key):
        with self.lock:
            item = self.channels.get(key)
            if not item or time.monotonic() - item['touched'] > 90:
                raise ValueError('网页通道已关闭，请返回主页面重新连接')
            return item

    def poll(self, key):
        item = self.channel(key)
        item['touched'] = time.monotonic()
        try:
            result = item['queue'].get(timeout=20)
        except queue.Empty:
            result = {'idle': True}
        item['touched'] = time.monotonic()
        return result

    def reply(self, key, message):
        item = self.channel(key)
        with self.lock:
            pending = item['pending'].get(message.get('id'))
            if not pending:
                return False
            pending['result'] = message['result']
            pending['ready'].set()
            return True

    def forward(self, key, request):
        item = self.channel(key)
        ident = secrets.token_hex(16)
        pending = {'ready': threading.Event(), 'result': None}
        with self.lock:
            if len(item['pending']) >= 32:
                raise ValueError('网页并发请求超过演示上限')
            item['pending'][ident] = pending
        try:
            item['queue'].put_nowait({'id': ident, 'request': request})
            if not pending['ready'].wait(timeout=45):
                raise ValueError('浏览器节点网页请求超时，请确认主页面仍在运行')
            return pending['result']
        finally:
            with self.lock:
                item['pending'].pop(ident, None)

    def serve(self, handler, content_origin):
        cookies = SimpleCookie()
        cookies.load(handler.headers.get('Cookie', ''))
        route = cookies.get('miopunch_demo_route')
        if not route:
            return handler.body(503, '请从 miopunch 主页面打开内网地址。'.encode())
        try:
            count = int(handler.headers.get('Content-Length', '0'))
            if count < 0 or count > 24000:
                handler.close_connection = True
                return handler.body(413, b'demo website upload limit: 24 KiB')
            body = handler.rfile.read(count) if count else b''
            request = {'path': handler.path, 'method': handler.command,
                       'headers': {name: handler.headers[name] for name in ('content-type', 'accept', 'x-requested-with') if name in handler.headers},
                       'body': base64.b64encode(body).decode()}
            result = self.forward(route.value, request)
            if result.get('error'):
                raise ValueError(result['error'])
            status = int(result['status'])
            if not 200 <= status <= 599:
                raise ValueError('invalid upstream HTTP status')
            outgoing = {str(k).lower(): str(v) for k, v in result['headers'] if str(k).lower() in ('content-type', 'location', 'cache-control', 'content-language')}
            data = base64.b64decode(result['body'], validate=True)
            if len(data) > 8 * 1024 * 1024:
                raise ValueError('website response exceeds demo limit')
            destination = outgoing.get('location', '')
            if destination.startswith(('http:', 'https:')):
                target = urlsplit(destination)
                origin = f'{target.scheme}://{target.netloc}'
                if origin != result['origin']:
                    raise ValueError('目标跳转到另一站点，请在地址栏单独打开该地址')
                outgoing['location'] = target.path + ('?' + target.query if target.query else '')
            kind = outgoing.pop('content-type', 'application/octet-stream')
            if any(item in kind for item in ('text/', 'javascript', 'json', 'svg')):
                data = data.replace(result['origin'].encode(), content_origin.encode())
            if 'text/html' in kind:
                outgoing['Content-Security-Policy'] = "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data: blob:; frame-src 'none'; object-src 'none'; base-uri 'self'; form-action 'self'"
            return handler.body(status, b'' if status in (204, 205, 304) else data, kind, outgoing)
        except (ValueError, KeyError, TypeError, queue.Full) as error:
            return handler.body(502, str(error).encode(), 'text/plain; charset=utf-8')
