import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { createServer as httpServer } from 'node:http';
import { createServer as httpsServer } from 'node:https';
import { connect } from 'node:net';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { createServer as tlsServer } from 'node:tls';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const argument = name => process.argv[process.argv.indexOf(name) + 1];
const evidence = resolve(argument('--evidence'));
const fixture = resolve(argument('--fixture'));
const [brokerHost, brokerPort] = argument('--broker').split(':');
const report = { startedUtc: new Date().toISOString(), passed: false, phases: [], normal: [], failures: [],
  native: [], websockets: [], proxy: [], tls: [], cleanup: {}, budgets: { outerSeconds: 60, phaseSeconds: 15 } };
const ownedSockets = new Set();
const nativeProcesses = new Set();
const servers = [];
let browser;
let page;
let stopping;

function save() { writeFileSync(join(evidence, 'mqtt-datagram.json'), `${JSON.stringify(report, null, 2)}\n`); }
function bounded(promise, label, milliseconds = 15000) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} deadline`)), milliseconds);
  })]).finally(() => clearTimeout(timer));
}
async function phase(name, action) {
  const start = Date.now();
  const entry = { name, startedUtc: new Date(start).toISOString(), budgetSeconds: 15 };
  report.phases.push(entry);
  try { const value = await bounded(action(), name); entry.passed = true; return value; }
  catch (error) { entry.passed = false; entry.cancelReason = String(error); throw error; }
  finally { entry.milliseconds = Date.now() - start; save(); }
}
function track(socket) {
  ownedSockets.add(socket);
  socket.on('error', () => {});
  socket.once('close', () => ownedSockets.delete(socket));
  return socket;
}
function mqttReader(consume) {
  let pending = Buffer.alloc(0);
  return bytes => {
    pending = Buffer.concat([pending, bytes]);
    if (pending.length > 16384) throw new Error('fixture MQTT buffer limit');
    while (pending.length >= 2) {
      let length = 0;
      let offset = 1;
      let digit;
      do {
        if (offset >= pending.length) return;
        if (offset > 4) throw new Error('fixture malformed MQTT length');
        digit = pending[offset++];
        length += (digit & 127) * (128 ** (offset - 2));
      } while (digit & 128);
      if (offset + length > 4096) throw new Error('fixture MQTT packet limit');
      if (pending.length < offset + length) return;
      const packet = Buffer.from(pending.subarray(0, offset + length));
      pending = pending.subarray(offset + length);
      consume(packet, offset);
    }
  };
}
function bridge(client, send, mode, label) {
  const broker = track(connect({ host: brokerHost, port: Number(brokerPort) }));
  const entry = { label, mode, packetsFromClient: [], packetsFromBroker: [], publishCount: 0 };
  report.proxy.push(entry);
  const outgoing = mqttReader((packet, header) => {
    const kind = packet[0] >> 4;
    entry.packetsFromClient.push({ kind, bytes: packet.length, atMs: Date.now() });
    if (kind === 3) entry.publishCount++;
    broker.write(packet);
  });
  const incoming = mqttReader((packet, header) => {
    const kind = packet[0] >> 4;
    entry.packetsFromBroker.push({ kind, bytes: packet.length, atMs: Date.now() });
    if (kind === 9 && mode === 'reject-suback') packet[header + 2] = 0x80;
    if (kind === 9 && mode === 'unmatched-suback') packet[header + 1] ^= 1;
    if (kind === 9 && mode === 'cut-before-suback') { client.destroy(); broker.destroy(); return; }
    send(packet);
  });
  broker.on('data', bytes => { try { incoming(bytes); } catch { client.destroy(); broker.destroy(); } });
  broker.on('error', () => client.destroy());
  broker.on('close', () => client.destroy());
  client.on('close', () => { entry.closedMs = Date.now(); broker.destroy(); });
  return bytes => { try { outgoing(bytes); } catch { client.destroy(); broker.destroy(); } };
}
function websocketFrame(bytes, opcode = 2) {
  const header = bytes.length < 126 ? Buffer.from([0x80 | opcode, bytes.length]) :
    Buffer.from([0x80 | opcode, 126, bytes.length >> 8, bytes.length & 255]);
  return Buffer.concat([header, bytes]);
}
function websocketReader(socket, receive) {
  let pending = Buffer.alloc(0);
  return bytes => {
    pending = Buffer.concat([pending, bytes]);
    if (pending.length > 16384) return socket.destroy();
    while (pending.length >= 2) {
      const opcode = pending[0] & 15;
      if (!(pending[0] & 128) || !(pending[1] & 128)) return socket.destroy();
      let length = pending[1] & 127;
      let offset = 2;
      if (length === 127) return socket.destroy();
      if (length === 126) {
        if (pending.length < 4) return;
        length = pending.readUInt16BE(2);
        offset = 4;
      }
      if (length > 4096) return socket.destroy();
      if (pending.length < offset + 4 + length) return;
      const mask = pending.subarray(offset, offset + 4);
      const frame = Buffer.from(pending.subarray(offset + 4, offset + 4 + length));
      for (let i = 0; i < frame.length; i++) frame[i] ^= mask[i % 4];
      pending = pending.subarray(offset + 4 + length);
      if (opcode === 8) { socket.end(websocketFrame(Buffer.alloc(0), 8)); return; }
      if (opcode === 9) socket.write(websocketFrame(frame, 10));
      else if (opcode === 2) receive(frame);
      else return socket.destroy();
    }
  };
}
async function listen(server) {
  servers.push(server);
  await new Promise((resolveListening, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolveListening);
  });
  return server.address().port;
}
function executable() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE) return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
  if (existsSync(chromium.executablePath())) return chromium.executablePath();
  for (const name of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(name => name.startsWith('chromium-')).sort().reverse()) {
    for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const candidate = join(homedir(), '.cache/ms-playwright', name, suffix);
      if (existsSync(candidate)) return candidate;
    }
  }
  throw new Error('pinned Chromium unavailable');
}
function quote(value) { return `'${value.replaceAll("'", "'\\''")}'`; }
function nativeFixture(config, name) {
  const configPath = join(fixture, `native-${name}.json`);
  writeFileSync(configPath, JSON.stringify(config));
  const child = spawn('rtk', ['proxy', join(root, 'target/debug/browser_mqtt_datagram'), configPath, join(fixture, 'ca.pem')], { stdio: ['pipe', 'pipe', 'pipe'] });
  nativeProcesses.add(child);
  const entries = [];
  const pending = [];
  let failure;
  createInterface({ input: child.stdout }).on('line', line => {
    const value = JSON.parse(line);
    report.native.push({ name, ...value });
    entries.push(value);
    for (const waiter of [...pending]) {
      const index = entries.findIndex(waiter.matches);
      if (index >= 0) { pending.splice(pending.indexOf(waiter), 1); waiter.resolve(entries.splice(index, 1)[0]); }
    }
  });
  child.stderr.on('data', bytes => console.error(`native ${name}: ${bytes}`));
  child.on('error', error => { failure = error; for (const waiter of pending.splice(0)) waiter.reject(error); });
  child.once('exit', (code, signal) => {
    nativeProcesses.delete(child);
    failure = new Error(`native ${name} exited ${code}/${signal}`);
    for (const waiter of pending.splice(0)) waiter.reject(failure);
  });
  let id = 0;
  function wait(event, match = () => true) {
    const matches = value => value.event === event && match(value);
    const index = entries.findIndex(matches);
    if (index >= 0) return Promise.resolve(entries.splice(index, 1)[0]);
    if (failure) return Promise.reject(failure);
    let waiter;
    const promise = new Promise((resolveValue, reject) => { waiter = { matches, resolve: resolveValue, reject }; pending.push(waiter); });
    return bounded(promise, `native ${name} ${event}`).finally(() => {
      const index = pending.indexOf(waiter); if (index >= 0) pending.splice(index, 1);
    });
  }
  return { child, wait, command(command, event) {
    const request = ++id;
    const result = wait(event, value => value.id === request || event === 'status');
    child.stdin.write(`${JSON.stringify({ ...command, id: request })}\n`);
    return result;
  }, async stop() {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = new Promise(resolveExit => child.once('exit', (code, signal) => resolveExit({ code, signal })));
    child.stdin.end(`${JSON.stringify({ command: 'stop' })}\n`);
    assert.equal((await bounded(exited, `native ${name} cleanup`, 3000)).code, 0);
  } };
}
async function startBrowser(config) {
  assert.equal(await page.evaluate(config => {
    window.carrier?.free();
    window.carrier = window.BrowserMqttDatagram.connect(JSON.stringify(config));
    try { window.carrier.route(); return false; } catch { return true; }
  }, config), true);
}
async function browserStatus() { return JSON.parse(await page.evaluate(() => window.carrier.diagnostics())); }
function configFor(nativePort, wssPort, mode = 'normal') {
  return { facility: { logical_id: 'owned-mqtt-datagram-fixture', native_url: `mqtts://localhost:${nativePort}/`, wss_url: `wss://localhost:${wssPort}/mqtt/${mode}` }, disabled: [] };
}
async function cleanup() {
  if (stopping) return stopping;
  stopping = (async () => {
    await page?.evaluate(() => { window.carrier?.close(); window.carrier?.free(); }).catch(() => {});
    await browser?.close();
    await Promise.all([...nativeProcesses].map(async child => {
      const exited = new Promise(resolveExit => child.once('exit', resolveExit));
      child.stdin.end(`${JSON.stringify({ command: 'stop' })}\n`);
      try { await bounded(exited, 'native final cleanup', 3000); }
      catch { child.kill('SIGKILL'); await exited; }
    }));
    for (const socket of ownedSockets) socket.destroy();
    await Promise.all(servers.map(server => new Promise(resolveClose => server.close(resolveClose))));
    report.cleanup.activeNativeProcesses = nativeProcesses.size;
    report.cleanup.activeProxySockets = ownedSockets.size;
    report.cleanup.serversClosed = servers.every(server => !server.listening);
    report.completedUtc = new Date().toISOString();
    save();
  })();
  return stopping;
}
process.on('SIGTERM', () => { report.failure = 'external process-group deadline/cancel'; cleanup().finally(() => process.exit(1)); });

