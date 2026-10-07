// M77 owned page: real SDK continuity stream and an owned RTC observer.
// No maintenance protocol or replacement transport is implemented here.
// Requests/results may contain private fixture material: never console-log them.
import init, { BrowserNetwork } from '/pkg/miopunch_browser_probe.js';

const actor = new URL(location.href).searchParams.get('actor');
const environmentRtcProbe = new URL(location.href).searchParams.get('environmentRtcProbe') !== 'false';
const members = new Map();
let network;
let stopping = false;
let cleanupRunning;
let cleanupResult;
let generation = 0;
const cleanupHistory = [];
const ownedDatabaseName = 'miopunch-browser-network';
const ownedStorageLock = 'miopunch-browser-network-v1';
let storeClaimed = false;
let storageCleanupRunning;
const websocketCloseRecords = [];
const previousWebSocket = globalThis.WebSocket;
if (typeof previousWebSocket === 'function') {
  function CloseObservedWebSocket(...args) {
    const socket = new previousWebSocket(...args);
    const record = { socket, closeCalls: 0, closeFailures: 0, firstCloseAtMs: null,
      stateBeforeFirstClose: null, stateAfterFirstClose: null, listeners: [], trackingComplete: true };
    websocketCloseRecords.push(record);
    const close = socket.close.bind(socket);
    socket.close = (...values) => {
      record.closeCalls++;
      if (record.firstCloseAtMs === null) {
        record.firstCloseAtMs = performance.now();
        record.stateBeforeFirstClose = socket.readyState;
      }
      try {
        const result = close(...values);
        if (record.stateAfterFirstClose === null) record.stateAfterFirstClose = socket.readyState;
        return result;
      } catch (error) { record.closeFailures++; throw error; }
    };
    const add = socket.addEventListener.bind(socket);
    const remove = socket.removeEventListener.bind(socket);
    const capture = options => typeof options === 'boolean' ? options : !!options?.capture;
    socket.addEventListener = (type, callback, options) => {
      const result = add(type, callback, options);
      if (options?.once || options?.signal) record.trackingComplete = false;
      if (callback && !record.listeners.some(item => item.type === type
        && item.callback === callback && item.capture === capture(options)))
        record.listeners.push({ type, callback, capture: capture(options) });
      return result;
    };
    socket.removeEventListener = (type, callback, options) => {
      const result = remove(type, callback, options);
      record.listeners = record.listeners.filter(item => !(item.type === type
        && item.callback === callback && item.capture === capture(options)));
      return result;
    };
    return socket;
  }
  CloseObservedWebSocket.prototype = previousWebSocket.prototype;
  Object.setPrototypeOf(CloseObservedWebSocket, previousWebSocket);
  globalThis.WebSocket = CloseObservedWebSocket;
}

