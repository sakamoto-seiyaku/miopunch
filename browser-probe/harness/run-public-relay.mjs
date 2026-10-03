import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const argument = name => process.argv[process.argv.indexOf(name) + 1];
assert(process.argv.includes('--evidence'), '--evidence directory is required');
const evidence = resolve(argument('--evidence'));
const relay = process.argv.includes('--relay') ? argument('--relay') : 'https://aps1-1.relay.n0.iroh.link./';
const official = ['aps1-1', 'use1-1', 'usw1-1', 'euc1-1'].map(region => `${region}.relay.n0.iroh.link`);
const relayAddress = new URL(relay);
assert.equal(relayAddress.protocol, 'https:');
assert(official.includes(relayAddress.hostname.replace(/\.$/, '')), 'select an official n0 production relay');
assert.equal(relayAddress.username + relayAddress.password + relayAddress.search + relayAddress.hash, '');
assert.equal(relayAddress.pathname, '/');
assert(!relayAddress.port || relayAddress.port === '443');
const temporary = process.env.MIO65_FIXTURE_DIR ?? mkdtempSync(join(tmpdir(), 'mio65-browser-'));
const report = { startedUtc: new Date().toISOString(), relay, scenarios: [], requests: [], websockets: [], native: [] };
let context;
let native;
let web;

