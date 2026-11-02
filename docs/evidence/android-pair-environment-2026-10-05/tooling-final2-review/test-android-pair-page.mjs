// Local VM validation of cleanup and real-diagnostic selection; no browser/network.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const original = readFileSync(new URL('./android-pair-page.js', import.meta.url), 'utf8');
const functions = original.slice(0, original.indexOf('\nmain().catch('))
  + '\nexport { run, stop, members, memberDiagnostics, claimOwnedStore, clearOwnedStore };'
  + '\nexport function setNetwork(value) { network = value; generation++; cleanupResult = undefined; }'
  + '\nexport function resetCleanup() { cleanupResult = undefined; cleanupHistory.length = 0; }'
  + '\nexport function resetStorage() { storeClaimed = false; storageCleanupRunning = undefined; }';
let existingDatabases = [], deleteOutcome = 'success', lockAvailable = true;
let nextSdkNetwork;
let lastDeleteRequest, lockHeld = false, lockReleaseCount = 0, storageTimeout;
const deletedNames = [], lockNames = [];
const context = vm.createContext({ URL, location: { href: 'http://127.0.0.1:32123/?actor=owned' }, performance,
  setTimeout(callback, delay) {
    if (delay === 1500) { storageTimeout = callback; return 'storage-timeout'; }
    return setTimeout(callback, delay);
  }, clearTimeout(value) { if (value !== 'storage-timeout') clearTimeout(value); },
  indexedDB: { async databases() { return existingDatabases; }, deleteDatabase(name) {
    deletedNames.push(name);
    const request = {}; lastDeleteRequest = request;
    queueMicrotask(() => request['on' + deleteOutcome]?.());
    return request;
  } }, navigator: { locks: { request(name, options, callback) {
    assert.equal(options.ifAvailable, true); lockNames.push(name);
    if (lockAvailable) lockHeld = true;
    return Promise.resolve(callback(lockAvailable ? {} : null)).finally(() => {
      if (lockAvailable) { lockHeld = false; lockReleaseCount++; }
    });
  } } } });
