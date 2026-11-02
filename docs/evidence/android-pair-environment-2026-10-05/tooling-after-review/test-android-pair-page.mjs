// Local VM validation of cleanup and real-diagnostic selection; no browser/network.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const original = readFileSync(new URL('./android-pair-page.js', import.meta.url), 'utf8');
const functions = original.slice(0, original.indexOf('\nmain().catch('))
  + '\nexport { run, stop, members, memberDiagnostics }; export function setNetwork(value) { network = value; }';
const context = vm.createContext({ URL, location: { href: 'http://127.0.0.1/?actor=owned' }, performance });
const sdk = new vm.SyntheticModule(['default', 'BrowserNetwork'], function () {
  this.setExport('default', async () => {}); this.setExport('BrowserNetwork', class {});
}, { context });
const module = new vm.SourceTextModule(functions, { context });
await module.link(() => sdk);
await module.evaluate();
const api = module.namespace;

const primary = api.memberDiagnostics({
  session_diagnostics: () => JSON.stringify({ newProtocol: true, expectedPeerMatched: false,
    memberAuthorized: true, paths: [{ route: 'actual-route', selected: true }],
    coordinator: { ordinaryReleased: true }, bootstrap: { sdp: 'private-description', binding: 'private-binding' } }),
  rtc_path_diagnostics: () => JSON.stringify({ expectedPeerMatched: true, activeAssociations: 0,
    activePumpTasks: 0, paths: [{ route: 'wrong-old-path' }], bootstrap: { exporter: 'private-exporter' } }),
});
assert.equal(primary.expectedPeerMatched, false);
assert.equal(primary.paths[0].route, 'actual-route');
assert.equal(primary.newProtocol, true);
assert.equal(primary.activeAssociations, 0);
assert.equal(primary.activePumpTasks, 0);
assert.equal(JSON.stringify(primary).includes('private-'), false);
const unobserved = api.memberDiagnostics({ session_diagnostics: () => '{"newProtocol":true,"paths":[]}' });
assert.equal(Object.hasOwn(unobserved, 'activePumpTasks'), false);
assert.equal(Object.hasOwn(unobserved, 'activeAssociations'), false);

const calls = [];
api.members.set('failed', {
  close() { calls.push('failed-close'); throw new Error('controlled member close'); },
  rtc_path_diagnostics() { calls.push('failed-diagnostics'); throw new Error('controlled getter'); },
  free() { calls.push('failed-free'); throw new Error('controlled member free'); },
});
api.members.set('healthy', {
  close() { calls.push('healthy-close'); },
  rtc_path_diagnostics() { calls.push('healthy-diagnostics'); return '{"activeAssociations":0}'; },
  free() { calls.push('healthy-free'); },
});
api.setNetwork({ close() { calls.push('network-close'); throw new Error('controlled network close'); },
  free() { calls.push('network-free'); } });
const cleanup = await api.stop();
assert.deepEqual(calls, ['failed-close', 'failed-diagnostics', 'failed-free', 'healthy-close',
  'healthy-diagnostics', 'healthy-free', 'network-close', 'network-free']);
assert.equal(cleanup.cleanupSucceeded, false);
assert.equal(cleanup.memberCloseCount, 1);
assert.equal(cleanup.memberFreeCount, 1);
assert.equal(cleanup.ownedMemberHandles, 1);
assert.equal(cleanup.networkClosed, false);
assert.equal(cleanup.networkFreed, true);
assert.equal(cleanup.errors.length, 4);
assert.equal(Object.hasOwn(cleanup.memberDiagnostics[0], 'activePumpTasks'), false);
assert.equal(cleanup.memberDiagnostics[0].activeAssociations, 0);
let releaseClose;
const concurrentCalls = [];
api.members.set('in-flight', {
  close() { concurrentCalls.push('close'); return new Promise(resolve => { releaseClose = resolve; }); },
  free() { concurrentCalls.push('free'); },
});
const firstStop = api.stop(), pageHideStop = api.stop();
assert.equal(firstStop, pageHideStop);
assert.deepEqual(concurrentCalls, ['close']);
releaseClose();
assert.equal((await firstStop).cleanupSucceeded, true);
await pageHideStop;
assert.deepEqual(concurrentCalls, ['close', 'free']);
console.log(JSON.stringify({ localOnly: true, sessionProofPreferred: true,
  missingRtcCountsOmitted: true, sensitiveDiagnosticFieldsRemoved: true,
  failuresDoNotStopRemainingCleanup: true, actualCloseFreeCountsPreserved: true,
  concurrentShutdownAndPagehideShareOneCleanup: true }));
