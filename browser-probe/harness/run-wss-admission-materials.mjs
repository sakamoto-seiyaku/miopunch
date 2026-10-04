import assert from 'node:assert/strict';
import { releaseProof, streamProgress, cleanupProof, observeTraffic, traffic } from './run-rtc-member-replica.mjs';

const status = async page => JSON.parse(await page.evaluate(() => window.network.status()));
const diagnostic = async page => JSON.parse(await page.evaluate(() => window.network.path_diagnostics()));

async function observeCommits(page) {
  await page.evaluate(() => {
    window.admissionCommits = [];
    const transaction = IDBDatabase.prototype.transaction;
    IDBDatabase.prototype.transaction = function(...args) {
      const tx = transaction.apply(this, args);
      if (args[1] === 'readwrite') {
        const event = { sequence: window.admissionCommits.length, durability: tx.durability, committed: false, aborted: false };
        window.admissionCommits.push(event);
        tx.addEventListener('complete', () => { event.committed = true; event.completedMs = performance.now(); });
        tx.addEventListener('abort', () => { event.aborted = true; });
      }
      return tx;
    };
  });
}

async function reopen(page, configuration, origin, identity, bounded) {
  await bounded(page.evaluate(async () => { await window.network.close(); window.network.free(); window.network = null; }));
  await bounded(page.goto(`${origin}/network.html?config=${encodeURIComponent(JSON.stringify(configuration))}`));
  assert.equal(await bounded(page.evaluate(() => window.ready)), identity);
  return status(page);
}

async function realBusiness({ page, native, ready, report, bounded, save }) {
  await observeTraffic(page);
  const released = native.wait('member-released', true); released.catch(() => {});
  await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
  const before = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
  const nativeRelease = await released;
  const nativeFinal = report.native.find(entry => entry.event === 'rtc-path' && entry.report.memberStableId === nativeRelease.stableId)?.report;
  releaseProof(before.bootstrap, nativeFinal);
  assert.equal(nativeRelease.alpn, 'miopunch/browser-member/rtc-path/3');
  const carrierBefore = (await diagnostic(page)).carriers;
  const physicalBefore = (await traffic(page))[0];
  const handler = native.wait('business', true); handler.catch(() => {});
  const reply = await bounded(page.evaluate(() => window.member.exchange('PING')));
  assert.equal(reply, 'PONG');
  const received = await handler;
  assert.equal(received.stableId, nativeRelease.stableId); assert.equal(received.memberMessage, true);
  const after = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
  const physicalAfter = (await traffic(page))[0];
  assert(physicalAfter.sentBytes > physicalBefore.sentBytes && physicalAfter.receivedBytes > physicalBefore.receivedBytes);
  streamProgress(before.paths, after.paths);
  const carrierAfter = (await diagnostic(page)).carriers;
  for (const field of ['sent_bytes', 'received_bytes']) assert.equal(carrierAfter[0].traffic[field], carrierBefore[0].traffic[field]);
  const cleanup = native.wait('rtc-path-cleanup', true); cleanup.catch(() => {});
  await page.evaluate(() => window.member.close());
  const cleaned = await cleanup; cleanupProof(cleaned);
  await bounded(page.waitForFunction(() => window.replicaRtcTraffic.every(ticket => ticket.peer.connectionState === 'closed')));
  const closedPhysical = await traffic(page, true);
  assert(closedPhysical.every(ticket => ticket.closed && ticket.callbacksDetached && ticket.observersDetached));
  const value = { purpose: 'NEW Member M', nativeRelease, nativeFinal, before, after, reply, received, cleaned, carrierBefore, carrierAfter, physicalBefore, physicalAfter, closedPhysical };
  save(); return value;
}

async function freshApplicant(context, report, configuration, bounded) {
  const isolated = await context.browser().newContext();
  const page = await isolated.newPage();
  await bounded(page.goto(`${report.origin}/network.html?config=${encodeURIComponent(JSON.stringify(configuration))}`));
  const identity = await bounded(page.evaluate(() => window.ready));
  const initial = await status(page);
  assert.equal(initial.authorization_saved, false); assert.equal(initial.secret_saved, false);
  return { context: isolated, page, identity, initial };
}

