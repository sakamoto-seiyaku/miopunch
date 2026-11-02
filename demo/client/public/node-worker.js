// One origin owns one demo SDK runtime. The broker stays in SharedWorker;
// the SDK runs in exactly one attached Window, where RTCPeerConnection exists.
// Lost owner windows fail closed: never expire a lease and start a second key.
if (self.name !== 'miopunch-demo-node-v1' || new URL(self.location.href).pathname !== '/node-worker.js' || new URL(self.location.href).search) throw new Error('invalid demo node worker identity');
const ports = new Set();
const owners = new Set();
const pending = new Map();
let host, peer, lost = false, next = 0;
let lifecycle = Promise.resolve();
const unavailable = () => new Error('浏览器节点宿主页面已关闭，请关闭所有演示页面后重新打开');
function transition(action) {
  const result = lifecycle.then(action);
  lifecycle = result.catch(() => {});
  return result;
}
function callHost(action, values = {}) {
  if (!host || lost) return Promise.reject(unavailable());
  const call = ++next;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(call); reject(new Error('浏览器节点操作超时')); }, 65000);
    pending.set(call, { resolve, reject, timer });
    host.postMessage({ event: 'hostCall', call, action, ...values });
  });
}
function loseHost() {
  lost = true;
  for (const item of pending.values()) { clearTimeout(item.timer); item.reject(unavailable()); }
  pending.clear();
  for (const port of ports) port.postMessage({ event: 'hostLost' });
}
self.onconnect = event => {
  const port = event.ports[0];
  ports.add(port);
  port.onmessage = async event => {
    const { id, action, configuration, request } = event.data;
    if (action === 'hostStatus') {
      if (port === host) for (const target of ports) target.postMessage({ event: 'status', status: event.data.status });
      return;
    }
    if (action === 'hostReply') {
      if (port !== host) return;
      const item = pending.get(event.data.call);
      if (!item) return;
      pending.delete(event.data.call); clearTimeout(item.timer);
      if (event.data.error) item.reject(new Error(event.data.error));
      else item.resolve(event.data.result);
      return;
    }
    if (action === 'host-pagehide') {
      if (port === host) loseHost();
      return;
    }
    try {
      let result;
      if (action === 'version') result = { runtime: 'window-rtc-v2', lost };
      else if (action === 'connect') result = await transition(async () => {
        if (lost) throw unavailable();
        if (peer && peer !== configuration.peer) throw new Error('demo node already belongs to another host');
        if (!host) { host = port; peer = configuration.peer; }
        const value = await callHost('connect', { configuration });
        owners.add(port);
        return value;
      });
      else if (action === 'close') result = await transition(async () => {
        owners.delete(port);
        if (!owners.size && host && !lost) {
          try { await callHost('close'); host = peer = undefined; }
          catch (error) { loseHost(); throw error; }
        }
        return { closed: true };
      });
      else if (owners.has(port) && ['rpc', 'diagnostics', 'improve'].includes(action)) result = await callHost(action, { request });
      else throw new Error('demo node not attached');
      port.postMessage({ id, result });
    } catch (error) { port.postMessage({ id, error: String(error) }); }
  };
  port.start();
};