try {
  await phase('fixture-and-browser-start', async () => {
    const tlsOptions = { key: readFileSync(join(fixture, 'leaf.key')), cert: readFileSync(join(fixture, 'leaf.pem')) };
    const native = tlsServer(tlsOptions, socket => {
      track(socket);
      report.tls.push({ role: 'native', protocol: socket.getProtocol(), cipher: socket.getCipher().standardName, atMs: Date.now() });
      const receive = bridge(socket, bytes => socket.write(bytes), 'normal', 'native-normal');
      socket.on('data', receive);
    });
    report.nativePort = await listen(native);
    const wss = httpsServer(tlsOptions, (request, response) => response.writeHead(404).end());
    wss.on('upgrade', (request, socket, head) => {
      track(socket);
      const mode = request.url.split('/').at(-1);
      if (!['normal', 'reject-suback', 'unmatched-suback', 'cut-before-suback'].includes(mode)
          || request.headers['sec-websocket-protocol'] !== 'mqtt') return socket.destroy();
      const accept = createHash('sha1').update(request.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: mqtt\r\n\r\n`);
      report.tls.push({ role: 'browser', protocol: socket.getProtocol(), cipher: socket.getCipher().standardName, atMs: Date.now() });
      const receive = bridge(socket, bytes => socket.write(websocketFrame(bytes)), mode, `browser-${mode}`);
      const read = websocketReader(socket, receive);
      socket.on('data', read);
      if (head.length) read(head);
    });
    report.wssPort = await listen(wss);
    const web = httpServer((request, response) => {
      const pathname = new URL(request.url, 'http://localhost').pathname;
      if (pathname === '/') return response.writeHead(200, { 'Content-Type': 'text/html' }).end('<!doctype html><title>MQTT datagram test</title>');
      const file = resolve(root, 'browser-probe', `.${pathname}`);
      if (!pathname.startsWith('/pkg/') || !file.startsWith(`${root}/browser-probe/pkg/`)) return response.writeHead(404).end();
      response.setHeader('Content-Type', pathname.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
      createReadStream(file).on('error', () => response.writeHead(404).end()).pipe(response);
    });
    const webPort = await listen(web);
    report.origin = `http://127.0.0.1:${webPort}`;
    const legacyDb = join(homedir(), '.pki/nssdb');
    const defaultDb = join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local/share'), 'pki/nssdb');
    const nssTarget = existsSync(legacyDb) ? legacyDb : defaultDb;
    assert(existsSync(nssTarget), 'existing NSS mountpoint required; never create or alter user trust');
    const wrapper = join(fixture, 'chromium-private-trust.sh');
    writeFileSync(wrapper, `#!/bin/sh\nexec rtk proxy bwrap --ro-bind / / --tmpfs /tmp --bind ${quote(fixture)} ${quote(fixture)} --bind ${quote(join(fixture, 'nssdb'))} ${quote(nssTarget)} --dev-bind /dev /dev --proc /proc -- ${quote(executable())} "$@"\n`, { mode: 0o700 });
    report.launch = { executablePath: wrapper, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false, privateNssMount: true, certificateBypassFlags: [] };
    browser = await chromium.launchPersistentContext(join(fixture, 'profile'), report.launch);
    report.browserVersion = browser.browser().version();
    page = await browser.newPage();
    const cdp = await browser.newCDPSession(page);
    await cdp.send('Network.enable');
    const sockets = new Map();
    cdp.on('Network.webSocketCreated', ({ requestId, url }) => {
      const socket = { url, atMs: Date.now(), sentFrames: 0, sentBytes: 0, receivedFrames: 0, receivedBytes: 0 };
      sockets.set(requestId, socket); report.websockets.push(socket);
    });
    cdp.on('Network.webSocketHandshakeResponseReceived', ({ requestId, response }) => {
      const socket = sockets.get(requestId);
      if (socket) Object.assign(socket, { status: response.status, protocol: response.headers['Sec-WebSocket-Protocol'] ?? response.headers['sec-websocket-protocol'] });
    });
    for (const [event, prefix] of [['Network.webSocketFrameSent', 'sent'], ['Network.webSocketFrameReceived', 'received']]) {
      cdp.on(event, ({ requestId, response }) => {
        const socket = sockets.get(requestId); if (!socket) return;
        socket[`${prefix}Frames`]++;
        socket[`${prefix}Bytes`] += response.opcode === 2 ? Buffer.from(response.payloadData, 'base64').length : Buffer.byteLength(response.payloadData);
      });
    }
    page.on('pageerror', error => console.error(`browser: ${error.stack}`));
    await page.goto(report.origin);
    await page.evaluate(async () => {
      const module = await import('/pkg/miopunch_browser_probe.js');
      await module.default();
      window.BrowserMqttDatagram = module.BrowserMqttDatagram;
    });
  });
  const config = configFor(report.nativePort, report.wssPort);
  report.facility = config.facility;
  let native;
  let nativeRoute;
  let browserRoute;
  await phase('successful-acks', async () => {
    native = nativeFixture(config, 'normal');
    assert.equal((await native.wait('starting')).route_rejected, true);
    await startBrowser(config);
    const [ready] = await Promise.all([native.wait('ready'), page.evaluate(() => window.carrier.wait_ready())]);
    nativeRoute = ready.route;
    browserRoute = await page.evaluate(() => window.carrier.route());
    assert.equal(nativeRoute.slice(0, 32), browserRoute.slice(0, 32), 'logical service identity must match across aliases');
    assert.notEqual(nativeRoute, browserRoute, 'independent endpoints must use distinct route tokens');
    assert.equal(ready.address_count, 1);
    assert.equal(ready.traffic.protocol.connack, 0);
    assert.equal(ready.traffic.protocol.subscribe_id, ready.traffic.protocol.suback_id);
    assert.deepEqual(ready.traffic.protocol.suback_codes, [0]);
  });
  await phase('four-full-datagrams-and-local-rejection', async () => {
    for (const length of [1200, 2048]) {
      // A deterministic independent oracle, checked byte for byte in both directions.
      const toBrowser = Buffer.from(Array.from({ length }, (_, i) => (i * 37 + 11) & 255));
      const receivedBrowser = page.evaluate(() => window.carrier.receive());
      assert.equal((await native.command({ command: 'send', route: browserRoute, bytes: toBrowser.toString('hex') }, 'sent')).accepted, true);
      const browserPacket = JSON.parse(await receivedBrowser);
      assert.equal(browserPacket.source, nativeRoute);
      assert.deepEqual(Buffer.from(browserPacket.bytes, 'hex'), toBrowser);
      report.normal.push({ direction: 'native-to-browser', bytes: length, byteEqual: true });
      const toNative = Buffer.from(Array.from({ length }, (_, i) => (i * 19 + 211) & 255));
      const receivedNative = native.wait('received');
      await page.evaluate(({ route, bytes }) => window.carrier.send(route, new Uint8Array(bytes)), { route: nativeRoute, bytes: [...toNative] });
      const nativePacket = await receivedNative;
      assert.equal(nativePacket.source, browserRoute);
      assert.deepEqual(Buffer.from(nativePacket.bytes, 'hex'), toNative);
      report.normal.push({ direction: 'browser-to-native', bytes: length, byteEqual: true });
    }
    const publications = () => report.proxy.reduce((count, proxy) => count + proxy.publishCount, 0);
    assert.equal(publications(), 4);
    for (const bad of ['oversize', 'wrong-service']) {
      const route = bad === 'wrong-service' ? `${nativeRoute[0] === '0' ? '1' : '0'}${nativeRoute.slice(1)}` : nativeRoute;
      const bytes = Buffer.alloc(bad === 'oversize' ? 2049 : 1200);
      assert.equal(await page.evaluate(({ route, bytes }) => { try { window.carrier.send(route, new Uint8Array(bytes)); return false; } catch { return true; } }, { route, bytes: [...bytes] }), true);
      const target = bad === 'wrong-service' ? `${browserRoute[0] === '0' ? '1' : '0'}${browserRoute.slice(1)}` : browserRoute;
      assert.equal((await native.command({ command: 'send', route: target, bytes: bytes.toString('hex') }, 'sent')).accepted, false);
      report.failures.push({ case: bad, browserRejected: true, nativeRejected: true });
    }
    await new Promise(resolveDelay => setTimeout(resolveDelay, 100));
    assert.equal(publications(), 4, 'rejected datagrams must not reach the actual broker');
    report.normalBrowser = await browserStatus();
    report.normalNative = await native.command({ command: 'status' }, 'status');
    assert.equal(report.normalBrowser.traffic.received_packets, 2);
    assert.equal(report.normalNative.traffic.received_packets, 2);
    assert.equal(report.normalBrowser.queued_packets, 0);
    assert.equal(report.normalNative.traffic.queued_packets, 0);
  });
  await phase('live-disconnection-withdrawal', async () => {
    const withdrawal = native.wait('withdrawn');
    for (const socket of ownedSockets) socket.destroy();
    const entry = await withdrawal;
    assert.equal(entry.route_rejected, true);
    assert.equal(entry.address_count, 0);
    await page.waitForFunction(() => JSON.parse(window.carrier.diagnostics()).closed);
    const browserClosed = await browserStatus();
    assert.equal(browserClosed.ready, false);
    assert.equal(browserClosed.route, null);
    assert(browserClosed.traffic.ready_ms < browserClosed.traffic.withdraw_ms);
    assert(entry.traffic.protocol.ready_ms < entry.traffic.protocol.withdraw_ms);
    await native.stop();
    report.failures.push({ case: 'disconnect-after-ready', browser: browserClosed, native: entry });
  });
  await phase('subscription-failure-and-pre-ready-disconnect', async () => {
    for (const mode of ['reject-suback', 'unmatched-suback', 'cut-before-suback']) {
      await startBrowser(configFor(report.nativePort, report.wssPort, mode));
      await assert.rejects(() => page.evaluate(() => window.carrier.wait_ready()));
      const state = await browserStatus();
      assert.equal(state.ready, false);
      assert.equal(state.route, null);
      assert.equal(state.traffic.ready_ms, null);
      assert.equal(state.queued_packets, 0);
      assert.equal(state.pending_bytes, 0);
      report.failures.push({ case: mode, browser: state });
    }
    // The same real-broker faults are also injected into the native MQTT stream.
    const options = { key: readFileSync(join(fixture, 'leaf.key')), cert: readFileSync(join(fixture, 'leaf.pem')) };
    for (const mode of ['reject-suback', 'unmatched-suback', 'cut-before-suback']) {
      const denied = tlsServer(options, socket => {
        track(socket);
        socket.on('data', bridge(socket, bytes => socket.write(bytes), mode, `native-${mode}`));
      });
      const deniedPort = await listen(denied);
      const failed = nativeFixture(configFor(deniedPort, report.wssPort), mode);
      const starting = await failed.wait('starting');
      assert.equal(starting.route_rejected, true);
      assert.equal(starting.address_count, 0);
      const entry = await failed.wait('failed');
      assert.equal(entry.route_rejected, true);
      assert.equal(entry.address_count, 0);
      assert.equal(entry.traffic.protocol.ready_ms, null);
      if (mode === 'reject-suback') assert.deepEqual(entry.traffic.protocol.suback_codes, [128]);
      await failed.stop();
      report.failures.push({ case: `native-${mode}`, native: entry });
    }
  });
  await phase('alias-disable-and-clean-close', async () => {
    const socketsBefore = report.websockets.length;
    for (const disabled of [config.facility.logical_id, config.facility.native_url, config.facility.wss_url]) {
      assert.equal(await page.evaluate(config => { try { window.BrowserMqttDatagram.connect(JSON.stringify(config)); return false; } catch { return true; } }, { ...config, disabled: [disabled] }), true);
    }
    assert.equal(report.websockets.length, socketsBefore);
    await startBrowser(config);
    await page.evaluate(() => window.carrier.wait_ready());
    const closeAt = Date.now();
    await page.evaluate(() => window.carrier.close());
    const closed = await browserStatus();
    assert.equal(closed.ready, false);
    assert.equal(closed.route, null);
    assert.equal(closed.queued_packets, 0);
    assert(closed.traffic.unsubscribe_sent && closed.traffic.disconnect_sent);
    const cleanNative = nativeFixture(config, 'clean-close');
    await cleanNative.wait('ready');
    await cleanNative.stop();
    const finalNative = report.native.findLast(entry => entry.name === 'clean-close' && entry.event === 'closed');
    assert(finalNative.traffic.protocol.unsubscribe_sent && finalNative.traffic.protocol.disconnect_sent);
    assert.equal(finalNative.traffic.queued_packets, 0);
    report.cleanup.carriers = { browser: closed, native: finalNative, closeAt };
  });
  assert.equal(report.normal.length, 4);
  assert(report.websockets.every(socket => socket.status === 101 && socket.protocol === 'mqtt'));
  assert(report.tls.every(entry => entry.protocol === 'TLSv1.3' || entry.protocol === 'TLSv1.2'));
  report.passed = true;
  console.log(`PASS ${JSON.stringify({ normal: report.normal, failures: report.failures.map(entry => entry.case) })}`);
} catch (error) {
  report.failure = error.stack;
  console.error(error.stack);
  process.exitCode = 1;
} finally {
  await cleanup();
}