async function approveApplicant(applicant, native, invitation, bounded) {
  assert.equal(await bounded(applicant.page.evaluate(inv => window.network.request_admission(inv), invitation)), 'AwaitingApproval');
  const requests = await native.command('admission-status', 'admission-status');
  const request = requests.requests.find(request => request.applicant === applicant.identity);
  assert(request); assert.equal(request.authorization, null);
  const approval = await native.command(`approve-request ${request.applicant} ${request.request}`, 'approved');
  assert.equal(approval.after - approval.before, 1);
  assert.equal(await bounded(applicant.page.evaluate(inv => window.network.request_admission(inv), invitation)), 'PendingMaterials');
  return { request, approval, material: JSON.parse(await applicant.page.evaluate(() => window.network.admission_material_receipt())) };
}

async function pendingAndIdentity({ context, native, ready, report, bounded, configuration }) {
  const applicant = await freshApplicant(context, report, configuration, bounded);
  const holder = await freshApplicant(context, report, configuration, bounded);
  try {
    assert.notEqual(applicant.identity, holder.identity);
    const approved = await approveApplicant(applicant, native, ready.invitation, bounded);
    await assert.rejects(() => applicant.page.evaluate(id => window.network.open_member_connection(id), ready.id));
    const reopenedPending = await reopen(applicant.page, {}, report.origin, applicant.identity, bounded);
    assert.equal(reopenedPending.stage, 'PendingMaterials'); assert.equal(reopenedPending.secret_saved, false);
    const senderBefore = await native.command('admission-status', 'admission-status');
    assert.equal(await bounded(holder.page.evaluate(inv => window.network.request_admission(inv), ready.invitation)), 'AwaitingApproval');
    const collision = JSON.parse(await bounded(holder.page.evaluate(({ request, claim }) => window.network.admission_identity_probe(request, claim), { request: approved.request.request, claim: holder.identity })));
    assert.equal(collision.approved_material, false); assert.equal(collision.response, 0);
    await assert.rejects(() => bounded(holder.page.evaluate(({ request, claim }) => window.network.admission_identity_probe(request, claim), { request: approved.request.request, claim: applicant.identity })));
    const holderAfter = await status(holder.page);
    assert.equal(holderAfter.authorization_saved, false); assert.equal(holderAfter.secret_saved, false);
    await assert.rejects(() => holder.page.evaluate(() => window.network.complete_admission()));
    const isolatedRequests = await native.command('admission-status', 'admission-status');
    assert(isolatedRequests.requests.filter(request => request.applicant === holder.identity).every(request => request.authorization === null));
    assert.equal(isolatedRequests.count, senderBefore.count);
    assert.equal(await bounded(applicant.page.evaluate(inv => window.network.request_admission(inv), ready.invitation)), 'PendingMaterials');
    const sameGrant = JSON.parse(await applicant.page.evaluate(() => window.network.admission_material_receipt()));
    assert.equal(sameGrant.authorization, approved.approval.authorization);
    assert.equal(sameGrant.request, approved.request.request);
    // This is the next real write after saved authorization, not a private
    // snapshot mutation: abort the secret transaction before its put commits.
    await applicant.page.evaluate(() => {
      const put = IDBObjectStore.prototype.put;
      window.secretAbort = { triggered: false, aborted: false };
      IDBObjectStore.prototype.put = function(...args) {
        IDBObjectStore.prototype.put = put;
        const tx = this.transaction;
        window.secretAbort.triggered = true;
        tx.addEventListener('abort', () => { window.secretAbort.aborted = true; });
        tx.abort();
        throw new DOMException('owned secret transaction abort', 'AbortError');
      };
    });
    await assert.rejects(() => bounded(applicant.page.evaluate(() => window.network.complete_admission())));
    const abort = await applicant.page.evaluate(() => window.secretAbort);
    assert.equal(abort.triggered, true); assert.equal(abort.aborted, true);
    const failedSave = await status(applicant.page); assert.equal(failedSave.stage, 'PendingMaterials');
    const failedEvents = (await diagnostic(applicant.page)).request.material_events;
    assert(failedEvents.some(event => event.stage === 'secret-received-generation-verified'));
    assert(!failedEvents.some(event => event.stage === 'secret-committed' || event.stage === 'SAVE-sent'));
    const afterAbortReopen = await reopen(applicant.page, {}, report.origin, applicant.identity, bounded);
    assert.equal(afterAbortReopen.stage, 'PendingMaterials');
    assert.equal(await bounded(applicant.page.evaluate(inv => window.network.request_admission(inv), ready.invitation)), 'PendingMaterials');
    assert.equal(await bounded(applicant.page.evaluate(() => window.network.complete_admission())), 'Complete');
    const resumed = JSON.parse(await applicant.page.evaluate(() => window.network.admission_material_receipt()));
    const senderAfter = await native.command('admission-status', 'admission-status');
    assert.equal(senderAfter.count, senderBefore.count);
    assert.equal(resumed.authorization, approved.approval.authorization); assert.equal(resumed.request, approved.request.request);
    const approvalInvocations = report.native.filter(event => event.event === 'approved' && event.applicant === applicant.identity).length;
    assert.equal(approvalInvocations, 1);
    report.pendingAndIdentity = { applicant: applicant.identity, holder: holder.identity, approved, reopenedPending, senderBefore, collision, holderAfter,
      isolatedRequests, sameGrant, abort, failedSave, failedEvents, afterAbortReopen, resumed, senderAfter, approvalInvocations };
  } finally { await applicant.context.close(); await holder.context.close(); }
}

