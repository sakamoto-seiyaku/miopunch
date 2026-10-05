import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFileSync, writeFileSync, existsSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {chromium} from '/home/js/Git/miopunch/browser-probe/harness/node_modules/playwright-core/index.mjs';
import {startMixedMqttProxy} from '/home/js/Git/miopunch/browser-probe/harness/owned-mixed-mqtt-proxy.mjs';
import {rtcCapabilityPrelude} from '/home/js/Git/miopunch/browser-probe/harness/rtc-capability-fixture.mjs';

const arg = name => process.argv[process.argv.indexOf(name) + 1];
const fixture = resolve(arg('--fixture'));
const output = resolve(arg('--output'));
const pkg = resolve(arg('--package'));
const report = {scope: 'actual owned Chromium Browser SDK startup only; createAnswer removal injection; no Native member, physical/public/NAT/STUN qualification',
  passed: false, startedUtc: new Date().toISOString(), stages: [], cleanup: []};
const servers = [], sockets = new Set();
let context, page, web;
const privateKeys = new Set(['id', 'remote_id', 'local_id', 'key', 'secret', 'authorization', 'request', 'route', 'checkpoint']);
const safe = value => Array.isArray(value) ? value.map(safe) : value && typeof value === 'object'
  ? Object.fromEntries(Object.entries(value).filter(([key]) => !privateKeys.has(key)).map(([key, child]) => [key, safe(child)])) : value;
