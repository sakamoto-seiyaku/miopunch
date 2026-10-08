// Browser-only SDK driver. The host controller owns native/services/Docker;
// this namespace has no daemon socket, host bridge or native-process launcher.
import assert from 'node:assert/strict';
import { createReadStream, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createInterface } from 'node:readline';
import { resolve } from 'node:path';
import { chromium } from 'playwright-core';
import { observeTraffic, traffic, exactRtc, releaseProof, streamProgress, cleanupProof, absent } from './run-rtc-member-replica.mjs';

throw new Error('Historical RTC cross-network fixture is retired with the old protocol. Use the current check_unified_* SDK runners. No test actor was started.');

let configure;
const initial = new Promise(resolve => { configure = resolve; });
const requests = new Map();
let nextRequest = 1;
const input = createInterface({ input: process.stdin });
input.on('line', line => {
  if (line === 'stop') return process.emit('SIGTERM');
  const message = JSON.parse(line);
  if (message.configure) return configure(message.configure);
  const request = requests.get(message.reply);
  assert(request, 'owned controller replied to an unknown call');
  requests.delete(message.reply);
  if (message.error) request.reject(new Error(message.error)); else request.resolve(message.result);
});
function rpc(method, payload = {}) {
  const id = nextRequest++;
  const response = new Promise((resolve, reject) => requests.set(id, { resolve, reject }));
  console.log(JSON.stringify({ event: 'rpc', id, method, payload }));
  return response;
}
function publicValues(key, value) {
  return typeof value === 'string' && /sdp|ufrag|username|password|icepwd|fingerprint|certificate/i.test(key) ? undefined : value;
}
const configuration = await initial;
const report = { scope: 'STEP11 controlled separated IPv4 networks with one permissive NAT per endpoint; not physical WAN/cellular',
  mode: configuration.mode, startedUtc: new Date().toISOString(), passed: false, normalVerification: true,
  certificateBypassFlags: [], turnServers: [], dataset: configuration.sourceRecord ?? null,
  wire: [], rtcObjects: 0, metrics: {} };
let context;
let page;
const web = createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const path = resolve('/repo/browser-probe', `.${pathname}`);
  if (!(pathname === '/network.html' || (pathname.startsWith('/pkg/') && path.startsWith('/repo/browser-probe/pkg/')))) return response.writeHead(404).end();
  response.setHeader('Content-Type', pathname.endsWith('.wasm') ? 'application/wasm' : pathname.endsWith('.html') ? 'text/html' : 'text/javascript');
  createReadStream(path).on('error', () => response.writeHead(404).end()).pipe(response);
});
let rejectDeadline;
const ended = new Promise((_, reject) => { rejectDeadline = reject; });
ended.catch(() => {});
const timer = setTimeout(() => rejectDeadline(new Error('owned STEP11 work deadline')), configuration.workMs);
for (const name of ['SIGTERM', 'SIGINT']) process.once(name, () => rejectDeadline(new Error(`owned browser cancelled by ${name}`)));
const bounded = value => Promise.race([value, ended]);
function save() { writeFileSync('/evidence/rtc-cross-network.json', JSON.stringify(report, publicValues, 2)+'\n'); }
async function startPage(config) {
  await bounded(page.goto(`http://127.0.0.1:38080/network.html?config=${encodeURIComponent(JSON.stringify(config))}`));
  await bounded(page.waitForFunction(() => window.ready !== undefined));
  return bounded(page.evaluate(() => window.ready));
}
async function closeSdk() {
  await bounded(page.evaluate(async () => { window.member?.close(); await window.network?.close(); window.network?.free(); window.network = undefined; window.member = undefined; }));
}
async function nativeCommand(command, event) { return bounded(rpc('native-command', { command, event })); }
async function nativeWait(event) { return bounded(rpc('native-wait', { event })); }
function exactRelay(paths) {
  assert(paths.length > 0 && paths.some(path => path.selected));
  assert(paths.every(path => path.isRelay && !path.isRtc && !path.isSignal));
}