function save() { writeFileSync(join(evidence, 'public-browser.json'), `${JSON.stringify(report, null, 2)}\n`); }
function bounded(promise, label, milliseconds = 30000) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), milliseconds);
  })]).finally(() => clearTimeout(timer));
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
  throw new Error('install the pinned Playwright Chromium');
}
function nativeFixture() {
  const process = spawn('rtk', ['proxy', join(root, 'target/debug/browser_network_native'), join(temporary, 'admin'), relay], { stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = [];
  const waiters = [];
  createInterface({ input: process.stdout }).on('line', line => {
    console.log(`native: ${line}`);
    let entry;
    try { entry = JSON.parse(line); } catch { return; }
    report.native.push(entry);
    lines.push(entry);
    for (const waiter of [...waiters]) if (entry.event === waiter.event) {
      waiters.splice(waiters.indexOf(waiter), 1);
      waiter.resolve(entry);
    }
  });
  process.stderr.on('data', data => console.error(`native: ${data}`));
  function wait(event, fresh = false) {
    const existing = !fresh && lines.find(entry => entry.event === event);
    if (existing) return Promise.resolve(existing);
    if (process.exitCode !== null || process.signalCode !== null) return Promise.reject(new Error(`native already exited before ${event}`));
    let waiter;
    let onExit;
    let onError;
    const waiting = new Promise((resolve, reject) => {
      waiter = { event, resolve };
      waiters.push(waiter);
      onExit = (code, signal) => reject(new Error(`native exited ${code}/${signal} before ${event}`));
      onError = reject;
      process.once('exit', onExit);
      process.once('error', onError);
    });
    return bounded(waiting, `native ${event}`).finally(() => {
      const index = waiters.indexOf(waiter);
      if (index >= 0) waiters.splice(index, 1);
      process.removeListener('exit', onExit);
      process.removeListener('error', onError);
    });
  }
  return { process, wait, command(value, event) {
    const result = wait(event, true);
    process.stdin.write(`${value}\n`);
    return result;
  } };
}
async function newPage() {
  const page = await context.newPage();
  page.on('console', message => console.log(`browser: ${message.text()}`));
  page.on('pageerror', error => console.error(`browser: ${error}`));
  const cdp = await context.newCDPSession(page);
  await cdp.send('Network.enable');
  const sockets = new Map();
  cdp.on('Network.webSocketCreated', ({ requestId, url }) => {
    const socket = { url, atUtc: new Date().toISOString(), sentFrames: 0, receivedFrames: 0, sentBytes: 0, receivedBytes: 0 };
    sockets.set(requestId, socket);
    report.websockets.push(socket);
  });
  cdp.on('Network.webSocketHandshakeResponseReceived', ({ requestId, response }) => {
    const socket = sockets.get(requestId);
    if (socket) Object.assign(socket, { status: response.status, protocol: response.headers['Sec-WebSocket-Protocol'] ?? response.headers['sec-websocket-protocol'] });
  });
  for (const [event, prefix] of [['Network.webSocketFrameSent', 'sent'], ['Network.webSocketFrameReceived', 'received']]) {
    cdp.on(event, ({ requestId, response }) => {
      const socket = sockets.get(requestId);
      if (!socket) return;
      socket[`${prefix}Frames`]++;
      socket[`${prefix}Bytes`] += response.opcode === 2 ? Buffer.from(response.payloadData, 'base64').length : Buffer.byteLength(response.payloadData);
    });
  }
  cdp.on('Network.webSocketFrameError', ({ requestId, errorMessage }) => {
    const socket = sockets.get(requestId);
    if (socket) socket.error = errorMessage;
  });
  return page;
}
async function start(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.ready !== undefined);
  return bounded(page.evaluate(() => window.ready), 'BrowserNetwork start');
}
async function exchange(page, message) {
  assert.equal(await bounded(page.evaluate(message => window.network.exchange(message), message), 'member exchange'), message);
}
function assertRelayPaths(entry) {
  assert(entry.paths?.length > 0, 'actual member transport paths must be recorded');
  assert(entry.paths.every(path => path.relay === relay && path.isRelay), 'all actual paths must use the declared relay');
  assert(entry.paths.some(path => path.selected && path.sentBytes > 0 && path.receivedBytes > 0), 'selected relay path must carry both directions');
}

try {
  native = nativeFixture();
  const ready = await native.wait('ready');
  web = createServer((request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    const file = resolve(root, 'browser-probe', `.${pathname}`);
    if (!(pathname === '/network.html' || (pathname.startsWith('/pkg/') && file.startsWith(`${root}/browser-probe/pkg/`)))) return response.writeHead(404).end();
    response.setHeader('Content-Type', pathname.endsWith('.wasm') ? 'application/wasm' : pathname.endsWith('.html') ? 'text/html' : 'text/javascript');
    createReadStream(file).on('error', () => response.writeHead(404).end()).pipe(response);
  });
  await new Promise(resolveListening => web.listen(0, '127.0.0.1', resolveListening));
  report.origin = `http://127.0.0.1:${web.address().port}`;
  report.executable = executable();
  report.launch = { executablePath: report.executable, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false };
  context = await chromium.launchPersistentContext(join(temporary, 'profile'), report.launch);
  report.browserVersion = context.browser().version();
  context.on('request', request => report.requests.push({ url: request.url(), method: request.method(), atUtc: new Date().toISOString() }));
  const config = { relays: [relay], disabled: [] };
  const url = `${report.origin}/network.html?config=${encodeURIComponent(JSON.stringify(config))}`;
  let page = await newPage();
  const identity = await start(page, url);
  assert.notEqual(identity, ready.id);
  assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'manual request'), 'AwaitingApproval');
  assert.equal((await native.command('approve', 'approved')).count, 2);
  assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'approved request'), 'PendingMaterials');
  assert.equal(await bounded(page.evaluate(() => window.network.complete_admission()), 'durable admission'), 'Complete');
  const status = JSON.parse(await page.evaluate(() => window.network.status()));
  assert.equal(status.authorization_saved, true);
  assert.equal(status.secret_saved, true);
  const business = native.wait('business', true);
  await exchange(page, 'official-public-relay-member-message');
  assertRelayPaths(await business);
  const authored = await native.command('record', 'record');
  const replica = native.wait('replica', true);
  const outcome = JSON.parse(await bounded(page.evaluate(id => window.network.sync_replica(id), ready.id), 'LogSync durable save'));
  assert.equal(outcome.complete, true);
  assert.equal(outcome.source, ready.id);
  assert(outcome.scope.includes(authored.record));
  assertRelayPaths(await replica);
  const record = JSON.parse(await page.evaluate(id => window.network.control_record(id), authored.record));
  assert.equal(record.body, Buffer.from('original-author browser replica body').toString('hex'));
  await page.reload();
  assert.equal(await bounded(page.evaluate(() => window.ready), 'refresh'), identity);
  assert.deepEqual(JSON.parse(await page.evaluate(id => window.network.control_record(id), authored.record)), record);
  const refreshed = JSON.parse(await page.evaluate(() => window.network.status()));
  assert.equal(refreshed.stage, 'Complete');
  assert.equal(refreshed.authorization_saved && refreshed.secret_saved, true);
  await exchange(page, 'after-official-relay-refresh');
  const before = await native.command('status', 'status');
  assert.equal(before.count, 3);
  report.scenarios.push({ case: 'first-real-public-check', identity, native: ready.id, status, LogSync: outcome, record: authored.record, reload: true, count: before.count });
  save();

  await page.close();
  page = await newPage();
  const disabledUrl = new URL(url);
  disabledUrl.searchParams.set('config', JSON.stringify({ relays: [relay], disabled: [relay] }));
  const requestOffset = report.requests.length;
  const socketOffset = report.websockets.length;
  const failureStart = Date.now();
  await assert.rejects(() => start(page, disabledUrl.href), /temporarily unreachable: no allowed browser relay/);
  const failureMilliseconds = Date.now() - failureStart;
  assert(failureMilliseconds < 15000, 'disabled relay must fail promptly');
  await assert.rejects(() => start(page, `${report.origin}/network.html?config=%7B%7D`), /temporarily unreachable: no allowed browser relay/);
  assert.equal(report.websockets.length, socketOffset, 'disabled and persisted-disabled starts must make no WSS attempt');
  assert(report.requests.slice(requestOffset).every(request => new URL(request.url).origin === report.origin), 'disabled startup must use no external facility');
  assert.equal(await page.evaluate(() => document.querySelector('#exchange').disabled), true);
  assert.match(await page.locator('#status').textContent(), /temporarily unreachable/);
  await page.close();
  page = await newPage();
  assert.equal(await start(page, url), identity);
  assert.deepEqual(JSON.parse(await page.evaluate(id => window.network.control_record(id), authored.record)), record);
  assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
  const recovered = native.wait('business', true);
  await exchange(page, 'same-member-after-explicit-relay-restore');
  assertRelayPaths(await recovered);
  const after = await native.command('status', 'status');
  assert.equal(after.count, before.count);
  assert.equal(after.requests, before.requests);
  assert.deepEqual(after.members, before.members);
  assert.equal(report.native.filter(entry => entry.event === 'approved').length, 1);
  report.scenarios.push({ case: 'selected-official-relay-disable-and-reopen', failureMilliseconds, externalAttemptsWhileDisabled: 0, persistedDisable: true, identity, sameMembership: true, count: after.count, noNewApproval: true });
  assert(report.requests.every(request => new URL(request.url).origin === report.origin), 'static origin must not proxy client network operations');
  assert(report.websockets.length > 0);
  assert(report.websockets.every(socket => socket.url.startsWith(relay.replace(/^https:/, 'wss:'))), 'no alternate facility accessed');
  assert(report.websockets.some(socket => socket.status === 101 && socket.sentBytes > 0 && socket.receivedBytes > 0), 'normal-trust WSS must transfer real packets');
  report.passed = true;
  console.log(`RESULT ${JSON.stringify(report.scenarios)}`);
} catch (error) {
  report.passed = false;
  report.failure = error.stack;
  console.error(`CHECK_FAILED ${error.stack}`);
  process.exitCode = 1;
} finally {
  await context?.close();
  if (web) await new Promise(resolveClose => web.close(resolveClose));
  if (native && native.process.exitCode === null && native.process.signalCode === null) {
    const exited = new Promise(resolveExit => native.process.once('exit', (code, signal) => resolveExit({ code, signal })));
    native.process.stdin.write('stop\n');
    native.process.stdin.end();
    const timer = setTimeout(() => native.process.kill('SIGKILL'), 15000);
    report.cleanup = await exited;
    clearTimeout(timer);
    console.log(`CLEANUP ${JSON.stringify(report.cleanup)}`);
    if (report.cleanup.code !== 0) { report.passed = false; process.exitCode = 1; }
  }
  rmSync(temporary, { recursive: true, force: true });
  report.completedUtc = new Date().toISOString();
  save();
}
