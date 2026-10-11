// Owned Host/Pixel Browser pair controller. Network traffic still uses the formal WASM SDK.
// Requests/results may contain private fixture material: never console-log them.
import init, { BrowserNetwork } from '/pkg/miopunch_browser_probe.js';

const actor = new URL(location.href).searchParams.get('actor');
const environmentRtcProbe = new URL(location.href).searchParams.get('environmentRtcProbe') !== 'false';
const members = new Map();
const expectedPeers = new Map();
const memberStableIds = new Map();
const memberRtcPeers = new Map();
let generationPeerStart = 0;
const generationRtcStableIds = new Set();
const attempts = new Map();
const exchanges = new Map();
const retirementErrors = [];
let lateHandleRetirements = 0;
const realPeers = [], realChannels = [];
const peerChannels = new Map();
const channelCounters = new Map();
const peerLifecycle = new Map();
const descriptionTypes = new Set(['offer', 'answer', 'pranswer', 'rollback']);
function descriptionFacts(description) {
  if (!description) return { present: false, type: null, candidates: null };
  const candidates = { total: 0, host: 0, srflx: 0, prflx: 0, relay: 0, other: 0, udp: 0, tcp: 0 };
  if (typeof description.sdp !== 'string') return { present: true,
    type: descriptionTypes.has(description.type) ? description.type : 'other', candidates: null };
  for (const line of description.sdp.split(/\r?\n/)) {
    if (!line.startsWith('a=candidate:')) continue;
    const fields = line.trim().split(/\s+/);
    candidates.total++;
    const kind = fields[6] === 'typ' && ['host', 'srflx', 'prflx', 'relay'].includes(fields[7]) ? fields[7] : 'other';
    candidates[kind]++;
    if (fields[2]?.toLowerCase() === 'udp') candidates.udp++;
    if (fields[2]?.toLowerCase() === 'tcp') candidates.tcp++;
  }
  return { present: true, type: descriptionTypes.has(description.type) ? description.type : 'other', candidates };
}
function stateFacts(peer) {
  const state = (value, allowed) => allowed.includes(value) ? value : 'unknown';
  return { pcState: state(peer.connectionState, ['new', 'connecting', 'connected', 'disconnected', 'failed', 'closed']),
    iceState: state(peer.iceConnectionState, ['new', 'checking', 'connected', 'completed', 'disconnected', 'failed', 'closed']),
    gatheringState: state(peer.iceGatheringState, ['new', 'gathering', 'complete']),
    signalingState: state(peer.signalingState, ['stable', 'have-local-offer', 'have-remote-offer',
      'have-local-pranswer', 'have-remote-pranswer', 'closed']),
    local: descriptionFacts(peer.localDescription), remote: descriptionFacts(peer.remoteDescription) };
}
function lifecycleEvent(peer, kind, extra = {}) {
  const life = peerLifecycle.get(peer);
  if (!life) return;
  const event = { elapsedMs: performance.now() - life.createdAt, kind, ...extra, ...stateFacts(peer) };
  if (life.events.length === 32) life.events.shift();
  life.events.push(event);
}
function observeLifecycle(peer) {
  const life = { createdAt: performance.now(), events: [], beforeClose: null, listenersDetached: false,
    candidateObservations: { types: { host: 0, srflx: 0, prflx: 0, relay: 0, other: 0 },
      endOfCandidates: 0, errorCount: 0, errorCodes: [], otherErrorCodes: 0 } };
  peerLifecycle.set(peer, life);
  const listeners = ['signalingstatechange', 'icegatheringstatechange', 'iceconnectionstatechange', 'connectionstatechange']
    .map(kind => { const listener = () => lifecycleEvent(peer, kind); peer.addEventListener?.(kind, listener); return [kind, listener]; });
  const candidate = event => {
    const kind = event.candidate?.type;
    if (event.candidate === null) life.candidateObservations.endOfCandidates++;
    else life.candidateObservations.types[['host', 'srflx', 'prflx', 'relay'].includes(kind) ? kind : 'other']++;
    lifecycleEvent(peer, event.candidate === null ? 'icecandidate-end' : 'icecandidate', {
      candidateType: ['host', 'srflx', 'prflx', 'relay'].includes(kind) ? kind : null,
    });
  };
  const candidateError = event => {
    const errorCode = Number.isInteger(event.errorCode) && event.errorCode >= 0 && event.errorCode <= 65535
      ? event.errorCode : null;
    const facts = life.candidateObservations;
    facts.errorCount++;
    const recorded = facts.errorCodes.find(entry => entry.code === errorCode);
    if (recorded) recorded.count++;
    else if (facts.errorCodes.length < 8) facts.errorCodes.push({ code: errorCode, count: 1 });
    else facts.otherErrorCodes++;
    lifecycleEvent(peer, 'icecandidateerror', { errorCode });
  };
  for (const [kind, listener] of [['icecandidate', candidate], ['icecandidateerror', candidateError]]) {
    peer.addEventListener?.(kind, listener);
    listeners.push([kind, listener]);
  }
  life.detach = () => {
    for (const [kind, listener] of listeners) peer.removeEventListener?.(kind, listener);
    life.listenersDetached = true;
  };
  lifecycleEvent(peer, 'created');
}
function lifecycleFacts(peer, attempt) {
  const life = peerLifecycle.get(peer);
  const regression = peer.__mio75EarlyReceiveRegression;
  const earlyReceiveRegression = regression ? Object.fromEntries([
    'armed', 'realStatsRequested', 'statsPendingAtFirstMessage', 'realMessageReceived',
    'statsReleased', 'channelStayedOpenAfterReceive', 'listenersDetached',
  ].map(name => [name, regression[name] === true])) : null;
  return { relativeToAttemptMs: attempt === undefined ? null : life.createdAt - attempt,
    beforeClose: life.beforeClose, listenersDetached: life.listenersDetached,
    candidateObservations: structuredClone(life.candidateObservations),
    earlyReceiveRegression,
    events: [...life.events], channels: peerChannels.get(peer).map(channel => {
      const counts = channelCounters.get(channel);
      return { everOpened: counts.everOpened, beforeCloseState: counts.beforeCloseState,
        sentPackets: counts.sentPackets, receivedPackets: counts.receivedPackets,
        sentBytes: counts.sentBytes, receivedBytes: counts.receivedBytes };
    }) };
}
function packetBytes(value) {
  if (typeof value === 'string') return new TextEncoder().encode(value).byteLength;
  if (typeof value?.byteLength === 'number') return value.byteLength;
  if (typeof value?.size === 'number') return value.size;
  throw new Error('real SDK datagram byte length is unavailable');
}
function observeChannel(peer, channel) {
  const counts = { sentPackets: 0, sentBytes: 0, receivedPackets: 0, receivedBytes: 0,
    observerDetached: false, everOpened: channel.readyState === 'open', beforeCloseState: null };
  const opened = () => { counts.everOpened = true; lifecycleEvent(peer, 'channel-open'); };
  channel.addEventListener('open', opened);
  const send = channel.send.bind(channel);
  channel.send = value => {
    const result = send(value); // Count only a successful real SDK send.
    counts.sentPackets++; counts.sentBytes += packetBytes(value);
    return result;
  };
  const receive = event => { counts.receivedPackets++; counts.receivedBytes += packetBytes(event.data); };
  channel.addEventListener('message', receive);
  const detach = () => {
    if (counts.observerDetached) return;
    channel.removeEventListener('message', receive);
    channel.removeEventListener('open', opened);
    channel.removeEventListener('close', detach);
    counts.observerDetached = true;
  };
  channel.addEventListener('close', detach);
  const close = channel.close.bind(channel);
  channel.close = () => {
    if (counts.beforeCloseState === null) {
      counts.beforeCloseState = ['connecting', 'open', 'closing', 'closed'].includes(channel.readyState) ? channel.readyState : 'unknown';
      lifecycleEvent(peer, 'channel-before-close', { channelState: counts.beforeCloseState });
    }
    detach(); return close();
  };
  counts.detach = detach; channelCounters.set(channel, counts);
}
let answerHold;
const OriginalPeer = globalThis.RTCPeerConnection;
if (typeof OriginalPeer === 'function') {
  function ObservedPeer(...args) {
    const peer = new OriginalPeer(...args); realPeers.push(peer); peerChannels.set(peer, []); observeLifecycle(peer);
    const create = peer.createDataChannel.bind(peer);
    peer.createDataChannel = (...values) => {
      const channel = create(...values); realChannels.push(channel); peerChannels.get(peer).push(channel); observeChannel(peer, channel); return channel;
    };
    const closePeer = peer.close.bind(peer);
    peer.close = () => {
      const life = peerLifecycle.get(peer);
      if (life.beforeClose === null) life.beforeClose = { elapsedMs: performance.now() - life.createdAt, ...stateFacts(peer) };
      life.detach();
      for (const channel of peerChannels.get(peer)) channelCounters.get(channel).detach();
      return closePeer();
    };
    const answer = peer.createAnswer.bind(peer);
    peer.createAnswer = async (...values) => {
      const hold = answerHold;
      if (!hold || hold.claimed) return answer(...values);
      hold.claimed = true;
      const result = await answer(...values); // Always the real browser answer.
      hold.answerCreated = true;
      await hold.work;
      hold.released = true;
      return result;
    };
    for (const method of ['createOffer', 'createAnswer', 'setLocalDescription', 'setRemoteDescription']) {
      if (typeof peer[method] !== 'function') continue;
      const call = peer[method].bind(peer);
      peer[method] = (...values) => {
        lifecycleEvent(peer, method + '-start', method.startsWith('set') ? { argument: descriptionFacts(values[0]) } : {});
        const failed = error => {
          const allowed = ['Error', 'TypeError', 'RangeError', 'OperationError', 'InvalidStateError', 'InvalidAccessError',
            'NotSupportedError', 'NotAllowedError', 'SecurityError', 'NetworkError'];
          lifecycleEvent(peer, method + '-failed', { failureClass: allowed.includes(error?.name) ? error.name : 'OtherError' });
          throw error;
        };
        try {
          return Promise.resolve(call(...values)).then(result => {
            lifecycleEvent(peer, method + '-done', method.startsWith('create') ? { result: descriptionFacts(result) } : {});
            return result;
          }, failed);
        } catch (error) { return failed(error); }
      };
    }
    return peer;
  }
  ObservedPeer.prototype = OriginalPeer.prototype;
  Object.setPrototypeOf(ObservedPeer, OriginalPeer);
  Object.defineProperty(globalThis, 'RTCPeerConnection', {
    configurable: true, writable: true, value: ObservedPeer,
  });
}
function rtcObjects(after = 0, attempt) {
  const peers = realPeers.slice(after);
  return { pcCreated: realPeers.length, openPeers: peers.filter(p => p.connectionState !== 'closed').length,
    dcCreated: realChannels.length, openChannels: peers.flatMap(p => peerChannels.get(p)).filter(c => c.readyState !== 'closed').length,
    closingChannels: peers.flatMap(p => peerChannels.get(p)).filter(c => c.readyState === 'closing').length,
    peers: peers.map(p => ({ closed: p.connectionState === 'closed',
      lifecycle: lifecycleFacts(p, attempt),
      ownObserversDetached: peerChannels.get(p).every(c => channelCounters.get(c).observerDetached),
      dataChannelCallbacksDetached: peerChannels.get(p).every(c => ['onopen', 'onmessage', 'onclose', 'onerror',
        'onbufferedamountlow'].every(name => c[name] === null)),
      callbacksDetached: p.onicegatheringstatechange === null && p.oniceconnectionstatechange === null
        && p.onconnectionstatechange === null })),
    answerHold: answerHold ? { claimed: answerHold.claimed, answerCreated: answerHold.answerCreated,
      released: answerHold.released } : null };
}
function releaseAnswer() { answerHold?.resolve(); }
async function retire(value, late = false) {
  const failures = [];
  try { await value.close(); } catch (error) { failures.push({ operation: 'close', failureClass: error.name }); }
  try { await value.free(); } catch (error) { failures.push({ operation: 'free', failureClass: error.name }); }
  retirementErrors.push(...failures);
  if (late && failures.length === 0) lateHandleRetirements++;
  return failures;
}
function attemptFacts(entry) {
  return { name: entry.name, role: entry.role, settled: entry.settled, cancelled: entry.cancelled,
    returned: entry.returned === true, expectedPeerMatched: entry.expectedPeerMatched, sdkFailure: entry.failure,
    callerBudgetMs: entry.callerMs, callerElapsedMs: entry.elapsedMs, failureClass: entry.failureClass,
    rtc: rtcObjects(entry.peerStart, entry.startedAt),
    lateHandleRetirements, retirementErrors: [...retirementErrors] };
}
function beginAttempt(args, role) {
  const name = args.name;
  if (typeof name !== 'string' || !name || attempts.has(name) || members.has(name))
    throw new Error('owned attempt name is required and cannot be reused');
  if (typeof args.expectedPeer !== 'string' || !args.expectedPeer
    || !Number.isInteger(args.callerMs) || ![12000, 60000].includes(args.callerMs))
    throw new Error('independent expected peer and explicit 12000ms or 60000ms caller are required');
  const started = performance.now(), epoch = generation;
  const handle = role === 'connect'
    ? feature(network, 'member_connection_attempt')(args.expectedPeer, args.callerMs)
    : feature(network, 'member_accept_attempt')(args.callerMs);
  const entry = { name, role, handle, callerMs: args.callerMs, settled: false, cancelled: false, peerStart: realPeers.length, startedAt: started };
  attempts.set(name, entry);
  entry.work = (async () => {
    let value;
    try {
      value = await feature(handle, role === 'connect' ? 'connect' : 'accept')();
      if (entry.cancelled || epoch !== generation || stopping) {
        await retire(value, true); value = undefined; return;
      }
      const current = parsed(feature(value, 'session_diagnostics')());
      entry.expectedPeerMatched = current.remoteId === args.expectedPeer;
      if (!entry.expectedPeerMatched) throw new Error('independently expected TLS peer differs');
      const owned = realPeers.slice(entry.peerStart);
      if (owned.length !== 1 || peerChannels.get(owned[0]).length !== 1)
        throw new Error('returned RTC M must own exactly one observed SDK PC/DC');
      const stable = current.bootstrap?.memberStableId;
      if (!Number.isSafeInteger(stable)) throw new Error('returned RTC M stable identity is unavailable');
      members.set(name, value); expectedPeers.set(name, args.expectedPeer);
      memberStableIds.set(name, stable); memberRtcPeers.set(name, owned[0]);
      generationRtcStableIds.add(stable);
      value = undefined; entry.returned = true;
    } catch (error) {
      entry.failureClass = error?.name ?? typeof error;
      // Preserve the first SDK error privately in controller memory only.
      entry.failure = String(error);
      if (value) await retire(value, true);
    } finally {
      try { handle.free(); } catch (error) { retirementErrors.push({ operation: 'attempt free', failureClass: error.name }); }
      entry.handle = undefined; entry.elapsedMs = performance.now() - started; entry.settled = true;
    }
  })();
  return attemptFacts(entry);
}
function pending(name) {
  const value = attempts.get(name);
  if (!value) throw new Error('owned attempt does not exist');
  return value;
}
async function cancelAttempt(entry) {
  entry.cancelled = true;
  if (entry.handle) feature(entry.handle, 'cancel')();
  if (members.has(entry.name)) {
    await retire(members.get(entry.name)); members.delete(entry.name); expectedPeers.delete(entry.name); memberStableIds.delete(entry.name); memberRtcPeers.delete(entry.name);
  }
  return attemptFacts(entry);
}
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

