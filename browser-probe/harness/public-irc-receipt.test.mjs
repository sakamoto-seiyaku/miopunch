import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { redactPublicIrcReceipt, publicIrcCleanupComplete } from './public-irc-receipt.mjs';

const historical = JSON.parse(readFileSync(new URL('../../docs/evidence/public-irc-restricted-2026-10-05/final/qualification.json', import.meta.url)));

test('public serialization removes nested member identities and keeps their count', () => {
  const input = { nativeStatus: { members: ['temporary-member-a', 'temporary-member-b'],
    requests: [{ applicant: 'temporary-applicant', request_id: 'temporary-request' }] },
    native: [{ id: 'temporary-node', invitation: 'temporary-invitation' }],
    diagnostics: { request: { remote_id: 'temporary-peer', business_handle: false } } };
  const output = redactPublicIrcReceipt(input);
  assert.equal(output.nativeStatus.members, 2);
  assert.equal(output.diagnostics.request.business_handle, false);
  assert.equal(JSON.stringify(output).includes('temporary-'), false);
  assert.deepEqual(input.nativeStatus.members, ['temporary-member-a', 'temporary-member-b']);
});

test('actual historical cleanup passes while IRC qualification remains unavailable', () => {
  assert.equal(historical.passed, true);
  assert.equal(publicIrcCleanupComplete(historical.cleanup), true);
  assert.equal(historical.available, false);
});

test('native nonzero exit or nongraceful receipt cannot pass cleanup', () => {
  for (const mutate of [
    c => { c.nativeExit.code = 1; },
    c => { c.nativeExit.signal = 'SIGKILL'; },
    c => { c.nativeReceipt.graceful_close = false; },
    c => { c.nativeReceipt.event = 'status'; },
    c => { delete c.nativeReceipt; },
  ]) {
    const cleanup = structuredClone(historical.cleanup); mutate(cleanup);
    assert.equal(publicIrcCleanupComplete(cleanup), false);
  }
});

test('native queue, assembly or driver residue cannot pass cleanup', () => {
  for (const mutate of [
    c => { c.nativeReceipt.traffic.protocol.queue_pending = 1; },
    c => { c.nativeReceipt.traffic.pending = 1; },
    c => { c.nativeReceipt.traffic.driver_active = 1; },
  ]) {
    const cleanup = structuredClone(historical.cleanup); mutate(cleanup);
    assert.equal(publicIrcCleanupComplete(cleanup), false);
  }
});

test('forced exit or incomplete immediate and late socket observations fail cleanup', () => {
  for (const mutate of [
    c => { c.forcedKill = true; },
    c => { c.activeBrowserIntervals = 1; },
    c => { c.immediate.allWebSocketsClosed = false; },
    c => { c.late.allWebSocketsClosed = false; },
    c => { c.late.nativeExited = false; },
  ]) {
    const cleanup = structuredClone(historical.cleanup); mutate(cleanup);
    assert.equal(publicIrcCleanupComplete(cleanup), false);
  }
});