async function precodeResume({ context, native, ready, report, bounded, configuration, restartNative }) {
  const applicant = await freshApplicant(context, report, configuration, bounded);
  const holder = await freshApplicant(context, report, configuration, bounded);
  try {
    const issued = await native.command('issue', 'issued');
    const code = issued.code;
    assert.equal(await bounded(applicant.page.evaluate(code => window.network.request_precode(code), code)), 'PendingMaterials');
    const material = JSON.parse(await applicant.page.evaluate(() => window.network.admission_material_receipt()));
    const originalAttempt = (await diagnostic(applicant.page)).request;
    await bounded(applicant.page.waitForTimeout(originalAttempt.remaining_caller_ms + 150));
    const expiredCompletion = await bounded(applicant.page.evaluate(async () => {
      const start = performance.now();
      try { await window.network.complete_admission(); return { completed: true }; }
      catch (error) { return { completed: false, elapsedMs: performance.now() - start, error: String(error) }; }
    }));
    assert.equal(expiredCompletion.completed, false); assert(expiredCompletion.elapsedMs < 300);
    assert.match(expiredCompletion.error, /caller deadline expired/);
    assert.equal((await status(applicant.page)).secret_saved, false);
    const pending = await reopen(applicant.page, {}, report.origin, applicant.identity, bounded);
    assert.equal(pending.stage, 'PendingMaterials');
    const before = await native.command('usage', 'usage');
    assert.equal(before.uses, 1); assert.equal(before.trusted, true); assert.equal(before.redemptions.length, 1);
    assert.equal(before.redemptions[0].applicant, applicant.identity); assert.equal(before.redemptions[0].authorization, material.authorization);
    native = await restartNative();
    const restored = await native.wait('ready'); assert.equal(restored.id, ready.id);
    configuration.contact_routes[0].address = restored.route;
    const afterNativeReopen = await native.command('usage', 'usage'); assert.deepEqual(afterNativeReopen, before);
    await reopen(applicant.page, configuration, report.origin, applicant.identity, bounded);
    assert.equal(await bounded(applicant.page.evaluate(code => window.network.request_precode(code), code)), 'PendingMaterials');
    assert.equal(await bounded(applicant.page.evaluate(() => window.network.complete_admission())), 'Complete');
    const completed = JSON.parse(await applicant.page.evaluate(() => window.network.admission_material_receipt()));
    assert.equal(completed.request, material.request); assert.equal(completed.authorization, material.authorization);
    await reopen(holder.page, configuration, report.origin, holder.identity, bounded);
    await assert.rejects(() => bounded(holder.page.evaluate(code => window.network.request_precode(code), code)));
    const holderAfter = await status(holder.page); assert.equal(holderAfter.secret_saved, false); assert.equal(holderAfter.authorization_saved, false);
    const after = await native.command('usage', 'usage');
    assert.equal(after.uses, 1); assert.equal(after.trusted, true); assert.equal(after.redemptions.length, 1);
    assert.equal(after.redemptions[0].authorization, material.authorization); assert.equal(after.redemptions[0].receiptStage, 'Complete');
    assert.equal(after.count, before.count);
    report.precodeResume = { applicant: applicant.identity, holder: holder.identity, material, originalAttempt, expiredCompletion, pending, before, afterNativeReopen, completed, holderAfter, after,
      originalNodeReopened: restored.id === ready.id, sameUse: true };
    return native;
  } finally { await applicant.context.close(); await holder.context.close(); }
}