try {
  await bounded(new Promise(resolve => web.listen(38080, '127.0.0.1', resolve)));
  context = await bounded(chromium.launchPersistentContext('/fixture/runtime/profile', {
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE, headless: true,
    args: ['--no-sandbox', '--disable-features=WebRtcHideLocalIpsWithMdns'],
    env: { ...process.env, LD_LIBRARY_PATH: '/opt/hostlib' }, ignoreHTTPSErrors: false,
  }));
  report.browserVersion = context.browser().version();
  page = await context.newPage();
  page.on('pageerror', error => console.error(String(error)));
  const cdp = await context.newCDPSession(page);
  await cdp.send('Network.enable');
  const sockets = new Map();
  cdp.on('Network.webSocketCreated', ({ requestId, url }) => {
    const row = { url, sentPayloadBytes: 0, receivedPayloadBytes: 0, sentFrames: 0, receivedFrames: 0 };
    sockets.set(requestId, row); report.wire.push(row);
  });
  cdp.on('Network.webSocketHandshakeResponseReceived', ({ requestId, response }) => { sockets.get(requestId).status = response.status; });
  for (const [event, direction] of [['Network.webSocketFrameSent', 'sent'], ['Network.webSocketFrameReceived', 'received']]) {
    cdp.on(event, ({ requestId, response }) => {
      const row = sockets.get(requestId); row[`${direction}Frames`]++;
      row[`${direction}PayloadBytes`] += response.opcode === 2 ? Buffer.from(response.payloadData, 'base64').length : Buffer.byteLength(response.payloadData);
    });
  }
  if (configuration.mode === 'prepare') {
    const native = await bounded(rpc('native-start', { kind: 'prepare' }));
    report.browserIdentity = await startPage({ relays: [configuration.relay], disabled: [], mqtt: [], irc: [], contact_routes: [] });
    assert.notEqual(report.browserIdentity, native.id);
    assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), native.invitation)), 'AwaitingApproval');
    await nativeCommand('approve', 'approved');
    assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), native.invitation)), 'PendingMaterials');
    assert.equal(await bounded(page.evaluate(() => window.network.complete_admission())), 'Complete');
    report.savedMaterial = JSON.parse(await page.evaluate(() => window.network.status()));
    await closeSdk();
    await bounded(rpc('native-stop'));
    await bounded(rpc('native-start', { kind: 'record' }));
    report.sourceRecord = await nativeCommand('record', 'record');
    assert.equal(report.sourceRecord.body, Buffer.from('original-author browser replica body').toString('hex'));
    assert.equal(report.sourceRecord.applied, true); assert.equal(report.sourceRecord.author, native.id);
    await bounded(rpc('native-stop'));
    report.preparationOnly = true;
  } else {
    const native = await bounded(rpc('native-start', { kind: configuration.mode === 'relay' ? 'relay' : 'direct' }));
    report.nativeReady = native;
    if (configuration.mode !== 'relay') {
      const facility = configuration.browser.mqtt[0];
      configuration.browser.contact_routes = [{ contact: native.id, facility: facility.logical_id, address: native.route }];
      assert.equal(native.ipAddresses, 0); assert.equal(native.relayAddresses, 0);
    }
    const beforeConnect = performance.now();
    report.connectStartedUtc = new Date().toISOString();
    report.browserIdentity = await startPage(configuration.browser);
    report.metrics.clientStartupMs = performance.now()-beforeConnect;
    report.materialBefore = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.equal(report.materialBefore.stage, 'Complete');
    await absent(page, configuration.sourceRecord.record);
    report.targetAbsentBeforeSync = true;
    await observeTraffic(page, true);
    const caller = performance.now();
    if (configuration.mode === 'blocked') {
      try {
        await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, native.id));
        report.unexpectedHandle = true;
      } catch (error) { report.connectionFailure = String(error); }
      report.metrics.memberCallerMs = performance.now()-caller;
      assert.equal(report.unexpectedHandle, undefined);
      assert(report.connectionFailure && /deadline|unreachable|closed/i.test(report.connectionFailure));
      report.materialAfter = JSON.parse(await page.evaluate(() => window.network.status()));
      assert.equal(report.materialAfter.stage, 'Complete');
      report.nativeImmediatelyAfterCaller = await nativeCommand('status', 'status');
      assert.equal(report.nativeImmediatelyAfterCaller.activeRtcPumps, 0);
      assert.equal(report.nativeImmediatelyAfterCaller.publishedRtcRoutes.length, 0);
      report.noOrdinaryRelease = await bounded(rpc('no-ordinary-release'));
      assert.equal(report.noOrdinaryRelease, true);
      const cleanupStarted = performance.now();
      // Caller failure is already final. Retirement is asynchronous and this
      // bounded observation cannot renew contact/member/maintenance budgets.
      report.blockedCleanup = await bounded(rpc('blocked-cleanup'));
      report.metrics.nativeCleanupAfterCallerMs = performance.now()-cleanupStarted;
      assert.equal(report.blockedCleanup.switched, false);
      cleanupProof(report.blockedCleanup);
      assert.equal(report.blockedCleanup.cleanup.dataChannel.actualSentBytes, 0);
      assert.equal(report.blockedCleanup.cleanup.dataChannel.actualReceivedBytes, 0);
      report.nativeAfter = await nativeCommand('status', 'status');
      assert.equal(report.nativeAfter.activeRtcAssociations, 0); assert.equal(report.nativeAfter.activeRtcPumps, 0);
      assert.equal(report.nativeAfter.socketCount, native.socketCount);
      assert.equal(await bounded(rpc('no-ordinary-release')), true);
      report.physical = await traffic(page, true);
      assert(report.physical.every(value => value.closed && value.callbacksDetached));
    } else {
      await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, native.id));
      report.metrics.memberSetupMs = performance.now()-caller;
      report.metrics.clientConnectToHandleMs = performance.now()-beforeConnect;
      report.businessBefore = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
      report.businessReleased = await nativeWait('member-released');
      if (configuration.mode === 'direct') {
        report.businessNativeFinal = await bounded(rpc('rtc-report', { stableId: report.businessReleased.stableId }));
        releaseProof(report.businessBefore.bootstrap, report.businessNativeFinal);
        exactRtc(report.businessBefore.paths);
        assert.equal(report.businessReleased.alpn, 'miopunch/browser-member/rtc-path/3');
      } else {
        exactRelay(report.businessBefore.paths); assert.equal(report.businessReleased.alpn, 'miopunch/browser-member/1');
      }
      const first = performance.now();
      report.businessPayloadStartedUtc = new Date().toISOString();
      report.firstReply = await bounded(page.evaluate(() => window.member.exchange('PING')));
      report.metrics.firstReplyFromClientConnectMs = performance.now()-beforeConnect;
      report.metrics.firstReplyAfterHandleMs = performance.now()-first;
      assert.equal(report.firstReply, 'PONG');
      report.businessHandler = await nativeWait('business');
      assert.equal(report.businessHandler.memberMessage, true); assert.equal(report.businessHandler.reply, 'PONG');
      report.metrics.steadyMemberRttMs = [];
      for (let index = 0; index < 5; index++) {
        const started = performance.now();
        assert.equal(await bounded(page.evaluate(() => window.member.exchange('PING'))), 'PONG');
        report.metrics.steadyMemberRttMs.push(performance.now()-started);
      }
      report.businessAfter = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
      report.businessPayloadEndedUtc = new Date().toISOString();
      assert.equal(report.businessAfter.memberStableId, report.businessBefore.memberStableId);
      if (configuration.mode === 'direct') streamProgress(report.businessBefore.paths, report.businessAfter.paths);
      else exactRelay(report.businessAfter.paths);
      report.physicalBeforeBusinessClose = await traffic(page);
      await bounded(page.evaluate(() => window.member.close()));
      if (configuration.mode === 'direct') {
        report.businessCleanup = await nativeWait('rtc-path-cleanup'); cleanupProof(report.businessCleanup);
      }
      const syncStarted = performance.now();
      report.replicaOperationStartedUtc = new Date().toISOString();
      report.replica = JSON.parse(await bounded(page.evaluate(id => window.network.sync_replica(id), native.id)));
      report.metrics.replicaSaveInclusiveMs = performance.now()-syncStarted;
      report.replicaOperationEndedUtc = new Date().toISOString();
      report.replicaSession = JSON.parse(await page.evaluate(() => window.network.replica_diagnostics()));
      report.replicaReleased = await nativeWait('member-released');
      report.replicaFinished = await nativeWait('replica-finished');
      if (configuration.mode === 'direct') {
        report.replicaNativeFinal = await bounded(rpc('rtc-report', { stableId: report.replicaReleased.stableId }));
        releaseProof(report.replicaSession.before.bootstrap, report.replicaNativeFinal);
        report.replicaCleanup = await nativeWait('rtc-path-cleanup'); cleanupProof(report.replicaCleanup);
        streamProgress(report.replicaSession.before.paths, report.replicaSession.after.paths);
        streamProgress(report.replicaFinished.before, report.replicaFinished.after);
      } else {
        exactRelay(report.replicaSession.before.paths); exactRelay(report.replicaSession.after.paths);
      }
      assert.equal(report.replica.complete, true); assert.equal(report.replicaFinished.durableReceiptAccepted, true);
      assert.equal(report.replicaSession.stableUnchanged, true); assert.equal(report.replicaSession.bindingUnchanged, true);
      assert.equal(report.replicaSession.before.currentPermission, true); assert.equal(report.replicaSession.after.currentPermission, true);
      assert.notEqual(report.replicaSession.before.stableId, report.businessBefore.memberStableId);
      assert.notEqual(report.replicaReleased.stableId, report.businessReleased.stableId);
      assert(report.replica.scope.includes(configuration.sourceRecord.record));
      assert.deepEqual(report.replica.scope, report.replica.saved); assert.deepEqual(report.replica.scope, report.replica.applied);
      report.receivedRecord = JSON.parse(await page.evaluate(id => window.network.control_record(id), configuration.sourceRecord.record));
      assert.deepEqual(report.receivedRecord, { header: configuration.sourceRecord.header, body: configuration.sourceRecord.body });
      report.metrics.uniqueVerifiedSavedBodyBytes = configuration.sourceRecord.bodyBytes;
      report.metrics.saveInclusiveGoodputBytesPerSecond = configuration.sourceRecord.bodyBytes/(report.metrics.replicaSaveInclusiveMs/1000);
      report.metrics.goodputScope = 'finite36B original record body; includes fresh replica setup, signature verification, durable save/Receipt; noisy, not sustained throughput';
      await bounded(page.waitForFunction(() => window.replicaRtcTraffic.every(value => value.peer.connectionState === 'closed')));
      report.physical = await traffic(page, true);
      if (configuration.mode === 'direct') {
        assert.equal(report.physical.length, 2);
        assert(report.physical.every(value => value.sentBytes > 0 && value.receivedBytes > 0 && value.closed && value.callbacksDetached && value.observersDetached));
      } else assert.equal(report.physical.length, 0, 'relay comparison never constructs RTC');
      await closeSdk();
      const reloaded = await startPage(configuration.browser);
      assert.equal(reloaded, report.browserIdentity);
      report.reopenedRecord = JSON.parse(await page.evaluate(id => window.network.control_record(id), configuration.sourceRecord.record));
      assert.deepEqual(report.reopenedRecord, report.receivedRecord);
      report.materialAfterReopen = JSON.parse(await page.evaluate(() => window.network.status()));
      assert.equal(report.materialAfterReopen.stage, 'Complete');
      assert.equal(JSON.parse(await page.evaluate(() => window.network.control_state())).pending, 0);
      report.nativeQuiescence = await nativeCommand('status', 'status');
      while (report.nativeQuiescence.socketCount !== native.socketCount || report.nativeQuiescence.activeRtcAssociations || report.nativeQuiescence.activeRtcPumps) {
        await bounded(new Promise(resolve => setTimeout(resolve, 20)));
        report.nativeQuiescence = await nativeCommand('status', 'status');
      }
    }
    report.measurementEndedUtc = new Date().toISOString();
    await closeSdk();
    await bounded(rpc('native-stop'));
  }
  assert(report.wire.every(value => value.status === 101));
  report.passed = true;
} catch (error) {
  report.failure = error.stack ?? String(error); process.exitCode = 1;
} finally {
  clearTimeout(timer);
  if (page && !page.isClosed()) await page.evaluate(async () => { window.member?.close(); await window.network?.close(); window.network?.free(); }).catch(() => {});
  await context?.close();
  web.closeAllConnections(); await new Promise(resolve => web.close(resolve));
  input.close();
  report.completedUtc = new Date().toISOString(); save();
  console.log(JSON.stringify({ event: 'browser-complete', passed: report.passed }));
}