function parsed(value) {
  return typeof value === 'string' && /^[\[{]/.test(value) ? JSON.parse(value) : value;
}

async function post(value) {
  const response = await fetch(`/result?actor=${encodeURIComponent(actor)}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value),
  });
  if (!response.ok) throw new Error('local control result was not accepted');
}

function rtcCapability() {
  const result = { secureContext: isSecureContext, peerConnectionApi: typeof RTCPeerConnection === 'function',
    indexedDb: typeof indexedDB !== 'undefined', webLocks: !!navigator.locks, peerCreated: false, channelCreated: false };
  let peer, channel;
  try {
    peer = new RTCPeerConnection({ iceServers: [] });
    result.peerCreated = true;
    channel = peer.createDataChannel('mio-quic-datagram/1', {
      negotiated: true, id: 0, ordered: false, maxRetransmits: 0,
      protocol: 'miopunch/quic-datagram/1',
    });
    result.channelCreated = true;
    result.channelParametersMatched = channel.id === 0 && !channel.ordered
      && channel.maxRetransmits === 0 && channel.negotiated;
  } catch (error) {
    result.failureClass = error.name;
  } finally {
    channel?.close(); peer?.close();
    result.peerClosed = !peer || peer.connectionState === 'closed';
  }
  return result;
}

function member(name = 'member') {
  const value = members.get(name);
  if (!value) throw new Error('owned member handle does not exist');
  return value;
}

function feature(value, name) {
  if (typeof value?.[name] !== 'function') throw new Error(`required SDK method unavailable: ${name}`);
  return value[name].bind(value);
}

function rtcFixture() {
  // The capability prelude is shared with MIO76, but this page owns the
  // observer lifetime and never treats its counters as a path qualification.
  return globalThis.__mio76RtcFixture ?? globalThis.__mio77RtcFixture;
}

function rtcFixtureSnapshot() {
  const snapshot = feature(rtcFixture(), 'snapshot')();
  if (websocketCloseRecords.length) {
    snapshot.webSocketCloseWitnesses = websocketCloseRecords.map((record, socketIndex) => {
      const callbacks = ['onopen', 'onmessage', 'onerror', 'onclose'].map(name => {
        const value = record.socket[name];
        return typeof value === 'function' ? 1 : value === null ? 0 : null;
      });
      return { socketIndex, readyState: record.socket.readyState, closeCalls: record.closeCalls,
        closeFailures: record.closeFailures, firstCloseAtMs: record.firstCloseAtMs,
        stateBeforeFirstClose: record.stateBeforeFirstClose,
        stateAfterFirstClose: record.stateAfterFirstClose,
        bufferedBytes: record.socket.bufferedAmount,
        callbackTrackingComplete: record.trackingComplete && !callbacks.includes(null),
        sdkPropertyCallbacks: callbacks.includes(null) ? null : callbacks.reduce((a, b) => a + b, 0),
        sdkRegisteredListeners: record.listeners.length };
    });
  }
  return snapshot;
}

function candidateObservation(name) {
  const current = memberDiagnostics(member(name));
  const networkObserver = network && typeof network.rtc_invalidation_diagnostics === 'function'
    ? diagnostic(parsed(network.rtc_invalidation_diagnostics())) : null;
  return { member: current, rtcObjects: rtcFixtureSnapshot(), network: networkObserver };
}

function diagnostic(value) {
  const hidden = new Set(['sdp', 'description', 'exporter', 'binding', 'bindinghash', 'protectedbinding',
    'privatekey', 'nodekey', 'secret', 'password', 'credential', 'invitation', 'code', 'usernamefragment',
    'icepwd', 'icepassword', 'ufrag', 'peerid', 'localid', 'remoteid', 'nodeid', 'expectedpeerid', 'publickey']);
  if (Array.isArray(value)) return value.map(diagnostic);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !hidden.has(key.replaceAll('_', '').toLowerCase()))
    .map(([key, child]) => [key, diagnostic(child)]));
  return value;
}

function memberDiagnostics(value) {
  const current = typeof value.session_diagnostics === 'function'
    ? parsed(value.session_diagnostics()) : parsed(feature(value, 'rtc_path_diagnostics')());
  if (typeof value.rtc_path_diagnostics === 'function') {
    const rtc = parsed(value.rtc_path_diagnostics());
    for (const name of ['activeAssociations', 'activePumpTasks']) {
      if (Object.hasOwn(rtc, name)) current[name] = rtc[name];
    }
  }
  return diagnostic(current);
}

function stop() {
  if (cleanupRunning) return cleanupRunning;
  if (cleanupResult) return Promise.resolve(cleanupResult);
  const pending = cleanupOwned().then(result => {
    cleanupResult = { ...result, generation };
    cleanupHistory.push(cleanupResult);
    return cleanupResult;
  });
  cleanupRunning = pending;
  pending.finally(() => { if (cleanupRunning === pending) cleanupRunning = undefined; }).catch(() => {});
  return pending;
}

async function claimOwnedStore() {
  if (storeClaimed) return;
  const address = new URL(location.href);
  if (address.protocol !== 'http:' || address.hostname !== '127.0.0.1' || !address.port || !actor)
    throw new Error('owned local origin is required for temporary SDK storage');
  if (typeof globalThis.indexedDB?.databases !== 'function')
    throw new Error('owned SDK database absence cannot be observed');
  const databases = await indexedDB.databases();
  if (databases.some(value => value.name === ownedDatabaseName))
    throw new Error('preexisting SDK storage belongs outside this run');
  // Claim before start so a partially failed SDK start can still be cleaned.
  storeClaimed = true;
}

function clearOwnedStore(cleanup) {
  if (storageCleanupRunning) return storageCleanupRunning;
  storageCleanupRunning = (async () => {
    if (!storeClaimed) return { required: false, ownedDatabaseClaimed: false };
    if (!cleanup.cleanupSucceeded || !cleanup.networkClosed || !cleanup.networkFreed)
      return { required: true, ownedDatabaseClaimed: true, deleted: false, outcome: 'connections-not-closed' };
    if (typeof navigator.locks?.request !== 'function')
      return { required: true, ownedDatabaseClaimed: true, deleted: false, outcome: 'lock-api-missing' };
    // The bounded controller reply and the Web Lock's lifetime are separate.
    // blocked/timeout is still a pending IDB request, so it must retain the lock.
    return new Promise(resolveReport => {
      const locked = navigator.locks.request(ownedStorageLock, { ifAvailable: true }, async lock => {
        if (!lock) {
          resolveReport({ required: true, ownedDatabaseClaimed: true, deleted: false,
            pending: false, terminal: true, outcome: 'other-page-active' });
          return;
        }
        await new Promise(resolveTerminal => {
          let terminal = false;
          const report = (outcome, deleted, pending) => resolveReport({ required: true,
            ownedDatabaseClaimed: true, exclusiveStorageLock: true, outcome, deleted,
            pending, terminal: !pending });
          const finish = (outcome, deleted) => {
            if (terminal) return;
            terminal = true; clearTimeout(timer);
            report(outcome, deleted, false);
            resolveTerminal();
          };
          const timer = setTimeout(() => report('timeout', false, true), 1500);
          try {
            const request = indexedDB.deleteDatabase(ownedDatabaseName);
            request.onsuccess = () => finish('success', true);
            request.onerror = () => finish('error', false);
            request.onblocked = () => report('blocked', false, true);
          } catch (_) { finish('exception', false); }
        });
      });
      locked.catch(error => resolveReport({ required: true, ownedDatabaseClaimed: true,
        deleted: false, pending: false, terminal: true, outcome: 'lock-exception',
        failureClass: error?.name ?? typeof error }));
    });
  })().catch(error => ({ required: true, ownedDatabaseClaimed: storeClaimed, deleted: false,
    outcome: 'exception', failureClass: error?.name ?? typeof error }));
  return storageCleanupRunning;
}

async function shutdown() {
  stopping = true;
  const current = await stop();
  const cleanup = { ...current, cleanupHistory: [...cleanupHistory],
    networkClosed: cleanupHistory.every(value => value.networkClosed),
    networkFreed: cleanupHistory.every(value => value.networkFreed),
    errors: cleanupHistory.flatMap(value => value.errors.map(error => ({ ...error, generation: value.generation }))),
    cleanupSucceeded: cleanupHistory.every(value => value.cleanupSucceeded) };
  // These are actual final object samples, separate from setup or maintenance clocks.
  cleanup.rtcObjectsImmediate = rtcFixtureSnapshot();
  await new Promise(resolve => setTimeout(resolve, 100));
  cleanup.rtcObjectsLate = rtcFixtureSnapshot();
  cleanup.storage = await clearOwnedStore(cleanup);
  cleanup.cleanupSucceeded &&= !cleanup.storage.required || (cleanup.storage.deleted === true
    && cleanup.storage.terminal === true && cleanup.storage.pending === false);
  return cleanup;
}

async function cleanupOwned() {
  const started = performance.now();
  const diagnostics = [], errors = [];
  let memberCloseCount = 0, memberFreeCount = 0;
  const ownedMemberCount = members.size;
  const failed = (resource, operation, error) => errors.push({ resource, operation,
    failureClass: error?.name ?? typeof error });
  for (const value of members.values()) {
    try { await value.close(); memberCloseCount++; }
    catch (error) { failed('member', 'close', error); }
    try {
      if (typeof value.rtc_path_diagnostics === 'function') {
        const rtc = parsed(value.rtc_path_diagnostics());
        const observed = { diagnosticsAvailable: true };
        for (const name of ['activeAssociations', 'activePumpTasks']) {
          if (Object.hasOwn(rtc, name)) observed[name] = rtc[name];
        }
        diagnostics.push(observed);
      } else diagnostics.push({ diagnosticsAvailable: false });
    } catch (error) { failed('member', 'diagnostics', error); }
    try { await value.free(); memberFreeCount++; }
    catch (error) { failed('member', 'free', error); }
  }
  members.clear();
  let networkClosed = !network, networkFreed = !network;
  const networkDiagnostics = [];
  if (network) {
    try { await network.close(); networkClosed = true; }
    catch (error) { failed('network', 'close', error); }
    if (typeof network.rtc_invalidation_diagnostics === 'function') {
      for (const delayMs of [0, 100]) {
        if (delayMs) await new Promise(resolve => setTimeout(resolve, delayMs));
        try {
          const observed = diagnostic(parsed(network.rtc_invalidation_diagnostics()));
          networkDiagnostics.push({ delayMs, observed });
          for (const name of ['activeAssociations', 'activePumpTasks']) {
            if (Object.hasOwn(observed, name) && observed[name] !== 0)
              failed('network', name, new Error('owned RTC resource remains after close'));
          }
        } catch (error) { failed('network', 'rtc diagnostics', error); }
      }
    }
    try { await network.free(); networkFreed = true; }
    catch (error) { failed('network', 'free', error); }
    if (networkFreed) network = undefined;
  }
  return { ownedMemberCount, memberCloseCount, memberFreeCount,
    ownedMemberHandles: ownedMemberCount - memberFreeCount, networkClosed, networkFreed,
    memberDiagnostics: diagnostics, networkDiagnostics, errors, cleanupSucceeded: errors.length === 0,
    cleanupElapsedMs: performance.now() - started };
}

async function run(request) {
  const args = request.args ?? {};
  switch (request.op) {
    case 'capability': return rtcCapability();
    case 'rtc_fixture_snapshot': return rtcFixtureSnapshot();
    case 'rtc_fixture_reset': return feature(rtcFixture(), 'resetCounts')();
    case 'rtc_fixture_restore': return feature(rtcFixture(), 'restore')();
    case 'start':
      if (network) throw new Error('owned network is already running');
      await claimOwnedStore();
      network = await BrowserNetwork.start(JSON.stringify(args.configuration ?? {}));
      generation++; cleanupResult = undefined;
      return { id: network.id(), status: parsed(network.status()) };
    case 'status': return parsed(feature(network, 'status')());
    case 'request_admission': return parsed(await feature(network, 'request_admission')(args.invitation));
    case 'request_precode': return parsed(await feature(network, 'request_precode')(args.code));
    case 'admission_identity_probe': return parsed(await feature(network, 'admission_identity_probe')(args.request, args.claimed));
    case 'complete_admission': return parsed(await feature(network, 'complete_admission')());
    case 'material_receipt': return parsed(await feature(network, 'admission_material_receipt')());
    case 'open_member': {
      const name = args.name ?? 'member';
      if (members.has(name)) throw new Error('owned member handle name is already in use');
      members.set(name, await feature(network, 'open_member_connection')(args.target));
      return { opened: true, name };
    }
    case 'exchange': return await member(args.name).exchange(args.message);
    case 'continuity_start': return parsed(await feature(member(args.name), 'rtc_continuity_start')());
    case 'continuity_exchange': {
      if (!Number.isSafeInteger(args.sequence) || args.sequence < 1)
        throw new Error('continuity sequence must be a positive safe integer');
      return parsed(await feature(member(args.name), 'rtc_continuity_exchange')(BigInt(args.sequence)));
    }
    case 'improve_path': {
      if (!Number.isSafeInteger(args.callerMs) || args.callerMs < 1 || args.callerMs > 60000)
        throw new Error('active improvement caller budget must be a bounded integer');
      return parsed(await feature(member(args.name), 'improve_path')(args.callerMs));
    }
    case 'candidate_observation': {
      const immediate = candidateObservation(args.name);
      await new Promise(resolve => setTimeout(resolve, 100));
      return { immediate, late: candidateObservation(args.name), lateDelayMs: 100 };
    }
    case 'member_diagnostics': return memberDiagnostics(member(args.name));
    case 'path_diagnostics': return parsed(feature(network, 'path_diagnostics')());
    case 'rtc_diagnostics': return diagnostic(parsed(feature(network, 'rtc_invalidation_diagnostics')()));
    case 'sync_replica': return parsed(await feature(network, 'sync_replica')(args.target));
    case 'replica_diagnostics': return parsed(feature(network, 'replica_diagnostics')());
    case 'close_member': {
      const name = args.name ?? 'member';
      member(name).close(); member(name).free(); members.delete(name);
      return { closed: true, name };
    }
    case 'stop': return await stop();
    case 'shutdown': return await shutdown();
    default: throw new Error('unsupported local device-control operation');
  }
}

async function main() {
  if (!actor) throw new Error('owned page actor is required');
  await init();
  await post({ event: 'ready', wasmLoaded: true,
    rtc: environmentRtcProbe ? rtcCapability() : { environmentProbeSkipped: true },
    sdk: { start: typeof BrowserNetwork.start === 'function',
      memberConnection: typeof BrowserNetwork.prototype.open_member_connection === 'function' } });
  while (!stopping) {
    const response = await fetch(`/control?actor=${encodeURIComponent(actor)}`, { cache: 'no-store' });
    if (!response.ok) throw new Error('local device controller is unavailable');
    const request = await response.json();
    if (!request) continue;
    const started = performance.now();
    try {
      const result = await run(request);
      await post({ id: request.id, ok: true, result, callerElapsedMs: performance.now() - started });
    } catch (error) {
      await post({ id: request.id, ok: false, failureClass: error.name,
        failure: String(error), callerElapsedMs: performance.now() - started });
    }
  }
}

main().catch(async error => {
  const cleanup = await stop().catch(() => ({ cleanupFailed: true }));
  await post({ event: 'terminal', failureClass: error.name, failure: String(error), cleanup }).catch(() => {});
});
addEventListener('pagehide', () => { stop().catch(() => {}); }, { once: true });
