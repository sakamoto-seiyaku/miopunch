// THROWAWAY: Fetch forwarding only. Native WebSocket handshakes are not covered.
let bridgeId;
let nextId = 0;
const pending = new Map();
self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('message', event => {
  const source = event.source;
  if (!source || new URL(source.url).pathname !== '/__bridge.html') return;
  if (event.data.type === 'register-bridge') {
    bridgeId = source.id;
    source.postMessage({ type: 'bridge-registered' });
  }
  if (event.data.type === 'http-response' && source.id === bridgeId) {
    const resolve = pending.get(event.data.id);
    if (resolve) { pending.delete(event.data.id); resolve(event.data.result); }
  }
});
function encode(bytes) {
  let value = '';
  for (const byte of bytes) value += String.fromCharCode(byte);
  return btoa(value);
}
async function forward(request) {
  const url = new URL(request.url);
  // A third-party or absolute original-origin request must not silently escape
  // the provider route. Broader URL rewriting is a separate compatibility task.
  if (url.origin !== self.location.origin) return new Response('Prototype refuses direct external requests', { status: 403 });
  const bridge = bridgeId && await self.clients.get(bridgeId);
  if (!bridge) return new Response('Prototype node bridge unavailable', { status: 503 });
  const body = request.method === 'GET' ? '' : encode(new Uint8Array(await request.arrayBuffer()));
  const id = ++nextId;
  let timer;
  const reply = new Promise(resolve => {
    pending.set(id, resolve);
    timer = setTimeout(() => { pending.delete(id); resolve({ error: 'Provider request timed out' }); }, 15000);
  });
  bridge.postMessage({ type: 'http-request', id, request: { path: url.pathname + url.search,
    method: request.method, body, content_type: request.headers.get('content-type') || '' } });
  const result = await reply;
  clearTimeout(timer);
  if (result.error) return new Response(result.error, { status: 502, headers: { 'content-type': 'text/plain; charset=utf-8' } });
  const bytes = Uint8Array.from(atob(result.body), character => character.charCodeAt(0));
  const headers = new Headers(result.headers);
  headers.set('cache-control', 'no-store');
  const location = headers.get('location');
  if (location?.startsWith('http://node-only.invalid')) headers.set('location', location.replace('http://node-only.invalid', self.location.origin));
  return new Response(bytes, { status: result.status, headers });
}
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (url.origin === self.location.origin && url.pathname.startsWith('/__')) return;
  event.respondWith(forward(event.request));
});