const sdk = new vm.SyntheticModule(['default', 'BrowserNetwork'], function () {
  this.setExport('default', async () => {}); this.setExport('BrowserNetwork', class {
    static async start() { return nextSdkNetwork; }
  });
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
// Regression 1: free succeeded after close failed; repeated stop/final shutdown
// must preserve that same generation's failures and forbid store deletion.
await api.claimOwnedStore();
const repeatedFailure = await api.stop();
assert.equal(repeatedFailure, cleanup);
const failedShutdown = await api.run({ op: 'shutdown' });
assert.equal(failedShutdown.cleanupSucceeded, false);
assert.equal(failedShutdown.networkClosed, false);
assert.equal(failedShutdown.networkFreed, true);
assert.equal(failedShutdown.errors.length, 4);
assert.equal(failedShutdown.cleanupHistory[0].generation, cleanup.generation);
assert.equal(failedShutdown.storage.outcome, 'connections-not-closed');
assert.deepEqual(deletedNames, []);
// A real next generation may start, but its success does not erase first failure.
nextSdkNetwork = { close() {}, free() {}, id() { return 'fixture-id'; }, status() { return '{}'; } };
await api.run({ op: 'start' });
const nextGeneration = await api.stop();
assert.equal(nextGeneration.cleanupSucceeded, true);
const afterRestart = await api.run({ op: 'shutdown' });
assert.equal(afterRestart.cleanupSucceeded, false);
assert.equal(afterRestart.cleanupHistory.length, 2);
assert.equal(afterRestart.errors.length, 4);
api.resetCleanup(); api.resetStorage();
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

// Existing origin stores are not claimed, modified or deleted.
existingDatabases = [{ name: 'miopunch-browser-network' }];
await assert.rejects(api.claimOwnedStore(), /preexisting SDK storage/);
assert.equal((await api.clearOwnedStore({ networkClosed: true, networkFreed: true })).required, false);
assert.deepEqual(deletedNames, []);
api.resetStorage(); existingDatabases = [];
await api.claimOwnedStore();
await api.stop();
assert.deepEqual(deletedNames, [], 'stop/reopen must retain the owned store');
const successfulShutdown = await api.run({ op: 'shutdown' });
assert.equal(successfulShutdown.cleanupSucceeded, true);
const storage = successfulShutdown.storage;
assert.equal(storage.deleted, true); assert.equal(storage.outcome, 'success');
assert.equal(storage.exclusiveStorageLock, true);
assert.deepEqual(deletedNames, ['miopunch-browser-network']);
assert.deepEqual(lockNames, ['miopunch-browser-network-v1']);
for (const outcome of ['blocked', 'error']) {
  api.resetStorage(); await api.claimOwnedStore(); deleteOutcome = outcome;
  const shutdown = await api.run({ op: 'shutdown' });
  assert.equal(shutdown.cleanupSucceeded, false);
  const observed = shutdown.storage;
  assert.equal(observed.deleted, false); assert.equal(observed.outcome, outcome);
  if (outcome === 'blocked') {
    // Regression 2: bounded blocked reply must not release the deletion lock.
    assert.equal(observed.pending, true); assert.equal(observed.terminal, false);
    const priorReleases = lockReleaseCount;
    assert.equal(lockHeld, true);
    lastDeleteRequest.onsuccess();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(lockHeld, false); assert.equal(lockReleaseCount, priorReleases + 1);
    assert.equal(shutdown.cleanupSucceeded, false, 'first blocked result remains a failure');
  } else { assert.equal(observed.pending, false); assert.equal(observed.terminal, true); }
}
api.resetStorage(); await api.claimOwnedStore(); deleteOutcome = 'none';
const timedShutdown = api.run({ op: 'shutdown' });
await new Promise(resolve => setImmediate(resolve));
assert.equal(lockHeld, true);
storageTimeout();
const timed = await timedShutdown;
assert.equal(timed.storage.outcome, 'timeout'); assert.equal(timed.storage.pending, true);
assert.equal(timed.storage.terminal, false); assert.equal(timed.cleanupSucceeded, false);
assert.equal(lockHeld, true);
const beforeTerminal = lockReleaseCount;
lastDeleteRequest.onerror();
await new Promise(resolve => setImmediate(resolve));
assert.equal(lockHeld, false); assert.equal(lockReleaseCount, beforeTerminal + 1);
api.resetStorage(); await api.claimOwnedStore(); lockAvailable = false;
const priorDeletes = deletedNames.length;
assert.equal((await api.clearOwnedStore({ cleanupSucceeded: true, networkClosed: true, networkFreed: true })).outcome, 'other-page-active');
assert.equal(deletedNames.length, priorDeletes);
api.resetStorage(); await api.claimOwnedStore();
assert.equal((await api.clearOwnedStore({ cleanupSucceeded: true, networkClosed: false, networkFreed: true })).outcome, 'connections-not-closed');
assert.equal(deletedNames.length, priorDeletes);
api.resetStorage(); delete context.indexedDB.databases;
await assert.rejects(api.claimOwnedStore(), /absence cannot be observed/);

// Typed getter and teardown sample the same actual SDK counter source.
let observedReads = 0;
api.setNetwork({ close() {}, free() {}, rtc_invalidation_diagnostics() {
  return JSON.stringify({ activeAssociations: observedReads++ ? 1 : 0, activePumpTasks: 0 });
} });
assert.equal((await api.run({ op: 'rtc_diagnostics' })).activeAssociations, 0);
observedReads = 0;
const lateCleanup = await api.stop();
assert.equal(lateCleanup.networkDiagnostics.length, 2);
assert.equal(lateCleanup.networkDiagnostics[0].delayMs, 0);
assert.equal(lateCleanup.networkDiagnostics[1].delayMs, 100);
assert.equal(lateCleanup.networkDiagnostics[1].observed.activeAssociations, 1);
assert.equal(lateCleanup.cleanupSucceeded, false);
console.log(JSON.stringify({ localOnly: true, sessionProofPreferred: true,
  missingRtcCountsOmitted: true, sensitiveDiagnosticFieldsRemoved: true,
  failuresDoNotStopRemainingCleanup: true, actualCloseFreeCountsPreserved: true,
  concurrentShutdownAndPagehideShareOneCleanup: true, preexistingStoreUntouched: true,
  stopPreservesOwnedStore: true, shutdownDeletesOnlyClaimedStoreWithExclusiveLock: true,
  blockedErrorAndContendedLockFailCleanup: true, actualImmediateAndLateRtcDiagnostics: true,
  sameGenerationFailureAndRestartHistoryRetained: true, blockedAndTimeoutHoldLockUntilActualTerminal: true }));
