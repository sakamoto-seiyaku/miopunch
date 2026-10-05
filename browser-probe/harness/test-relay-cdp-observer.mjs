import assert from 'node:assert/strict';
import test from 'node:test';
import { networkObservation } from './relay-cdp-observer.mjs';

test('CDP diagnostics keep handshake status while dropping headers and all frame payloads', () => {
  const sockets = new Map();
  const requestId = 'private-request-id';
  assert.deepEqual(networkObservation({ method: 'Network.webSocketCreated', params: {
    requestId, url: 'wss://example.com/private-route?secret=private-value',
  } }, sockets), { event: 'socket-created', number: 1, origin: 'wss://example.com' });
  assert.deepEqual(networkObservation({ method: 'Network.webSocketHandshakeResponseReceived', params: {
    requestId, timestamp: 1, response: { status: 101, headers: { Authorization: 'private-value' } },
  } }, sockets), { event: 'handshake-response', socket: 1, origin: 'wss://example.com', timestamp: 1, status: 101 });
  assert.equal(networkObservation({ method: 'Network.webSocketFrameSent', params: {
    requestId, response: { payloadData: 'private SDP or encrypted fixture material' },
  } }, sockets), undefined);
});

test('socket errors retain fixed browser error codes rather than arbitrary error text', () => {
  const sockets = new Map([['owned', { number: 1, origin: 'wss://example.com' }]]);
  assert.deepEqual(networkObservation({ method: 'Network.loadingFailed', params: {
    requestId: 'owned', timestamp: 2, errorText: 'net::ERR_CERT_AUTHORITY_INVALID secret=private-value',
  } }, sockets), { event: 'socket-error', socket: 1, origin: 'wss://example.com', timestamp: 2,
    networkCode: 'net::ERR_CERT_AUTHORITY_INVALID', handshakeHttpStatus: null, cancelled: false });
});
