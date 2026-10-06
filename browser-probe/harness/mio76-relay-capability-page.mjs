// Owned M76 prelude. The shared Android page still owns every real SDK call.
import { rtcCapabilityPrelude } from './rtc-capability-fixture.mjs';

export function installNecessaryAnswerFailure() {
  const peer = globalThis.RTCPeerConnection;
  if (typeof peer !== 'function' || typeof peer.prototype.createAnswer !== 'function') {
    throw new Error('owned missing-Answer injection requires the real Answer API first');
  }
  if (!Reflect.deleteProperty(peer.prototype, 'createAnswer')
    || typeof peer.prototype.createAnswer !== 'undefined') {
    throw new Error('owned necessary Answer API removal failed');
  }
}

export function installCumulativeReceipt(mode) {
  const fixture = globalThis.__mio76RtcFixture;
  if (!fixture || globalThis.__mio76CumulativeObserver) {
    throw new Error('owned cumulative RTC observer cannot be installed');
  }
  const snapshot = fixture.snapshot.bind(fixture);
  const sockets = [];
  const PreviousWebSocket = globalThis.WebSocket;
  function CloseObservedWebSocket(...args) {
    const socket = new PreviousWebSocket(...args);
    const record = { socket, closeCalls: 0, closeFailures: 0, firstCloseAtMs: null,
      stateBeforeFirstClose: null, stateAfterFirstClose: null, listeners: [], trackingComplete: true };
    sockets.push(record);
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
    // The inherited fixture installed only its own listeners before returning
    // this socket. Later registrations belong to the SDK, not that observer.
    const add = socket.addEventListener.bind(socket);
    const remove = socket.removeEventListener.bind(socket);
    const capture = options => typeof options === 'boolean' ? options : !!options?.capture;
    socket.addEventListener = (type, callback, options) => {
      const result = add(type, callback, options);
      if (options?.once || options?.signal) record.trackingComplete = false;
      if (callback && !record.listeners.some(item => item.type === type
        && item.callback === callback && item.capture === capture(options))) {
        record.listeners.push({ type, callback, capture: capture(options) });
      }
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
  CloseObservedWebSocket.prototype = PreviousWebSocket.prototype;
  Object.setPrototypeOf(CloseObservedWebSocket, PreviousWebSocket);
  globalThis.WebSocket = CloseObservedWebSocket;
  let resetAttempts = 0;
  const injection = {
    mode,
    necessaryAnswerMissing: mode === 'missing-answer'
      ? typeof globalThis.RTCPeerConnection.prototype.createAnswer === 'undefined' : null,
  };
  fixture.snapshot = () => ({ ...snapshot(), cumulative: true, resetAttempts, injection,
    sdkCloseObserverInstalled: globalThis.__mio76SdkRetirement?.installed === true,
    webSocketCloseWitnesses: sockets.map((record, socketIndex) => {
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
    }) });
  fixture.resetCounts = () => {
    resetAttempts++;
    throw new Error('M76 counters are cumulative across every SDK generation');
  };
  globalThis.__mio76CumulativeObserver = { mode };
  globalThis.__mio76SdkRetirement = { installed: false, records: [] };

  // The existing page constructs this DTO only after its real close/free. Add
  // object retirement to that same reply before the page/tab can disappear.
  // Failed SDK DTOs are forwarded byte-for-byte; no diagnostic getter runs here.
  const originalFetch = globalThis.fetch.bind(globalThis);
  globalThis.fetch = async (input, options) => {
    const address = new URL(typeof input === 'string' ? input : input.url, location.href);
    if (address.origin === location.origin && address.pathname === '/result'
      && options?.method === 'POST' && typeof options.body === 'string') {
      const reply = JSON.parse(options.body);
      if (reply.ok === true && reply.result && Object.hasOwn(reply.result, 'networkClosed')
        && Object.hasOwn(reply.result, 'networkFreed')) {
        const started = performance.now();
        const immediate = fixture.snapshot();
        // Timer callbacks may run just before the requested monotonic interval.
        // Sample only after the existing 100 ms observation actually elapsed.
        while (performance.now() - started < 100) {
          await new Promise(resolve => setTimeout(resolve,
            Math.max(1, 100 - (performance.now() - started))));
        }
        reply.result.mio76Objects = {
          immediate,
          late: fixture.snapshot(),
          requestedLateDelayMs: 100,
          observedLateDelayMs: performance.now() - started,
          afterSdkCloseFree: reply.result.networkClosed === true && reply.result.networkFreed === true,
          finalShutdown: Object.hasOwn(reply.result, 'storage'),
        };
        const sdk = globalThis.__mio76SdkRetirement;
        reply.result.mio76SdkRetirement = {
          installed: sdk.installed, observationFailureClass: sdk.observationFailureClass ?? null,
          record: sdk.records.at(-1) ?? null,
        };
        options = { ...options, body: JSON.stringify(reply) };
      }
    }
    return originalFetch(input, options);
  };
}

export function installSdkRetirementObserver(BrowserNetwork) {
  const probe = globalThis.__mio76SdkRetirement;
  const prototype = BrowserNetwork?.prototype;
  if (!probe || probe.installed || typeof prototype?.close !== 'function'
    || typeof prototype?.free !== 'function' || typeof prototype?.path_diagnostics !== 'function') {
    throw new Error('actual Browser SDK retirement methods are required');
  }
  const close = prototype.close;
  const free = prototype.free;
  const records = new WeakMap();
  prototype.close = async function (...args) {
    const record = { index: probe.records.length + 1, closeReturned: false, freeReturned: false,
      freeMatchedCloseNetwork: false, carriers: null, queueGetterSucceeded: false };
    probe.records.push(record);
    records.set(this, record);
    // A failed SDK close is rethrown unchanged, without a later getter.
    const result = await close.apply(this, args);
    record.closeReturned = true;
    try {
      const raw = this.path_diagnostics();
      const diagnostics = typeof raw === 'string' ? JSON.parse(raw) : raw;
      record.carriers = Array.isArray(diagnostics?.carriers) ? diagnostics.carriers.map(carrier => ({
        ready: carrier.ready, closed: carrier.closed, queuedPackets: carrier.queued_packets,
        pendingBytes: carrier.pending_bytes, bufferedBytes: carrier.buffered_bytes,
      })) : null;
      record.queueGetterSucceeded = true;
    } catch (error) { record.queueGetterFailureClass = error?.name ?? typeof error; }
    return result;
  };
  prototype.free = function (...args) {
    const result = free.apply(this, args);
    const record = records.get(this);
    if (record) { record.freeReturned = true; record.freeMatchedCloseNetwork = true; }
    return result;
  };
  probe.installed = true;
}

export function relayCapabilityPrelude(mode = 'missing-answer') {
  const observerModes = {
    'missing-answer': 'observe',
    'pc-denied': 'peer-connection-denied',
    'dc-denied': 'data-channel-denied',
  };
  if (!Object.hasOwn(observerModes, mode)) throw new Error('unknown M76 capability mode');
  return (mode === 'missing-answer' ? `(${installNecessaryAnswerFailure.toString()})();\n` : '')
    + rtcCapabilityPrelude(observerModes[mode]) + '\n'
    + `(${installCumulativeReceipt.toString()})(${JSON.stringify(mode)});\n`
    + `import('/pkg/miopunch_browser_probe.js').then(module => (${installSdkRetirementObserver.toString()})(module.BrowserNetwork))`
    + `.catch(error => { globalThis.__mio76SdkRetirement.observationFailureClass = error?.name ?? typeof error; });\n`;
}