function diagnostic(value) {
  const hidden = new Set(['sdp', 'description', 'exporter', 'binding', 'bindinghash', 'protectedbinding',
    'privatekey', 'nodekey', 'secret', 'password', 'credential', 'invitation', 'code', 'usernamefragment',
    'icepwd', 'icepassword', 'ufrag', 'stableid', 'memberstableid', 'controlstableid', 'stats', 'route', 'localroute', 'remoteroute', 'address', 'peerid', 'localid', 'remoteid', 'nodeid', 'expectedpeerid', 'publickey']);
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
    current.rtcPaths = rtc.paths; current.rtcHandshakeLease = rtc.bootstrap?.route;
    current.sameDiagnosticsM = rtc.memberStableId === current.bootstrap?.memberStableId;
    for (const name of ['activeAssociations', 'activePumpTasks']) {
      if (Object.hasOwn(rtc, name)) current[name] = rtc[name];
    }
  }
  const name = [...members.entries()].find(([, handle]) => handle === value)?.[0];
  current.independentExpectedPeerMatched = current.remoteId === expectedPeers.get(name);
  const peer = memberRtcPeers.get(name), channels = peer ? peerChannels.get(peer) : [];
  const channel = channels.length === 1 ? channels[0] : undefined;
  const counters = channel ? channelCounters.get(channel) : undefined;
  current.liveOwnedRtc = peer && channel && counters ? {
    sameOwnedPeer: true, pcState: peer.connectionState, channelState: channel.readyState,
    localDescriptionType: peer.localDescription?.type, remoteDescriptionType: peer.remoteDescription?.type,
    channelId: channel.id, negotiated: channel.negotiated, ordered: channel.ordered,
    maxRetransmits: channel.maxRetransmits,
    sentPackets: counters.sentPackets, sentBytes: counters.sentBytes,
    receivedPackets: counters.receivedPackets, receivedBytes: counters.receivedBytes,
    observerDetached: counters.observerDetached,
  } : { sameOwnedPeer: false };
  current.originalMUnchanged = current.bootstrap?.memberStableId !== undefined
    && current.bootstrap.memberStableId === memberStableIds.get(name);
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
  cleanup.storage = await clearOwnedStore(cleanup);
  cleanup.cleanupSucceeded &&= !cleanup.storage.required || (cleanup.storage.deleted === true
    && cleanup.storage.terminal === true && cleanup.storage.pending === false);
  return cleanup;
}

