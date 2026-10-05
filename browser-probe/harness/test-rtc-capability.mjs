// Runtime adapter boundary only; the complete SDK relay stories are separate.
import assert from 'node:assert/strict';
import test from 'node:test';
import { probeRtcCapability } from '../src/rtc_capability.js';

test('disabled RTC stays unavailable without accessing the runtime API', () => {
  Object.defineProperty(globalThis, 'RTCPeerConnection', {
    configurable: true,
    get() { throw new Error('RTC policy must prevent runtime access'); },
  });
  try {
    const result = JSON.parse(probeRtcCapability(false, 'mio-quic-datagram/1', 'miopunch/quic-datagram/1'));
    assert.equal(result.supported, null);
    assert.equal(result.permitted, false);
    assert.equal(result.effective, false);
    assert.equal(result.reason, 'disabled-by-policy');
    assert.equal(result.probe.pcCreated, false);
    assert.equal(result.probe.channelCreated, false);
  } finally {
    delete globalThis.RTCPeerConnection;
  }
});

test('missing RTC API preserves permission while withholding runtime support', () => {
  const result = JSON.parse(probeRtcCapability(true, 'mio-quic-datagram/1', 'miopunch/quic-datagram/1'));
  assert.equal(result.supported, false);
  assert.equal(result.permitted, true);
  assert.equal(result.effective, false);
  assert.equal(result.reason, 'api-missing');
  assert.equal(result.probe.pcCreated, false);
});

test('a policy-rejected PeerConnection does not become an available RTC runtime', () => {
  class RejectedPeerConnection {
    constructor() { throw new DOMException('blocked by host policy', 'NotAllowedError'); }
    createDataChannel() {}
    createOffer() {}
    createAnswer() {}
    setLocalDescription() {}
    setRemoteDescription() {}
    getStats() {}
    close() {}
  }
  globalThis.RTCPeerConnection = RejectedPeerConnection;
  try {
    const result = JSON.parse(probeRtcCapability(true, 'mio-quic-datagram/1', 'miopunch/quic-datagram/1'));
    assert.equal(result.reason, 'peer-connection-rejected');
    assert.equal(result.supported, false);
    assert.equal(result.permitted, true);
    assert.equal(result.effective, false);
    assert.equal(result.probe.pcCreated, false);
  } finally {
    delete globalThis.RTCPeerConnection;
  }
});

test('a rejected DataChannel retires the created PeerConnection before relay fallback', () => {
  let peer;
  class ChannelRejectedPeerConnection {
    constructor() { this.connectionState = 'new'; peer = this; }
    createDataChannel() { throw new DOMException('channel denied by policy', 'NotAllowedError'); }
    createOffer() {}
    createAnswer() {}
    setLocalDescription() {}
    setRemoteDescription() {}
    getStats() {}
    close() { this.connectionState = 'closed'; }
  }
  globalThis.RTCPeerConnection = ChannelRejectedPeerConnection;
  try {
    const result = JSON.parse(probeRtcCapability(true, 'mio-quic-datagram/1', 'miopunch/quic-datagram/1'));
    assert.equal(result.reason, 'data-channel-rejected');
    assert.equal(result.effective, false);
    assert.equal(result.probe.pcCreated, true);
    assert.equal(result.probe.pcClosed, true);
    assert.equal(peer.connectionState, 'closed');
  } finally {
    delete globalThis.RTCPeerConnection;
  }
});

test('a runtime missing createAnswer is withheld before creating RTC objects', () => {
  let peerCreated = 0, channelCreated = 0;
  class OfferOnlyPeerConnection {
    constructor() { peerCreated++; this.connectionState = 'new'; }
    createOffer() {}
    setLocalDescription() {}
    setRemoteDescription() {}
    getStats() {}
    close() { this.connectionState = 'closed'; }
    createDataChannel(label, options) {
      channelCreated++;
      return { ...options, label, maxPacketLifeTime: null, binaryType: 'arraybuffer', send() {}, close() {} };
    }
  }
  globalThis.RTCPeerConnection = OfferOnlyPeerConnection;
  try {
    const result = JSON.parse(probeRtcCapability(true, 'mio-quic-datagram/1', 'miopunch/quic-datagram/1'));
    assert.equal(result.supported, false);
    assert.equal(result.permitted, true);
    assert.equal(result.effective, false);
    assert.equal(result.reason, 'api-missing');
    assert.equal(peerCreated, 0);
    assert.equal(channelCreated, 0);
  } finally {
    delete globalThis.RTCPeerConnection;
  }
});