async function completeThenRtcFailure({ page, native, ready, report, bounded, configuration }) {
  const savedBefore = await reopen(page, configuration, report.origin, report.browserIdentity, bounded);
  assert.equal(savedBefore.stage, 'Complete');
  const materialBefore = JSON.parse(await page.evaluate(() => window.network.admission_material_receipt()));
  await observeTraffic(page);
  await page.evaluate(() => {
    const create = RTCPeerConnection.prototype.createDataChannel;
    window.droppedRtc = { packets: 0, bytes: 0, open: false };
    RTCPeerConnection.prototype.createDataChannel = function(...args) {
      const channel = create.apply(this, args);
      channel.send = function(bytes) {
        window.droppedRtc.open ||= this.readyState === 'open';
        window.droppedRtc.packets++; window.droppedRtc.bytes += bytes.byteLength ?? bytes.length;
      };
      return channel;
    };
    window.restoreRtcSend = () => { RTCPeerConnection.prototype.createDataChannel = create; };
  });
  const failure = await bounded(page.evaluate(async id => {
    const attempt = window.network.member_connection_attempt(id, 2000);
    const started = performance.now();
    try { window.unexpectedMember = await attempt.connect(); window.unexpectedMember.close(); return { returnedHandle: true }; }
    catch (error) { return { returnedHandle: false, error: String(error), elapsedMs: performance.now() - started }; }
    finally { attempt.free(); window.restoreRtcSend(); }
  }, ready.id));
  assert.equal(failure.returnedHandle, false); assert(failure.elapsedMs < 3000);
  const drop = await page.evaluate(() => window.droppedRtc); assert(drop.open && drop.packets > 0);
  const afterFailure = await status(page); assert.equal(afterFailure.stage, 'Complete');
  await bounded(page.waitForFunction(() => window.replicaRtcTraffic.every(ticket => ticket.peer.connectionState === 'closed')));
  const failedPhysical = await traffic(page, true);
  assert(failedPhysical.every(ticket => ticket.closed && ticket.callbacksDetached && ticket.observersDetached));
  const reopened = await reopen(page, {}, report.origin, report.browserIdentity, bounded);
  assert.equal(reopened.stage, 'Complete');
  const materialAfter = JSON.parse(await page.evaluate(() => window.network.admission_material_receipt()));
  assert.deepEqual(materialAfter, materialBefore);
  report.completeRtcFailure = { savedBefore, materialBefore, failure, drop, afterFailure, failedPhysical, reopened, materialAfter };
}

