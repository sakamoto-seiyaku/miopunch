import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, mkdtempSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const argument = name => { const index = process.argv.indexOf(name); assert(index >= 0, `${name} required`); return process.argv[index + 1]; };
const evidence = resolve(argument('--evidence'));
const requestedRelay = argument('--relay');
let relay = requestedRelay;
assert(requestedRelay === 'owned' || ['aps1-1', 'use1-1', 'usw1-1', 'euc1-1'].some(region => relay === `https://${region}.relay.n0.iroh.link./`));
const temporary = requestedRelay === 'owned' ? join(argument('--fixture'), 'runtime') : mkdtempSync(join(tmpdir(), 'mio-rtc-s4-'));
mkdirSync(temporary, { recursive: true });
const report = { scope: 'step 4 synthetic RTC; signaling via authenticated protected relay M; no QUIC bridge', startedUtc: new Date().toISOString(), relay, iceServers: [], turnServers: [], native: [], websockets: [], preparation: [], passed: false };
let browser;
let context;
let relayProcess;
let native;
let web;
let page;
let deadline;
let rejectDeadline;
const total = new Promise((_, reject) => { rejectDeadline = reject; deadline = setTimeout(() => reject(new Error('RTC fixture 45 second deadline; 15 seconds reserved for finally cleanup')), 45000); });
// Attach immediately: synchronous startup errors still proceed through finally.
total.catch(() => {});
for (const name of ['SIGTERM', 'SIGINT']) process.once(name, () => rejectDeadline(new Error(`RTC fixture cancelled by ${name}`)));
function save() { writeFileSync(join(evidence, 'rtc-channel.json'), `${JSON.stringify(report, null, 2)}\n`); }
function bounded(promise) { return Promise.race([promise, total]); }
async function preparation(name, action) {
  const started = Date.now();
  try { const value = await bounded(action()); report.preparation.push({ name, startedUtc: new Date(started).toISOString(), milliseconds: Date.now()-started, passed: true }); return value; }
  catch (error) { report.preparation.push({ name, startedUtc: new Date(started).toISOString(), milliseconds: Date.now()-started, passed: false, error: String(error) }); throw error; }
}
function executable() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE) return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
  if (existsSync(chromium.executablePath())) return chromium.executablePath();
  for (const name of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(name => name.startsWith('chromium-')).sort().reverse()) {
    for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const path = join(homedir(), '.cache/ms-playwright', name, suffix);
      if (existsSync(path)) return path;
    }
  }
  throw new Error('install pinned Playwright Chromium');
}
function fixture() {
  const process = spawn('rtk', ['proxy', argument('--native'), join(temporary, 'admin'), relay, ...(requestedRelay === 'owned' ? [join(argument('--fixture'), 'ca.der')] : [])], { stdio: ['pipe', 'pipe', 'pipe'] });
  const entries = [];
  const waiters = [];
  createInterface({ input: process.stdout }).on('line', line => {
    let entry;
    try { entry = JSON.parse(line); } catch { return; }
    entries.push(entry);
    // Invitation is temporary setup material; never retain it in experiment evidence.
    const { invitation, ...publicEntry } = entry;
    report.native.push({ ...publicEntry, receivedUtc: new Date().toISOString() });
    for (const waiter of [...waiters]) if (entry.event === waiter.event) {
      waiters.splice(waiters.indexOf(waiter), 1);
      waiter.resolve(entry);
    }
  });
  process.stderr.on('data', data => console.error(String(data)));
  process.once('exit', (code, signal) => { for (const waiter of waiters.splice(0)) waiter.reject(new Error(`native exited ${code}/${signal} before ${waiter.event}`)); });
  process.once('error', error => { rejectDeadline(error); });
  function wait(event, fresh = false) {
    const existing = !fresh && entries.find(entry => entry.event === event);
    if (existing) return Promise.resolve(existing);
    if (process.exitCode !== null || process.signalCode !== null) return Promise.reject(new Error('native fixture exited'));
    let waiter;
    return bounded(new Promise((resolve, reject) => { waiter = { event, resolve, reject }; waiters.push(waiter); })).finally(() => { const index = waiters.indexOf(waiter); if (index >= 0) waiters.splice(index, 1); });
  }
  return { process, wait, command(value, event) { const result = wait(event, true); process.stdin.write(`${value}\n`); return result; } };
}
try {
  if (requestedRelay === 'owned') {
    const trustFixture = argument('--fixture');
    relayProcess = spawn('rtk', ['proxy', argument('--relay-fixture'), 'relay-tls-files', join(trustFixture, 'leaf.der'), join(trustFixture, 'leaf-key.der')], { stdio: ['pipe', 'pipe', 'pipe'] });
    relayProcess.stderr.on('data', data => console.error(String(data)));
    relay = await bounded(new Promise((resolveReady, rejectReady) => {
      createInterface({ input: relayProcess.stdout }).on('line', line => { if (line.startsWith('RELAY ')) resolveReady(line.slice(6)); });
      relayProcess.once('error', rejectReady);
      relayProcess.once('exit', (code, signal) => rejectReady(new Error(`owned relay exited ${code}/${signal}`)));
    }));
    report.relay = relay;
    report.ownedRelay = true;
  }
  native = fixture();
  const ready = await native.wait('ready');
  web = createServer((request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    const file = resolve(root, 'browser-probe', `.${pathname}`);
    if (!(pathname === '/network.html' || (pathname.startsWith('/pkg/') && file.startsWith(`${root}/browser-probe/pkg/`)))) return response.writeHead(404).end();
    response.setHeader('Content-Type', pathname.endsWith('.wasm') ? 'application/wasm' : pathname.endsWith('.html') ? 'text/html' : 'text/javascript');
    createReadStream(file).on('error', () => response.writeHead(404).end()).pipe(response);
  });
  await bounded(new Promise(resolveListening => web.listen(0, '127.0.0.1', resolveListening)));
  report.origin = `http://127.0.0.1:${web.address().port}`;
  let browserExecutable = executable();
  if (requestedRelay === 'owned') {
    const trustFixture = argument('--fixture');
    const legacyDb = join(homedir(), '.pki/nssdb');
    const defaultDb = join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local/share'), 'pki/nssdb');
    const nssTarget = existsSync(legacyDb) ? legacyDb : defaultDb;
    assert(existsSync(nssTarget), 'existing NSS mountpoint required; do not alter user trust');
    const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
    const wrapper = join(trustFixture, 'chromium-private-trust.sh');
    writeFileSync(wrapper, `#!/bin/sh\nexec rtk proxy bwrap --ro-bind / / --tmpfs /tmp --bind ${quote(trustFixture)} ${quote(trustFixture)} --bind ${quote(temporary)} ${quote(temporary)} --bind ${quote(join(trustFixture, 'nssdb'))} ${quote(nssTarget)} --dev-bind /dev /dev --proc /proc -- ${quote(browserExecutable)} "$@"\n`, { mode: 0o700 });
    browserExecutable = wrapper;
  }
  report.launch = { executablePath: browserExecutable, headless: true, args: ['--no-sandbox', '--disable-features=WebRtcHideLocalIpsWithMdns'], ignoreHTTPSErrors: false, privateNssMount: requestedRelay === 'owned', certificateBypassFlags: [] };
  context = await bounded(chromium.launchPersistentContext(join(temporary, 'profile'), report.launch));
  browser = context.browser();
  report.browserVersion = browser.version();
  page = await context.newPage();
  page.on('pageerror', error => console.error(String(error)));
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Network.enable');
  const sockets = new Map();
  cdp.on('Network.webSocketCreated', ({ requestId, url }) => { const socket = { url, sentFrames: 0, receivedFrames: 0 }; sockets.set(requestId, socket); report.websockets.push(socket); });
  cdp.on('Network.webSocketHandshakeResponseReceived', ({ requestId, response }) => { const socket = sockets.get(requestId); if (socket) socket.status = response.status; });
  for (const [event, field] of [['Network.webSocketFrameSent', 'sentFrames'], ['Network.webSocketFrameReceived', 'receivedFrames']]) cdp.on(event, ({ requestId }) => { const socket = sockets.get(requestId); if (socket) socket[field]++; });
  await bounded(page.goto(`${report.origin}/network.html?config=${encodeURIComponent(JSON.stringify({ relays: [relay], disabled: [] }))}`));
  await bounded(page.waitForFunction(() => window.ready !== undefined));
  report.browserIdentity = await bounded(page.evaluate(() => window.ready));
  assert.notEqual(report.browserIdentity, ready.id);
  assert.equal(await preparation('request', () => page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation)), 'AwaitingApproval');
  await preparation('approval', () => native.command('approve', 'approved'));
  assert.equal(await preparation('authorization-save', () => page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation)), 'PendingMaterials');
  assert.equal(await preparation('material-save', () => page.evaluate(() => window.network.complete_admission())), 'Complete');
  await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
  report.preparedUtc = new Date().toISOString();
  const attempts = await bounded(page.evaluate(async () => {
    const results = await Promise.allSettled([window.member.rtc_channel_probe(), window.member.rtc_channel_probe()]);
    const fulfilled = results.filter(result => result.status === 'fulfilled');
    const refused = results.filter(result => result.status === 'rejected');
    if (fulfilled.length !== 1 || refused.length !== 1 || !String(refused[0].reason).includes('attempt already active')) throw new Error('concurrent RTC attempt was not refused');
    return { rtc: JSON.parse(fulfilled[0].value), overlapRefused: true };
  }));
  report.rtc = attempts.rtc;
  report.overlapRefused = attempts.overlapRefused;
  await native.wait('rtc');
  for (const endpoint of [report.rtc.browser, report.rtc.native]) {
    assert.equal(endpoint.channel.ordered, false);
    assert.equal(endpoint.channel.maxRetransmits, 0);
    assert.equal(endpoint.channel.negotiated, true);
    assert.equal(endpoint.channel.id, 0);
    assert.equal(endpoint.channel.maxPacketLifeTime, null);
    assert.equal(endpoint.sentBytes, 1200); assert.equal(endpoint.matchedBytes, 1200);
    assert(endpoint.maxMessageSize >= 1200);
    assert.equal(endpoint.bindingMatched, true);
    assert(endpoint.txPeakBytes <= 65536 && endpoint.rxPeakBytes <= 65536);
  }
  assert.notEqual(report.rtc.native.selectedPair.local.typ, 'relay');
  assert.notEqual(report.rtc.native.selectedPair.remote.typ, 'relay');
  assert.equal(report.rtc.native.cleanup.pcClosed, true);
  assert.equal(report.rtc.native.cleanup.txBytes + report.rtc.native.cleanup.rxBytes, 0);
  report.failures = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_failure_probe())));
  assert.equal(report.failures.queueFullRefused, true); assert.equal(report.failures.lateOpenIgnored, true);
  assert(report.failures.events.some(event => event.kind === 'lateOpen' && event.state === 'ignored'));
  assert.equal(report.failures.txBytes + report.failures.rxBytes, 0);
  assert(report.websockets.length > 0 && report.websockets.every(socket => socket.url.startsWith(relay.replace(/^https:/, 'wss:'))));
  assert(report.websockets.some(socket => socket.status === 101 && socket.sentFrames > 0 && socket.receivedFrames > 0));
  report.passed = true;
} catch (error) {
  report.failure = error.stack;
  process.exitCode = 1;
  console.error(error.stack);
} finally {
  clearTimeout(deadline);
  const cleanupStart = Date.now();
  if (page && !page.isClosed()) await page.evaluate(async () => { window.member?.close(); await window.network?.close(); }).catch(() => {});
  await context?.close();
  if (web) { web.closeAllConnections(); await new Promise(resolveClose => web.close(resolveClose)); }
  if (native && native.process.exitCode === null && native.process.signalCode === null) {
    const exited = new Promise(resolveExit => native.process.once('exit', (code, signal) => resolveExit({ code, signal })));
    native.process.stdin.write('stop\n'); native.process.stdin.end();
    const killer = setTimeout(() => native.process.kill('SIGKILL'), 10000);
    report.fixtureCleanup = await exited; clearTimeout(killer);
    if (report.fixtureCleanup.code !== 0) { report.passed = false; process.exitCode = 1; }
  }
  if (relayProcess && relayProcess.exitCode === null && relayProcess.signalCode === null) {
    const exited = new Promise(resolveExit => relayProcess.once('exit', (code, signal) => resolveExit({ code, signal })));
    relayProcess.stdin.write('stop\n'); relayProcess.stdin.end();
    const killer = setTimeout(() => relayProcess.kill('SIGKILL'), 3000);
    report.relayCleanup = await exited; clearTimeout(killer);
    if (report.relayCleanup.code !== 0) { report.passed = false; process.exitCode = 1; }
  }
  rmSync(temporary, { recursive: true, force: true });
  report.cleanupMilliseconds = Date.now()-cleanupStart;
  report.completedUtc = new Date().toISOString(); save();
  console.log(`RESULT ${JSON.stringify({ passed: report.passed, evidence })}`);
}