async function cleanupOwned() {
  const started = performance.now();
  const diagnostics = [], errors = [];
  const networkRequired = !!network;
  releaseAnswer();
  for (const entry of attempts.values()) if (!entry.settled) await cancelAttempt(entry);
  await Promise.all([...attempts.values()].map(entry => entry.work));
  attempts.clear();
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
  members.clear(); expectedPeers.clear(); memberStableIds.clear(); memberRtcPeers.clear();
  await Promise.all([...exchanges.values()].map(entry => entry.work));
  exchanges.clear();
  errors.push(...retirementErrors);
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
          networkDiagnostics.push({ delayMs, observed, realRtcObjects: rtcObjects(generationPeerStart) });
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
    ownedMemberHandles: ownedMemberCount - memberFreeCount, networkRequired, networkClosed, networkFreed,
    memberDiagnostics: diagnostics, networkDiagnostics, realRtcObjects: rtcObjects(generationPeerStart),
    ownedRtcAllocationWitness: { generation, allocatedPeerCount: realPeers.length - generationPeerStart,
      knownStableIds: [...generationRtcStableIds] }, lateHandleRetirements, errors, cleanupSucceeded: errors.length === 0,
    cleanupElapsedMs: performance.now() - started };
}

async function run(request) {
  const args = request.args ?? {};
  switch (request.op) {
    case 'capability': return rtcCapability();
    case 'rtc_fixture_snapshot': return feature(globalThis.__mio76RtcFixture, 'snapshot')();
    case 'rtc_objects': return rtcObjects(args.after ?? 0);
    case 'hold_next_answer': {
      if (answerHold && !answerHold.released) throw new Error('owned answer hold is already active');
      answerHold = { claimed: false, answerCreated: false, released: false };
      answerHold.work = new Promise(resolve => { answerHold.resolve = resolve; });
      return { armed: true };
    }
    case 'release_answer': releaseAnswer(); return { released: true };
    case 'begin_connect': return beginAttempt(args, 'connect');
    case 'begin_accept': return beginAttempt(args, 'accept');
    case 'attempt_status': return attemptFacts(pending(args.name));
    case 'cancel_attempt': return await cancelAttempt(pending(args.name));
    case 'finish_attempt': {
      const entry = pending(args.name);
      if (!entry.settled) throw new Error('poll settlement before finish to keep cancellation control available');
      return attemptFacts(entry);
    }
    case 'begin_serve_exchange': {
      const key = args.operation;
      if (typeof key !== 'string' || !key || exchanges.has(key)) throw new Error('owned exchange name cannot be reused');
      const entry = { settled: false, started: performance.now() };
      exchanges.set(key, entry);
      entry.work = feature(member(args.name), 'serve_exchange')().then(value => { entry.result = value; },
        error => { entry.failureClass = error?.name ?? typeof error; }).finally(() => {
          entry.settled = true; entry.elapsedMs = performance.now() - entry.started;
        });
      return { started: true };
    }
    case 'serve_status': {
      const entry = exchanges.get(args.operation);
      if (!entry) throw new Error('owned exchange does not exist');
      return { settled: entry.settled, failureClass: entry.failureClass, callerElapsedMs: entry.elapsedMs };
    }
    case 'finish_serve_exchange': {
      const entry = exchanges.get(args.operation);
      if (!entry?.settled) throw new Error('poll server settlement before finish');
      exchanges.delete(args.operation);
      if (entry.failureClass) throw new Error('owned server exchange failed');
      return { message: entry.result, callerElapsedMs: entry.elapsedMs };
    }
    case 'start':
      if (network) throw new Error('owned network is already running');
      await claimOwnedStore();
      network = await BrowserNetwork.start(JSON.stringify(args.configuration ?? {}));
      generation++; generationPeerStart = realPeers.length; generationRtcStableIds.clear(); cleanupResult = undefined;
      return { id: network.id(), status: parsed(network.status()) };
    case 'status': return parsed(feature(network, 'status')());
    case 'request_admission': return parsed(await feature(network, 'request_admission')(args.invitation));
    case 'complete_admission': return parsed(await feature(network, 'complete_admission')());
    case 'material_receipt': return parsed(await feature(network, 'admission_material_receipt')());
    case 'exchange': return await member(args.name).exchange(args.message);
    case 'member_diagnostics': return memberDiagnostics(member(args.name));
    case 'path_diagnostics': return parsed(feature(network, 'path_diagnostics')());
    case 'rtc_diagnostics': return diagnostic(parsed(feature(network, 'rtc_invalidation_diagnostics')()));
    case 'sync_replica': return parsed(await feature(network, 'sync_replica')(args.target));
    case 'replica_diagnostics': return parsed(feature(network, 'replica_diagnostics')());
    case 'close_member': {
      const name = args.name ?? 'member';
      const errors = await retire(member(name)); members.delete(name); expectedPeers.delete(name); memberStableIds.delete(name); memberRtcPeers.delete(name);
      return { closed: errors.length === 0, name, errors };
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
      memberConnection: typeof BrowserNetwork.prototype.member_connection_attempt === 'function',
      memberAccept: typeof BrowserNetwork.prototype.member_accept_attempt === 'function' } });
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
  await post({ event: 'terminal', failureClass: error.name, cleanup }).catch(() => {});
});
addEventListener('pagehide', () => { stop().catch(() => {}); }, { once: true });
