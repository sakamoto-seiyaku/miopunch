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
const memberCase = process.argv.includes('--case') && argument('--case') === 'relay-member-rtc';
const cutCase = process.argv.includes('--case') && argument('--case') === 'rtc-data-cut';
const invalidationCase = process.argv.includes('--case') && argument('--case') === 'rtc-invalidation';
const cutRelay = cutCase && argument('--cut-mode') === 'relay';
const wssMemberCase = process.argv.includes('--case') && (argument('--case') === 'wss-member-rtc' || cutCase || invalidationCase);
const legacyNoRtc = process.argv.includes('--legacy-no-rtc');
const legacyRelay = process.argv.includes('--legacy-relay');
const relayBackup = legacyRelay || cutRelay;
const requestedRelay = argument('--relay');
let relay = requestedRelay;
assert(requestedRelay === 'owned' || ['aps1-1', 'use1-1', 'usw1-1', 'euc1-1'].some(region => relay === `https://${region}.relay.n0.iroh.link./`));
const temporary = requestedRelay === 'owned' ? join(argument('--fixture'), 'runtime') : mkdtempSync(join(tmpdir(), 'mio-rtc-s4-'));
mkdirSync(temporary, { recursive: true });
const report = { scope: memberCase ? 'step 5; admitted Ordinary-M relay to RTC, original QUIC and stream' : 'step 4 synthetic RTC; signaling via authenticated protected relay M; no QUIC bridge', startedUtc: new Date().toISOString(), relay, iceServers: [], turnServers: [], native: [], websockets: [], preparation: [], passed: false };
let browser;
let context;
let relayProcess;
let native;
let web;
let page;
let deadline;
let rejectDeadline;
const total = new Promise((_, reject) => { rejectDeadline = reject; deadline = setTimeout(() => reject(new Error('RTC fixture deadline; reserved finally cleanup')), cutCase || invalidationCase ? Number(process.env.MIO_CUT_WORK_MS ?? 40000) : wssMemberCase ? 55000 : memberCase ? 35000 : 45000); });
const ownedSockets = new Set();
const facilityServers = [];
// Attach immediately: synchronous startup errors still proceed through finally.
total.catch(() => {});
for (const name of ['SIGTERM', 'SIGINT']) process.once(name, () => rejectDeadline(new Error(`RTC fixture cancelled by ${name}`)));
function save() {
  const redact = cutCase || invalidationCase ? (key, value) => typeof value === 'string' && (/sdp|ufrag|username|password|icepwd|certificate|fingerprint/i.test(key) || /^CF[0-9A-F]{2}(:[0-9A-F]{2})+$/i.test(value)) ? undefined : value : undefined;
  writeFileSync(join(evidence, invalidationCase ? 'rtc-invalidation.json' : cutCase ? 'rtc-data-cut.json' : wssMemberCase ? 'wss-member-rtc.json' : memberCase ? 'relay-member-rtc.json' : 'rtc-channel.json'), `${JSON.stringify(report, redact, 2)}\n`);
}
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
function fixture(memberFacilityPath) {
  const process = spawn('rtk', ['proxy', memberFacilityPath ? argument('--wss-native') : argument('--native'), join(temporary, 'admin'), ...(memberFacilityPath ? [memberFacilityPath, join(argument('--fixture'), 'ca.pem'), ...(legacyNoRtc ? ['legacy-no-rtc'] : legacyRelay ? ['legacy-relay', relay] : cutRelay ? ['rtc-relay', relay] : [])] : [relay, ...(requestedRelay === 'owned' ? [join(argument('--fixture'), 'ca.der')] : [])])], { stdio: ['pipe', 'pipe', 'pipe'] });
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
async function stopProcess(child, label) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolveExit => child.once('exit', (code, signal) => resolveExit({ code, signal })));
  child.stdin.end('stop\n');
  const killer = setTimeout(() => child.kill('SIGKILL'), 3000);
  const result = await exited; clearTimeout(killer);
  assert.equal(result.code, 0, `${label} must close cleanly`);
  return result;
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
  if (requestedRelay === 'owned' && process.env.MIO_PRIVATE_NSS_NAMESPACE !== '1') {
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
  const launchOptions = { ...report.launch };
  if (process.env.MIO_PRIVATE_NSS_NAMESPACE === '1') {
    launchOptions.env = { ...process.env, LD_LIBRARY_PATH: '/opt/hostlib' };
    report.chromiumRuntimeLibraries = '/opt/hostlib; namespace fixture only';
  }
  context = await bounded(chromium.launchPersistentContext(join(temporary, 'profile'), launchOptions));
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
  if (!wssMemberCase) await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
  report.preparedUtc = new Date().toISOString();
  await page.evaluate(() => {
    const Original = window.RTCPeerConnection;
    window.rtcObjects = [];
    window.RTCPeerConnection = class extends Original {
      constructor(...args) { super(...args); window.rtcObjects.push(this); }
    };
  });
  if (wssMemberCase) {
    report.scope = 'step 6; known saved-member MQTT/WSS Control-M admission to exact RTC';
    report.savedBefore = JSON.parse(await page.evaluate(() => window.network.status()));
    await bounded(page.evaluate(async () => { await window.network.close(); window.network.free(); window.network = null; }));
    report.preparationNativeClosed = await stopProcess(native.process, 'preparation native');
    if (!relayBackup) {
      report.preparationRelayClosed = await stopProcess(relayProcess, 'preparation relay');
      relayProcess = null;
    } else { report.preparationEndpointsClosedRelayStillExplicitlyPermitted = true; }
    const { startMixedMqttProxy } = await import('./owned-mixed-mqtt-proxy.mjs');
    const facility = await startMixedMqttProxy({ fixture: argument('--fixture'), broker: argument('--broker'), report, ownedSockets,
      dropPublish: () => false, listen: async server => {
        facilityServers.push(server);
        await bounded(new Promise((resolveListening, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolveListening); }));
        return server.address().port;
      } });
    const facilityPath = join(temporary, 'member-facility.json');
    writeFileSync(facilityPath, JSON.stringify(facility));
    native = fixture(facilityPath);
    const memberReady = await native.wait('ready');
    assert.equal(memberReady.id, ready.id);
    assert(memberReady.knownMembers.includes(report.browserIdentity));
    assert.equal(memberReady.ipAddresses, 0);
    assert.equal(memberReady.relayAddresses, relayBackup ? 1 : 0);
    const configuration = { relays: relayBackup ? [relay] : [], disabled: [], irc: [], mqtt: [facility],
      contact_routes: [{ contact: ready.id, facility: facility.logical_id, address: memberReady.route }] };
    let legacyRelayCapabilities;
    if (relayBackup) {
      // Approved at startup; dormant only in this owned fixture until actual
      // MQTT capability exchange, so a relay-started M cannot pass this story.
      await page.evaluate(relayHost => {
        const Original = window.WebSocket;
        const openers = [];
        let released = false;
        window.ownedRelayEvents = [];
        window.releaseOwnedRelay = () => { released = true; for (const open of openers.splice(0)) open(); };
        // This fixture object emits no open/message and has no network socket
        // until release. It then delegates to the real normally verified WS.
        window.WebSocket = class extends EventTarget {
          static CONNECTING = Original.CONNECTING; static OPEN = Original.OPEN;
          static CLOSING = Original.CLOSING; static CLOSED = Original.CLOSED;
          constructor(url, protocols) {
            super();
            if (new URL(url).host !== relayHost) return protocols === undefined ? new Original(url) : new Original(url, protocols);
            this.url = String(url); this.socket = null; this.cancelled = false; this._binaryType = 'blob';
            window.ownedRelayEvents.push({ kind: 'deferred', readyState: this.readyState });
            const open = () => {
              if (this.cancelled) return;
              window.ownedRelayEvents.push({ kind: 'native-constructor' });
              this.socket = protocols === undefined ? new Original(url) : new Original(url, protocols);
              this.socket.binaryType = this._binaryType;
              for (const kind of ['open', 'message', 'error', 'close']) this.socket.addEventListener(kind, event => {
                window.ownedRelayEvents.push({ kind, readyState: this.readyState });
                const copy = kind === 'message' ? new MessageEvent(kind, { data: event.data, origin: event.origin }) :
                  kind === 'close' ? new CloseEvent(kind, { code: event.code, reason: event.reason, wasClean: event.wasClean }) : new Event(kind);
                this.dispatchEvent(copy); this[`on${kind}`]?.call(this, copy);
              });
            };
            if (released) open(); else openers.push(open);
          }
          get readyState() { return this.cancelled ? Original.CLOSED : this.socket?.readyState ?? Original.CONNECTING; }
          get bufferedAmount() { return this.socket?.bufferedAmount ?? 0; }
          get protocol() { return this.socket?.protocol ?? ''; }
          get extensions() { return this.socket?.extensions ?? ''; }
          get binaryType() { return this._binaryType; }
          set binaryType(value) { this._binaryType = value; if (this.socket) this.socket.binaryType = value; }
          send(bytes) { if (!this.socket) throw new DOMException('Owned relay remains dormant', 'InvalidStateError'); this.socket.send(bytes); }
          close(code, reason) {
            this.cancelled = true;
            if (this.socket) this.socket.close(code, reason);
            else queueMicrotask(() => { const event = new CloseEvent('close'); this.dispatchEvent(event); this.onclose?.call(this, event); });
          }
        };
      }, new URL(relay).host);
      if (legacyRelay) {
        legacyRelayCapabilities = native.wait('path-capabilities', true);
        legacyRelayCapabilities.then(async () => {
          report.legacyRelayRevealed = await page.evaluate(() => { window.releaseOwnedRelay(); return window.ownedRelayEvents; });
        }).catch(rejectDeadline);
      }
    }
    report.reopenedIdentity = await bounded(page.evaluate(async configuration => {
      const { BrowserNetwork } = await import('/pkg/miopunch_browser_probe.js');
      window.network = await BrowserNetwork.start(JSON.stringify(configuration));
      return window.network.id();
    }, configuration));
    assert.equal(report.reopenedIdentity, report.browserIdentity);
    report.savedAfterReopen = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.equal(report.savedAfterReopen.stage, 'Complete');
    assert.equal(report.savedAfterReopen.secret_saved, true);
    report.beforeMember = JSON.parse(await page.evaluate(() => window.network.path_diagnostics()));
    if (invalidationCase) {
      const { runRtcInvalidation } = await import('./run-rtc-invalidation.mjs');
      await runRtcInvalidation({ page, native, ready: memberReady, report, bounded, save, mode: argument('--invalidation-mode') });
    } else if (cutCase) {
      const { runRtcDataCut } = await import('./run-rtc-data-cut.mjs');
      await runRtcDataCut({ page, native, ready: memberReady, report, bounded, save, evidence, relayBackup });
    } else if (legacyNoRtc) {
      report.legacyUnreachable = await bounded(page.evaluate(async id => {
        const started = performance.now();
        try { window.unexpectedMember = await window.network.open_member_connection(id); return { returnedHandle: true }; }
        catch (error) { return { returnedHandle: false, failure: String(error), milliseconds: performance.now()-started, peersCreated: window.rtcObjects.length }; }
      }, ready.id));
      assert.equal(report.legacyUnreachable.returnedHandle, false);
      assert.match(report.legacyUnreachable.failure, /temporarily unreachable/);
      assert(report.legacyUnreachable.milliseconds < 3000);
      assert.equal(report.legacyUnreachable.peersCreated, 0);
      report.legacyCapabilities = await native.wait('path-capabilities');
      assert.equal(report.legacyCapabilities.alpn, 'miopunch/browser-member/1');
      assert.equal(report.legacyCapabilities.version, 2);
      assert.equal(report.legacyCapabilities.localRtc, 'None');
      assert.equal(report.legacyCapabilities.peerRtc, 'None');
      report.legacyNativeWithheld = await native.wait('handle-withheld');
      assert.match(report.legacyNativeWithheld.error, /temporarily unreachable/);
      assert(!report.native.some(entry => entry.event === 'member-released'));
      report.legacySaved = JSON.parse(await page.evaluate(() => window.network.status()));
      assert.equal(report.legacySaved.stage, 'Complete');
      assert.equal(report.legacySaved.id, report.browserIdentity);
    } else if (legacyRelay) {
      try {
        await bounded(page.evaluate(async id => { window.legacyMember = await window.network.open_member_connection(id); }, ready.id));
      } catch (error) {
        report.legacyRelayFailure = String(error);
        report.legacyCapabilities = await legacyRelayCapabilities;
        throw error;
      }
      report.legacyCapabilities = await legacyRelayCapabilities;
      assert.equal(report.legacyCapabilities.version, 2);
      assert.equal(report.legacyCapabilities.relay, true);
      assert.equal(report.legacyCapabilities.rtc, false);
      assert(report.legacyCapabilities.initialPaths.every(path => path.isMqtt && !path.isRelay && !path.isIp));
      report.legacyRelayMember = JSON.parse(await page.evaluate(() => window.legacyMember.rtc_path_diagnostics()));
      report.legacyRelayNativeValidation = await native.wait('relay-path');
      for (const side of [report.legacyRelayMember.bootstrap, report.legacyRelayNativeValidation]) {
        assert.equal(side.stableUnchanged, true); assert.equal(side.bindingUnchanged, true);
        const probe = side.validatedRelay ?? side.probe;
        assert.equal(probe.freshAck, true);
        assert(probe.freshAckMs >= probe.freshProbeStartedMs);
        assert(probe.probeWatermark > 0);
        assert(side.relayFramesAfterConfirmation.some(path => path.streamSent + path.streamReceived > 0));
      }
      assert.equal(report.legacyRelayMember.bootstrap.privateStreamId, report.legacyRelayNativeValidation.privateStreamId);
      assert(report.legacyRelayMember.paths.some(path => path.isRelay && path.selected));
      assert(report.legacyRelayMember.paths.every(path => path.isRelay && !path.isRtc));
      assert.equal(await bounded(page.evaluate(() => window.legacyMember.exchange('legacy v2 same-M permitted relay DATA'))), 'legacy v2 same-M permitted relay DATA');
      report.legacyRelayAfterData = JSON.parse(await page.evaluate(() => window.legacyMember.rtc_path_diagnostics()));
      assert(report.legacyRelayAfterData.paths[0].streamFramesSent > report.legacyRelayMember.paths[0].streamFramesSent);
      assert(report.legacyRelayAfterData.paths[0].streamFramesReceived > report.legacyRelayMember.paths[0].streamFramesReceived);
      report.legacyRelayReleased = await native.wait('member-released');
      assert.equal(report.legacyRelayReleased.stableId, report.legacyCapabilities.stableId);
      assert.equal(report.legacyRelayReleased.alpn, 'miopunch/browser-member/1');
      assert.equal(await page.evaluate(() => window.rtcObjects.length), 0);
      await page.evaluate(() => window.legacyMember.close());
      report.legacySaved = JSON.parse(await page.evaluate(() => window.network.status()));
      assert.equal(report.legacySaved.stage, 'Complete');
    } else {
    // This is the tracer-bullet red: both native and browser admission hooks
    // must finish an actual RTC path before this call returns a public handle.
    await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
    report.member = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    for (const side of [report.member.bootstrap, report.member.bootstrap.native]) {
      assert.equal(side.stableUnchanged, true);
      assert.equal(side.bindingUnchanged, true);
      assert.equal(side.freshAck, true);
      assert(side.freshAckMs >= side.freshProbeStartedMs);
      assert(Object.values(side.beforeRetainFrames).every(frames => frames === 0));
    }
    assert.equal(report.member.bootstrap.finalReleaseFreshAck, true);
    assert(report.member.bootstrap.finalReleaseProbeWatermark > report.member.bootstrap.probeWatermark);
    assert(report.member.bootstrap.finalReleaseAckMs >= report.member.bootstrap.finalReleaseProbeStartedMs);
    assert.equal(report.member.bootstrap.rtcRole, 'offerer');
    assert.equal(report.member.bootstrap.native.rtcRole, 'answerer');
    assert.equal(report.member.bootstrap.capabilitiesVersion, 3);
    assert(report.member.paths.some(path => path.isRtc && path.selected));
    assert(report.member.paths.every(path => path.isRtc && !path.isRelay));
    report.carrierBeforeData = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
    assert.equal(await bounded(page.evaluate(() => window.member.exchange('saved member DATA after restricted MQTT RTC setup'))), 'saved member DATA after restricted MQTT RTC setup');
    report.businessAfter = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    report.carrierAfterData = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
    assert(report.carrierAfterData.every(carrier => carrier.ready));
    assert.equal(report.carrierAfterData[0].traffic.sent_bytes, report.carrierBeforeData[0].traffic.sent_bytes);
    assert.equal(report.carrierAfterData[0].traffic.received_bytes, report.carrierBeforeData[0].traffic.received_bytes);
    report.memberReleased = await native.wait('member-released');
    report.nativeFinalRelease = report.native.find(entry => entry.event === 'rtc-path')?.report;
    assert.equal(report.nativeFinalRelease?.finalReleaseFreshAck, true);
    assert(report.nativeFinalRelease.finalReleaseProbeWatermark > report.nativeFinalRelease.probeWatermark);
    assert(report.nativeFinalRelease.finalReleaseAckMs >= report.nativeFinalRelease.finalReleaseProbeStartedMs);
    // The route's retained lifetime must outlive this caller's 12-second setup.
    await bounded(new Promise(resolveWait => setTimeout(resolveWait, 12500)));
    assert.equal(await bounded(page.evaluate(() => window.member.exchange('RTC DATA beyond original setup deadline'))), 'RTC DATA beyond original setup deadline');
    report.afterSetupDeadline = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    assert.equal(report.afterSetupDeadline.memberStableId, report.member.memberStableId);
    assert(report.afterSetupDeadline.paths.some(path => path.isRtc && path.selected));
    await page.evaluate(() => window.member.close());
    report.normalCleanup = await native.wait('rtc-path-cleanup');
    assert.equal(report.normalCleanup.cleanup.pcClosed, true);
    assert.equal(report.normalCleanup.cleanup.ownedPumpTasks, 0);
    report.normalBrowserCleanup = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    assert.equal(report.normalBrowserCleanup.activeAssociations, 0);
    assert.equal(report.normalBrowserCleanup.activePumpTasks, 0);
    // Missing API is deliberately the next red. The separate pending-operation
    // object must cancel this one connect attempt while WASM borrows its future.
    await page.evaluate(id => { window.cancelAttempt = window.network.member_connection_attempt(id, 12000); }, ready.id);
    const cancelledNativeCleanup = native.wait('rtc-path-cleanup', true);
    cancelledNativeCleanup.catch(() => {});
    await page.evaluate(() => {
      // Drop actual datagram sends after RTC channel setup in this owned
      // fixture, while the protected original MQTT stream remains available.
      window.dropRtcPackets = true;
      window.droppedRtcPackets = 0;
      window.droppedOpenRtcPackets = 0;
      const send = RTCDataChannel.prototype.send;
      if (!window.originalRtcSend) {
        window.originalRtcSend = send;
        RTCDataChannel.prototype.send = function(bytes) {
          if (window.dropRtcPackets) { window.droppedRtcPackets++; if (this.readyState === 'open') window.droppedOpenRtcPackets++; return; }
          return window.originalRtcSend.call(this, bytes);
        };
      }
      window.pendingMember = window.cancelAttempt.connect().then(member => { window.unexpectedMember = member; return { returnedHandle: true }; }, error => ({ returnedHandle: false, failure: String(error) }));
    });
    // Loss is active before QUIC can open a path; open-only is insufficient.
    await bounded(page.waitForFunction(() => window.rtcObjects.some(peer => peer.connectionState === 'connected') && window.droppedRtcPackets > 0));
    report.cancelled = await bounded(page.evaluate(async () => {
      const started = performance.now(); window.cancelAttempt.cancel();
      return { ...(await window.pendingMember), milliseconds: performance.now()-started, droppedQuicPackets: window.droppedRtcPackets, droppedWhileOpen: window.droppedOpenRtcPackets };
    }));
    assert.equal(report.cancelled.returnedHandle, false);
    assert.match(report.cancelled.failure, /cancel/i);
    assert(report.cancelled.milliseconds < 1000);
    assert(report.cancelled.droppedQuicPackets > 0);
    assert(report.cancelled.droppedWhileOpen > 0);
    report.cancelledNativeCleanup = await cancelledNativeCleanup;
    assert.equal(report.cancelledNativeCleanup.cleanup.pcClosed, true);
    assert.equal(report.cancelledNativeCleanup.cleanup.ownedPumpTasks, 0);
    assert.equal(report.cancelledNativeCleanup.switched, false);
    await page.evaluate(() => { window.dropRtcPackets = false; window.cancelAttempt.free(); });
    report.savedAfterCancellation = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.equal(report.savedAfterCancellation.stage, 'Complete');
    assert.equal(report.savedAfterCancellation.secret_saved, true);
    assert.equal(report.savedAfterCancellation.running, true);
    assert.equal(report.savedAfterCancellation.id, report.browserIdentity);
    await bounded(page.evaluate(async id => { window.memberAfterCancellation = await window.network.open_member_connection(id); }, ready.id));
    assert.equal(await bounded(page.evaluate(() => window.memberAfterCancellation.exchange('same node retry after pending cancellation'))), 'same node retry after pending cancellation');
    report.retryAfterCancellation = JSON.parse(await page.evaluate(() => window.memberAfterCancellation.rtc_path_diagnostics()));
    const retryCleanup = native.wait('rtc-path-cleanup', true); retryCleanup.catch(() => {});
    await page.evaluate(() => window.memberAfterCancellation.close());
    await retryCleanup;
    // The original 12-second clock covers setup even when the channel opens.
    // Dropping real QUIC packets must never return an application handle.
    const timeoutCleanup = native.wait('rtc-path-cleanup', true); timeoutCleanup.catch(() => {});
    report.openChannelTimeout = await bounded(page.evaluate(async id => {
      window.dropRtcPackets = true; window.droppedRtcPackets = 0; window.droppedOpenRtcPackets = 0;
      const attempt = window.network.member_connection_attempt(id, 12000);
      const started = performance.now();
      let returnedHandle = false, failure;
      try { window.unexpectedMember = await attempt.connect(); returnedHandle = true; }
      catch (error) { failure = String(error); }
      finally { attempt.free(); }
      return { returnedHandle, failure, milliseconds: performance.now()-started, droppedQuicPackets: window.droppedRtcPackets, droppedWhileOpen: window.droppedOpenRtcPackets,
        connectedChannels: window.rtcObjects.filter(peer => peer.connectionState === 'connected').length };
    }, ready.id));
    assert.equal(report.openChannelTimeout.returnedHandle, false);
    assert(report.openChannelTimeout.droppedQuicPackets > 0);
    assert(report.openChannelTimeout.droppedWhileOpen > 0);
    assert(report.openChannelTimeout.milliseconds >= 11000 && report.openChannelTimeout.milliseconds < 13500);
    report.timeoutNativeCleanup = await timeoutCleanup;
    assert.equal(report.timeoutNativeCleanup.switched, false);
    assert.equal(report.timeoutNativeCleanup.cleanup.pcClosed, true);
    assert.equal(report.timeoutNativeCleanup.cleanup.ownedPumpTasks, 0);
    // Creation fixes this shorter clock; delaying connect consumes its budget.
    report.consumedCallerBudget = await bounded(page.evaluate(async id => {
      const attempt = window.network.member_connection_attempt(id, 2000);
      const started = performance.now();
      await new Promise(resolve => setTimeout(resolve, 2200));
      let returnedHandle = false, failure;
      try { await attempt.connect(); returnedHandle = true; } catch (error) { failure = String(error); }
      finally { attempt.free(); }
      return { returnedHandle, failure, milliseconds: performance.now()-started };
    }, ready.id));
    assert.equal(report.consumedCallerBudget.returnedHandle, false);
    assert(report.consumedCallerBudget.milliseconds < 2700);
    await page.evaluate(() => { window.dropRtcPackets = false; });
    // Late browser callbacks cannot revive a closed per-M association.
    await bounded(page.waitForFunction(() => window.rtcObjects.every(peer => peer.connectionState === 'closed' && peer.onconnectionstatechange === null && peer.oniceconnectionstatechange === null)));
    report.closedBrowserPeers = await page.evaluate(() => ({ created: window.rtcObjects.length,
      closed: window.rtcObjects.filter(peer => peer.connectionState === 'closed').length }));
    report.nativeLateCleanup = await native.command('status', 'status');
    assert.equal(report.nativeLateCleanup.socketCount, memberReady.socketCount);
    report.savedAfterTimeout = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.equal(report.savedAfterTimeout.stage, 'Complete');
    await bounded(page.evaluate(async () => { await window.network.close(); window.network.free(); }));
    await bounded(page.goto(`${report.origin}/network.html?config=${encodeURIComponent(JSON.stringify(configuration))}`));
    assert.equal(await bounded(page.evaluate(() => window.ready)), report.browserIdentity);
    report.savedAfterPageReload = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.equal(report.savedAfterPageReload.stage, 'Complete');
    assert.equal(report.savedAfterPageReload.secret_saved, true);
    const replicaReleased = native.wait('member-released', true); replicaReleased.catch(() => {});
    const replicaCleanup = native.wait('rtc-path-cleanup', true); replicaCleanup.catch(() => {});
    report.replica = JSON.parse(await bounded(page.evaluate(id => window.network.sync_replica(id), ready.id)));
    report.replicaReleased = await replicaReleased;
    assert.equal(report.replicaReleased.alpn, 'miopunch/control-replica/rtc-path/3');
    report.replicaBootstrap = JSON.parse(await page.evaluate(() => window.network.path_diagnostics()));
    const replicaBootstraps = Object.values(report.replicaBootstrap.memberBootstraps);
    assert.equal(replicaBootstraps.length, 1);
    assert.equal(replicaBootstraps[0].freshAck, true);
    assert.equal(replicaBootstraps[0].native.freshAck, true);
    report.replicaCleanup = await replicaCleanup;
    assert.equal(report.replicaCleanup.cleanup.pcClosed, true);
    assert.equal(report.replicaCleanup.cleanup.ownedPumpTasks, 0);
    }
  } else if (memberCase) {
    const { runMemberRtc } = await import('./run-rtc-member.mjs');
    await runMemberRtc({ page, native, ready, report, bounded, save });
  } else {
  const attempts = await bounded(page.evaluate(async () => {
    const results = await Promise.allSettled([window.member.rtc_channel_probe(false), window.member.rtc_channel_probe(false)]);
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
  await bounded(page.evaluate(async id => { window.cancelMember = await window.network.open_member_connection(id); }, ready.id));
  const waiting = native.wait('rtc-waiting', true);
  waiting.catch(() => {});
  await bounded(page.evaluate(() => {
    window.pendingCancel = window.cancelMember.rtc_channel_probe(true);
    window.pendingCancel.catch(() => {});
  }));
  await waiting;
  report.cancellation = await bounded(page.evaluate(async () => {
    const started = performance.now();
    window.cancelMember.close();
    let rejection;
    try { await window.pendingCancel; } catch (error) { rejection = String(error); }
    return { milliseconds: performance.now()-started, rejection,
      peersCreated: window.rtcObjects.length,
      peersClosed: window.rtcObjects.filter(peer => peer.connectionState === 'closed').length,
      callbacksDetached: window.rtcObjects.every(peer => peer.onicegatheringstatechange === null && peer.oniceconnectionstatechange === null && peer.onconnectionstatechange === null) };
  }));
  assert.match(report.cancellation.rejection, /signaling M closed/);
  assert(report.cancellation.milliseconds < 1000, 'public close must promptly cancel local RTC RX');
  assert.equal(report.cancellation.peersCreated, report.cancellation.peersClosed);
  assert.equal(report.cancellation.callbacksDetached, true);
  }
  assert(report.websockets.length > 0 && (wssMemberCase || report.websockets.every(socket => socket.url.startsWith(relay.replace(/^https:/, 'wss:')))));
  assert(report.websockets.some(socket => socket.status === 101 && socket.sentFrames > 0 && socket.receivedFrames > 0));
  report.passed = true;
} catch (error) {
  report.failure = error.stack;
  process.exitCode = 1;
  console.error(error.stack);
} finally {
  clearTimeout(deadline);
  const cleanupStart = Date.now();
  // Release only owned diagnostic holds before waiting for normal teardown.
  if (invalidationCase && native && native.process.exitCode === null && native.process.signalCode === null) native.process.stdin.write('control-clear\n');
  if (page && !page.isClosed()) await page.evaluate(async () => { window.member?.close(); window.secondMember?.close(); window.newMember?.close(); window.failedMember?.close(); window.pumpMember?.close(); await window.network?.close(); }).catch(() => {});
  await context?.close();
  if (web) { web.closeAllConnections(); await new Promise(resolveClose => web.close(resolveClose)); }
  if (native && native.process.exitCode === null && native.process.signalCode === null) {
    const exited = new Promise(resolveExit => native.process.once('exit', (code, signal) => resolveExit({ code, signal })));
    native.process.stdin.write('stop\n'); native.process.stdin.end();
    const killer = setTimeout(() => native.process.kill('SIGKILL'), 10000);
    report.fixtureCleanup = await exited; clearTimeout(killer);
    if (report.fixtureCleanup.code !== 0) { report.passed = false; process.exitCode = 1; }
  }
  for (const socket of ownedSockets) socket.destroy();
  for (const server of facilityServers) { server.closeAllConnections?.(); await new Promise(resolveClose => server.close(resolveClose)); }
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