const save = () => writeFileSync(join(output, 'receipt.json'), JSON.stringify(safe(report), null, 2) + '\n');
let rejectWork;
const cancelled = new Promise((_, reject) => { rejectWork = reject; });
cancelled.catch(() => {});
const timer = setTimeout(() => rejectWork(new Error('owned startup fixture deadline')), 45000);
for (const name of ['SIGTERM', 'SIGINT']) process.once(name, () => rejectWork(new Error('owned startup cancelled')));
const bounded = promise => Promise.race([promise, cancelled]);
async function stage(name, action) {
  const start = performance.now();
  try {
    const value = await bounded(action());
    report.stages.push({name, ok: true, elapsedMs: performance.now() - start}); save(); return value;
  } catch (error) {
    report.stages.push({name, ok: false, elapsedMs: performance.now() - start, errorClass: error.name}); save(); throw error;
  }
}
async function closeSdk(label) {
  const immediate = await page.evaluate(async () => {
    if (!window.network) return null;
    await window.network.close();
    const rtc = JSON.parse(window.network.rtc_invalidation_diagnostics());
    const objects = window.__mio76RtcFixture.snapshot();
    return {rtc, objects};
  });
  await new Promise(resolve => setTimeout(resolve, 100));
  const late = await page.evaluate(() => window.network ? {
    rtc: JSON.parse(window.network.rtc_invalidation_diagnostics()), objects: window.__mio76RtcFixture.snapshot(),
  } : null);
  if (late) {
    assert.equal(late.rtc.activeAssociations, 0); assert.equal(late.rtc.activePumpTasks, 0);
    assert.equal(late.objects.openPeerConnections, 0); assert.equal(late.objects.openDataChannels, 0);
    assert.equal(late.objects.openWebSockets, 0);
  }
  await page.evaluate(() => { if (window.network) { window.network.free(); window.network = null; } });
  report.cleanup.push({label, closed: true, immediate, late});
}
try {
  const facility = await startMixedMqttProxy({fixture, broker: arg('--broker'), report,
    ownedSockets: sockets, dropPublish: () => false, listen: async server => {
      servers.push(server);
      await bounded(new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); }));
      return server.address().port;
    }});
  web = createServer((request, response) => {
    const path = new URL(request.url, 'http://localhost').pathname;
    if (path === '/') { response.setHeader('Content-Type', 'text/html'); return response.end('<!doctype html><title>owned createAnswer startup</title>'); }
    if (!path.startsWith('/pkg/') || path.includes('..')) return response.writeHead(404).end();
    const file = join(pkg, path.slice(5));
    if (!existsSync(file)) return response.writeHead(404).end();
    response.setHeader('Content-Type', file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript'); response.end(readFileSync(file));
  });
  await bounded(new Promise(resolve => web.listen(0, '127.0.0.1', resolve)));
  const origin = `http://127.0.0.1:${web.address().port}`;
  const executable = '/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome';
  const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
  const wrapper = join(fixture, 'chromium-private-nss.sh');
  writeFileSync(wrapper, `#!/bin/sh\nexec rtk proxy bwrap --ro-bind / / --tmpfs /tmp --bind ${quote(fixture)} ${quote(fixture)} --bind ${quote(join(fixture, 'nssdb'))} /home/js/.pki/nssdb --dev-bind /dev /dev --proc /proc -- ${quote(executable)} "$@"\n`, {mode: 0o700});
  context = await stage('owned-browser-context', () => chromium.launchPersistentContext(join(fixture, 'browser-profile'), {
    executablePath: wrapper, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false,
  }));
  await context.addInitScript(rtcCapabilityPrelude('observe'));
  page = await context.newPage(); await bounded(page.goto(origin));
  await page.evaluate(async () => { const module = await import('/pkg/miopunch_browser_probe.js'); await module.default(); window.BrowserNetwork = module.BrowserNetwork; });
  const configuration = {relays: [], disabled: [], irc: [], mqtt: [facility], contact_routes: [], rtc_enabled: false,
    rtc_ice: {stun_servers: [], local_bind_addresses: [], candidate_addresses: []}};
  const before = await stage('sdk-own-identity-reservation', () => page.evaluate(async configuration => {
    window.network = await window.BrowserNetwork.start(JSON.stringify(configuration));
    return {id: window.network.id(), status: JSON.parse(window.network.status()), paths: JSON.parse(window.network.path_diagnostics())};
  }, configuration));
  report.baseline = before;
  await closeSdk('baseline-sdk');
  await page.evaluate(() => window.__mio76RtcFixture.resetCounts());
  report.injection = await page.evaluate(() => {
    const prototype = window.RTCPeerConnection.prototype;
    window.__ownedCreateAnswerDescriptor = Object.getOwnPropertyDescriptor(prototype, 'createAnswer');
    if (typeof prototype.createAnswer !== 'function' || !window.__ownedCreateAnswerDescriptor) throw new Error('real createAnswer API is unavailable before injection');
    const deleted = delete prototype.createAnswer;
    return {ownedRuntime: 'real Chromium RTCPeerConnection prototype', deleted,
      beforeCreateAnswer: 'function', afterCreateAnswer: typeof prototype.createAnswer,
      createOffer: typeof prototype.createOffer, createDataChannel: typeof prototype.createDataChannel};
  });
  assert.equal(report.injection.deleted, true); assert.equal(report.injection.afterCreateAnswer, 'undefined');
  const observed = await stage('sdk-create-answer-missing-startup', () => page.evaluate(async configuration => {
    window.network = await window.BrowserNetwork.start(JSON.stringify(configuration));
    return {id: window.network.id(), status: JSON.parse(window.network.status()), paths: JSON.parse(window.network.path_diagnostics()),
      rtc: JSON.parse(window.network.rtc_invalidation_diagnostics()), objects: window.__mio76RtcFixture.snapshot()};
  }, {...configuration, rtc_enabled: true}));
  report.observed = observed;
  report.identityPreserved = before.id === observed.id;
  report.savedStatusPreserved = ['stage', 'authorization_saved', 'secret_saved'].every(key => before.status[key] === observed.status[key]);
  report.materialScope = 'owned persistent identity/reservation only; authorization and secret absent before and after; no approved member material qualification';
  assert.equal(report.identityPreserved, true); assert.equal(report.savedStatusPreserved, true);
  assert.equal(observed.status.running, true); assert.equal(observed.rtc.activeAssociations, 0); assert.equal(observed.rtc.activePumpTasks, 0);
  assert.equal(observed.objects.openPeerConnections, 0); assert.equal(observed.objects.openDataChannels, 0);
  const capability = observed.paths.rtcCapability;
  assert.equal(capability.supported, false); assert.equal(capability.permitted, true); assert.equal(capability.effective, false);
  assert.equal(capability.reason, 'api-missing'); assert.equal(observed.objects.peerCreated, 0); assert.equal(observed.objects.channelCreated, 0);
  report.passed = true;
} catch (error) {
  report.failure = {class: error.name, observation: String(error).replace(/\b[0-9a-f]{32,128}\b/gi, '<private>')};
} finally {
  clearTimeout(timer);
  if (page) {
    try {
      await closeSdk('injected-sdk');
      report.ownedDatabaseDeleted = await page.evaluate(async () => {
        if (window.__ownedCreateAnswerDescriptor) Object.defineProperty(window.RTCPeerConnection.prototype, 'createAnswer', window.__ownedCreateAnswerDescriptor);
        window.__mio76RtcFixture?.restore();
        return new Promise((resolve, reject) => { const request = indexedDB.deleteDatabase('miopunch-browser-network'); request.onsuccess = () => resolve(true); request.onerror = request.onblocked = () => reject(new Error('owned database deletion incomplete')); });
      });
    } catch (error) { report.cleanup.push({label: 'browser-sdk', closed: false, errorClass: error.name}); report.passed = false; }
  }
  if (context) { try { await context.close(); report.cleanup.push({label: 'browser-context', closed: true}); } catch { report.cleanup.push({label: 'browser-context', closed: false}); report.passed = false; } }
  for (const socket of sockets) socket.destroy();
  for (const server of servers) await new Promise(resolve => server.close(resolve));
  if (web) await new Promise(resolve => web.close(resolve));
  report.cleanup.push({label: 'local-services', serversClosed: true, socketSetEmpty: sockets.size === 0});
  if (sockets.size !== 0) report.passed = false;
  report.completedUtc = new Date().toISOString(); save();
}
console.log(JSON.stringify({passed: report.passed, capability: report.observed?.paths?.rtcCapability,
  identityPreserved: report.identityPreserved, receipt: join(output, 'receipt.json')}));
process.exitCode = report.passed ? 0 : 1;