export async function runWssAdmissionMaterials({ page, context, native, ready, report, bounded, save, configuration, restartNative }) {
  report.scope = 'STEP10 real empty applicant WSS A, durable materials, distinct new RTC Member M';
  report.initial = await status(page);
  assert.equal(report.initial.stage, 'AwaitingApproval');
  assert.equal(report.initial.authorization_saved, false); assert.equal(report.initial.secret_saved, false);
  assert.equal(ready.ipAddresses, 0); assert.equal(ready.relayAddresses, 0);
  assert.deepEqual(ready.knownMembers, [ready.id]);
  await observeCommits(page);
  assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation)), 'AwaitingApproval');
  report.initialA = (await diagnostic(page)).request;
  assert.equal(report.initialA.alpn, 'miopunch/admission-request/1');
  assert.equal(report.initialA.remote_id, ready.id); assert.equal(report.initialA.restricted, true); assert.equal(report.initialA.business_handle, false);
  const pending = await native.command('admission-status', 'admission-status');
  const request = pending.requests.find(request => request.applicant === report.browserIdentity);
  assert(request); assert.equal(request.authorization, null);
  report.approval = await native.command(`approve-request ${request.applicant} ${request.request}`, 'approved');
  assert.equal(report.approval.after - report.approval.before, 1);
  assert.equal(await bounded(page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation)), 'PendingMaterials');
  report.materialA = (await diagnostic(page)).request;
  report.authorizationSaved = await status(page);
  assert.equal(report.authorizationSaved.authorization_saved, true); assert.equal(report.authorizationSaved.secret_saved, false);
  report.authorizationCommits = await page.evaluate(() => window.admissionCommits);
  assert(report.authorizationCommits.every(tx => tx.committed && !tx.aborted && tx.durability === 'strict'));
  await bounded(page.waitForTimeout(350));
  assert.equal(await bounded(page.evaluate(() => window.network.complete_admission())), 'Complete');
  report.localComplete = await status(page);
  report.materialReceipt = JSON.parse(await page.evaluate(() => window.network.admission_material_receipt()));
  report.materialEvents = (await diagnostic(page)).request.material_events;
  report.completionRemainingMs = (await diagnostic(page)).request.completion_remaining_ms;
  assert(report.completionRemainingMs < report.materialA.remaining_caller_ms - 250);
  assert.deepEqual(report.materialEvents.map(event => event.stage), ['AUTH-sent', 'secret-received-generation-verified', 'secret-committed', 'SAVE-sent', 'DONE-received']);
  report.secretCommits = await page.evaluate(() => window.admissionCommits);
  assert.equal(report.secretCommits.length, report.authorizationCommits.length + 1);
  assert.equal(report.secretCommits.at(-1).committed, true);
  report.senderReceipt = await native.command('admission-status', 'admission-status');
  const result = report.senderReceipt.requests.find(value => value.applicant === request.applicant && value.request === request.request);
  assert.equal(result.authorization, report.approval.authorization); assert.equal(result.receiptStage, 'Complete');
  report.signedGrant = JSON.parse(await page.evaluate(id => window.network.control_record(id), result.authorization));
  report.savedState = JSON.parse(await page.evaluate(() => window.network.control_state()));
  assert.equal(report.savedState.records, report.senderReceipt.records.length); assert.equal(report.savedState.pending, 0);
  assert.deepEqual(report.materialReceipt.records, report.senderReceipt.records);
  report.reopened = await reopen(page, {}, report.origin, report.browserIdentity, bounded);
  assert.equal(report.reopened.stage, 'Complete'); assert.equal(report.reopened.secret_saved, true);
  assert.deepEqual(JSON.parse(await page.evaluate(id => window.network.control_record(id), result.authorization)), report.signedGrant);
  report.newRtcM = await realBusiness({ page, native, ready, report, bounded, save });
  assert.notEqual(report.newRtcM.nativeRelease.alpn, report.materialA.alpn);
  report.finalSaved = await status(page); assert.equal(report.finalSaved.stage, 'Complete');
  await pendingAndIdentity({ context, native, ready, report, bounded, configuration }); save();
  native = await precodeResume({ context, native, ready, report, bounded, configuration, restartNative }); save();
  await completeThenRtcFailure({ page, native, ready, report, bounded, configuration }); save();
  report.finalNative = await native.command('status', 'status');
  assert.equal(report.finalNative.activeRtcAssociations, 0); assert.equal(report.finalNative.activeRtcPumps, 0);
  save();
}
