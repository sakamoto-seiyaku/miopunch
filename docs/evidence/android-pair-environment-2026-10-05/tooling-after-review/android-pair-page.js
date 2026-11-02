// Local ADB-controlled test page. Network traffic still uses the formal WASM SDK.
// Requests/results may contain private fixture material: never console-log them.
import init, { BrowserNetwork } from '/pkg/miopunch_browser_probe.js';

const actor = new URL(location.href).searchParams.get('actor');
const environmentRtcProbe = new URL(location.href).searchParams.get('environmentRtcProbe') !== 'false';
const members = new Map();
let network;
let stopping = false;
let cleanupRunning;

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
  const pending = cleanupOwned();
  cleanupRunning = pending;
  pending.finally(() => { if (cleanupRunning === pending) cleanupRunning = undefined; }).catch(() => {});
  return pending;
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
  if (network) {
    try { await network.close(); networkClosed = true; }
    catch (error) { failed('network', 'close', error); }
    try { await network.free(); networkFreed = true; }
    catch (error) { failed('network', 'free', error); }
    if (networkFreed) network = undefined;
  }
  return { ownedMemberCount, memberCloseCount, memberFreeCount,
    ownedMemberHandles: ownedMemberCount - memberFreeCount, networkClosed, networkFreed,
    memberDiagnostics: diagnostics, errors, cleanupSucceeded: errors.length === 0,
    cleanupElapsedMs: performance.now() - started };
}

async function run(request) {
  const args = request.args ?? {};
  switch (request.op) {
    case 'capability': return rtcCapability();
    case 'rtc_fixture_snapshot': return feature(globalThis.__mio76RtcFixture, 'snapshot')();
    case 'rtc_fixture_reset': return feature(globalThis.__mio76RtcFixture, 'resetCounts')();
    case 'rtc_fixture_restore': return feature(globalThis.__mio76RtcFixture, 'restore')();
    case 'start':
      if (network) throw new Error('owned network is already running');
      network = await BrowserNetwork.start(JSON.stringify(args.configuration ?? {}));
      return { id: network.id(), status: parsed(network.status()) };
    case 'status': return parsed(feature(network, 'status')());
    case 'request_admission': return parsed(await feature(network, 'request_admission')(args.invitation));
    case 'complete_admission': return parsed(await feature(network, 'complete_admission')());
    case 'material_receipt': return parsed(await feature(network, 'admission_material_receipt')());
    case 'open_member': {
      const name = args.name ?? 'member';
      if (members.has(name)) throw new Error('owned member handle name is already in use');
      members.set(name, await feature(network, 'open_member_connection')(args.target));
      return { opened: true, name };
    }
    case 'exchange': return await member(args.name).exchange(args.message);
    case 'member_diagnostics': return memberDiagnostics(member(args.name));
    case 'path_diagnostics': return parsed(feature(network, 'path_diagnostics')());
    case 'sync_replica': return parsed(await feature(network, 'sync_replica')(args.target));
    case 'replica_diagnostics': return parsed(feature(network, 'replica_diagnostics')());
    case 'close_member': {
      const name = args.name ?? 'member';
      member(name).close(); member(name).free(); members.delete(name);
      return { closed: true, name };
    }
    case 'stop': return await stop();
    case 'shutdown': stopping = true; return await stop();
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
  await post({ event: 'terminal', failureClass: error.name, cleanup }).catch(() => {});
});
addEventListener('pagehide', () => { stop().catch(() => {}); }, { once: true });
