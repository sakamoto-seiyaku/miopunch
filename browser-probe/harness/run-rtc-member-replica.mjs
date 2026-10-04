import assert from 'node:assert/strict';

const body = 'original-author browser replica body';

async function observeTraffic(page) {
  await page.evaluate(() => {
    const Original = window.RTCPeerConnection;
    window.replicaRtcTraffic = [];
    window.RTCPeerConnection = class extends Original {
      constructor(...args) {
        super(...args);
        this.trafficTicket = { peer: this, channels: [], sentPackets: 0, sentBytes: 0, receivedPackets: 0, receivedBytes: 0 };
        window.replicaRtcTraffic.push(this.trafficTicket);
      }
      createDataChannel(...args) {
        const channel = super.createDataChannel(...args);
        const ticket = this.trafficTicket;
        const send = channel.send;
        channel.send = function(bytes) {
          const result = send.call(this, bytes);
          ticket.sentPackets++; ticket.sentBytes += bytes.byteLength ?? bytes.length;
          return result;
        };
        const receive = event => { ticket.receivedPackets++; ticket.receivedBytes += event.data.byteLength ?? event.data.size; };
        channel.addEventListener('message', receive);
        ticket.channels.push({ channel, receive, send });
        return channel;
      }
    };
  });
}

async function traffic(page, detach = false) {
  return page.evaluate(detach => window.replicaRtcTraffic.map(ticket => {
    if (detach) for (const entry of ticket.channels) {
      entry.channel.removeEventListener('message', entry.receive);
      entry.channel.send = entry.send;
    }
    return { scope: 'successful actual DataChannel.send submissions and delivered message events',
      sentPackets: ticket.sentPackets, sentBytes: ticket.sentBytes, receivedPackets: ticket.receivedPackets, receivedBytes: ticket.receivedBytes,
      closed: ticket.peer.connectionState === 'closed',
      callbacksDetached: ticket.peer.onicegatheringstatechange === null && ticket.peer.oniceconnectionstatechange === null && ticket.peer.onconnectionstatechange === null &&
        ticket.channels.every(({ channel }) => channel.onopen === null && channel.onmessage === null && channel.onclose === null && channel.onerror === null && channel.onbufferedamountlow === null),
      observersDetached: detach,
    };
  }), detach);
}

function exactRtc(paths) {
  assert(paths.length > 0);
  assert(paths.every(path => path.isRtc && !path.isRelay && !path.isIp && !path.isSignal));
  assert(paths.some(path => path.selected));
}

function releaseProof(bootstrap, nativeFinal) {
  for (const endpoint of [bootstrap, nativeFinal]) {
    assert.equal(endpoint.freshAck, true);
    assert.equal(endpoint.stableUnchanged, true);
    assert.equal(endpoint.bindingUnchanged, true);
    assert(Object.values(endpoint.beforeRetainFrames).every(value => value === 0));
    assert.equal(endpoint.finalReleaseFreshAck, true);
    assert(endpoint.finalReleaseProbeWatermark > endpoint.probeWatermark);
    assert(endpoint.finalReleaseAckMs >= endpoint.finalReleaseProbeStartedMs);
  }
  assert.equal(bootstrap.capabilitiesVersion, 3);
  assert.equal(bootstrap.rtcRole, 'offerer'); assert.equal(nativeFinal.rtcRole, 'answerer');
  // The event is emitted after native's new final-release probe; the earlier
  // native object in private FINAL is intentionally not substituted here.
  assert.equal(nativeFinal.memberStableId, bootstrap.native.memberStableId);
}

function streamProgress(before, after) {
  exactRtc(before); exactRtc(after);
  const initial = before.find(path => path.selected);
  const current = after.find(path => path.pathId === initial.pathId);
  assert(current?.selected);
  for (const field of ['sentBytes', 'receivedBytes', 'streamFramesSent', 'streamFramesReceived']) assert(current[field] > initial[field], `actual RTC ${field} grows`);
}

function cleanupProof(value) {
  assert.equal(value.cleanup.pcClosed, true);
  assert.equal(value.cleanup.ownedPumpTasks, 0);
  assert.equal(value.cleanup.txBytes + value.cleanup.rxBytes, 0);
}

async function absent(page, id) {
  await assert.rejects(() => page.evaluate(id => window.network.control_record(id), id));
}

function canonicalState(state) {
  return { ...state, members: [...state.members].sort((left, right) => left.id.localeCompare(right.id)) };
}

async function reopen(page, report, bounded, configuration) {
  await bounded(page.evaluate(async () => { await window.network.close(); window.network.free(); }));
  await bounded(page.goto(`${report.origin}/network.html?config=${encodeURIComponent(JSON.stringify(configuration))}`));
  assert.equal(await bounded(page.evaluate(() => window.ready)), report.browserIdentity);
  const status = JSON.parse(await page.evaluate(() => window.network.status()));
  assert.equal(status.stage, 'Complete');
  assert.equal(status.authorization_saved, true); assert.equal(status.secret_saved, true);
  return status;
}

export async function runRtcMemberReplica({ page, native, ready, report, bounded, save, configuration }) {
  report.scope = 'step 9; real member PING/PONG and independently qualified nonempty original-record LogSync';
  await observeTraffic(page);
  report.sourceRecord = await native.command('record', 'record');
  assert.equal(report.sourceRecord.applied, true); assert.equal(report.sourceRecord.author, ready.id);
  assert.equal(report.sourceRecord.action, 'Add');
  assert.notEqual(report.sourceRecord.target, ready.id); assert.notEqual(report.sourceRecord.target, report.browserIdentity);
  assert.equal(report.sourceRecord.body, Buffer.from(body).toString('hex'));
  await absent(page, report.sourceRecord.record);
  report.targetAbsentBeforeSync = true;
  const businessRelease = native.wait('member-released', true); businessRelease.catch(() => {});
  await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
  report.businessBefore = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
  report.businessNativeReleased = await businessRelease;
  report.businessNativeFinal = report.native.find(entry => entry.event === 'rtc-path' && entry.report.memberStableId === report.businessNativeReleased.stableId)?.report;
  releaseProof(report.businessBefore.bootstrap, report.businessNativeFinal);
  report.carrierBeforeBusiness = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
  report.businessPhysicalBefore = (await traffic(page))[0];
  const businessHandler = native.wait('business', true); businessHandler.catch(() => {});
  report.businessReply = await bounded(page.evaluate(() => window.member.exchange('PING')));
  save();
  assert.equal(report.businessReply, 'PONG', 'the member handler must respond PONG, not echo PING');
  report.businessAfter = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
  report.businessPhysicalAfter = (await traffic(page))[0];
  report.businessHandler = await businessHandler;
  assert.equal(report.businessHandler.stableId, report.businessNativeReleased.stableId);
  assert.equal(report.businessHandler.memberMessage, true);
  assert.equal(report.businessHandler?.reply, 'PONG'); assert.equal(report.businessHandler.bytes, 4);
  assert.equal(report.businessNativeReleased.alpn, 'miopunch/browser-member/rtc-path/3');
  assert.equal(report.businessAfter.memberStableId, report.businessBefore.memberStableId);
  streamProgress(report.businessBefore.paths, report.businessAfter.paths);
  report.carrierAfterBusiness = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
  for (const field of ['sent_bytes', 'received_bytes']) assert.equal(report.carrierAfterBusiness[0].traffic[field], report.carrierBeforeBusiness[0].traffic[field]);
  assert(report.businessPhysicalAfter.sentBytes > report.businessPhysicalBefore.sentBytes);
  assert(report.businessPhysicalAfter.receivedBytes > report.businessPhysicalBefore.receivedBytes);
  const businessCleanup = native.wait('rtc-path-cleanup', true); businessCleanup.catch(() => {});
  await page.evaluate(() => window.member.close());
  report.businessCleanup = await businessCleanup; cleanupProof(report.businessCleanup);

  const replicaReleased = native.wait('member-released', true); replicaReleased.catch(() => {});
  const replicaCleanup = native.wait('rtc-path-cleanup', true); replicaCleanup.catch(() => {});
  const replicaFinished = native.wait('replica-finished', true); replicaFinished.catch(() => {});
  try {
    report.replica = JSON.parse(await bounded(page.evaluate(id => window.network.sync_replica(id), ready.id)));
  } finally {
    report.replicaSession = JSON.parse(await page.evaluate(() => window.network.replica_diagnostics()));
    report.replicaPaths = JSON.parse(await page.evaluate(() => window.network.path_diagnostics()));
    report.afterSyncState = JSON.parse(await page.evaluate(() => window.network.control_state()));
    try { report.receivedRecord = JSON.parse(await page.evaluate(id => window.network.control_record(id), report.sourceRecord.record)); } catch {}
    save();
  }
  report.replicaReleased = await replicaReleased;
  report.replicaFinished = await replicaFinished;
  report.replicaCleanup = await replicaCleanup; cleanupProof(report.replicaCleanup);
  assert.equal(report.replica.complete, true); assert.equal(report.replica.source, ready.id);
  assert(report.replica.scope.includes(report.sourceRecord.record));
  assert.deepEqual(report.replica.saved, report.replica.scope); assert.deepEqual(report.replica.applied, report.replica.scope);
  assert.deepEqual(report.receivedRecord, { header: report.sourceRecord.header, body: report.sourceRecord.body });
  const session = report.replicaSession;
  report.replicaNativeFinal = report.native.find(entry => entry.event === 'rtc-path' && entry.report.memberStableId === report.replicaReleased.stableId)?.report;
  releaseProof(session.before.bootstrap, report.replicaNativeFinal);
  assert.equal(session.before.alpn, 'miopunch/control-replica/rtc-path/3');
  assert.notEqual(session.before.stableId, report.businessBefore.memberStableId);
  assert.notEqual(session.before.bootstrap.route.original_stable_id, report.businessBefore.bootstrap.route.original_stable_id);
  assert.notEqual(report.replicaReleased.stableId, report.businessNativeReleased.stableId);
  assert.equal(session.stableUnchanged, true); assert.equal(session.bindingUnchanged, true);
  assert.equal(session.before.currentPermission, true); assert.equal(session.after.currentPermission, true);
  assert.equal(session.before.closed, false); assert.equal(session.after.closed, false);
  assert(session.after.sourceRevision > session.before.sourceRevision, 'the real saved snapshot revision advances');
  assert.equal(session.after.permissionRevision, session.before.permissionRevision, 'this M remains effectively permitted');
  assert(session.operationMs < session.callerBudgetMs && session.remainingCallerMs > 0);
  streamProgress(session.before.paths, session.after.paths);
  assert.equal(report.replicaFinished.durableReceiptAccepted, true);
  assert.equal(report.replicaFinished.stableUnchanged, true); assert.equal(report.replicaFinished.bindingUnchanged, true);
  streamProgress(report.replicaFinished.before, report.replicaFinished.after);
  report.normalPhysical = await traffic(page, true);
  await bounded(page.waitForFunction(() => window.replicaRtcTraffic.every(ticket => ticket.peer.connectionState === 'closed')));
  report.normalClosedPhysical = await traffic(page, true);
  assert.equal(report.normalClosedPhysical.length, 2);
  assert(report.normalClosedPhysical.every(ticket => ticket.closed && ticket.callbacksDetached && ticket.observersDetached));
  report.savedAfterRecordReopen = await reopen(page, report, bounded, configuration);
  report.reopenedRecord = JSON.parse(await page.evaluate(id => window.network.control_record(id), report.sourceRecord.record));
  assert.deepEqual(report.reopenedRecord, report.receivedRecord);
  assert.equal(JSON.parse(await page.evaluate(() => window.network.control_state())).pending, 0);
  save();

  // Abort only the newly verified target's actual transaction, before its put.
  // Baseline duplicates and test-side completion flags cannot trigger this.
  await observeTraffic(page);
  report.abortSourceRecord = await native.command('record', 'record');
  await absent(page, report.abortSourceRecord.record);
  report.abortTargetAbsentBeforeSync = true;
  report.beforeAbortState = JSON.parse(await page.evaluate(() => window.network.control_state()));
  await page.evaluate(source => {
    const header = Array.from(source.header.match(/../g), value => parseInt(value, 16));
    const body = Array.from(source.body.match(/../g), value => parseInt(value, 16));
    const equal = (left, right) => left?.length === right.length && left.every((value, index) => value === right[index]);
    const put = IDBObjectStore.prototype.put;
    window.replicaAbort = { triggered: false, target: source.record };
    window.restoreReplicaPut = () => { IDBObjectStore.prototype.put = put; };
    IDBObjectStore.prototype.put = function(value, key) {
      if (!window.replicaAbort.triggered && this.name === 'node' && key === 'state' && typeof value === 'string') {
        const next = JSON.parse(value);
        const target = next.records.find(([candidate]) => equal(candidate, header));
        if (target) {
          window.replicaAbort = { triggered: true, target: source.record, headerMatched: true, bodyMatched: equal(target[1], body),
            transactionMode: this.transaction.mode, boundary: 'real IndexedDB readwrite transaction, before target put/commit' };
          this.transaction.abort();
          return undefined;
        }
      }
      return put.call(this, value, key);
    };
  }, report.abortSourceRecord);
  const abortReleased = native.wait('member-released', true); abortReleased.catch(() => {});
  const abortCleanup = native.wait('rtc-path-cleanup', true); abortCleanup.catch(() => {});
  const abortFinished = native.wait('replica-finished', true); abortFinished.catch(() => {});
  try {
    report.abortOutcome = await bounded(page.evaluate(async id => {
      try { return { resolved: true, outcome: JSON.parse(await window.network.sync_replica(id)) }; }
      catch (error) { return { resolved: false, failure: String(error) }; }
    }, ready.id));
  } finally {
    report.abortTransaction = await page.evaluate(() => { window.restoreReplicaPut(); return window.replicaAbort; });
  }
  assert.equal(report.abortTransaction.triggered, true); assert.equal(report.abortTransaction.headerMatched, true); assert.equal(report.abortTransaction.bodyMatched, true);
  assert.equal(report.abortTransaction.transactionMode, 'readwrite');
  assert.equal(report.abortOutcome.resolved, false); assert.match(report.abortOutcome.failure, /commit failed|storage changed/);
  report.abortSession = JSON.parse(await page.evaluate(() => window.network.replica_diagnostics()));
  assert.equal(report.abortSession.completeReturned, false);
  report.abortReleased = await abortReleased; report.abortFinished = await abortFinished;
  report.abortCleanup = await abortCleanup; cleanupProof(report.abortCleanup);
  report.abortNativeFinal = report.native.find(entry => entry.event === 'rtc-path' && entry.report.memberStableId === report.abortReleased.stableId)?.report;
  releaseProof(report.abortSession.before.bootstrap, report.abortNativeFinal);
  assert.equal(report.abortFinished.durableReceiptAccepted, false);
  report.afterAbortState = JSON.parse(await page.evaluate(() => window.network.control_state()));
  assert.deepEqual(canonicalState(report.afterAbortState), canonicalState(report.beforeAbortState));
  await absent(page, report.abortSourceRecord.record);
  await bounded(page.waitForFunction(() => window.replicaRtcTraffic.every(ticket => ticket.peer.connectionState === 'closed')));
  report.abortClosedPhysical = await traffic(page, true);
  assert.equal(report.abortClosedPhysical.length, 1);
  assert(report.abortClosedPhysical.every(ticket => ticket.closed && ticket.callbacksDetached && ticket.observersDetached));
  report.savedAfterAbortReopen = await reopen(page, report, bounded, configuration);
  await absent(page, report.abortSourceRecord.record);
  report.abortTargetAbsentAfterReopen = true;
  report.baselineRecordAfterAbortReopen = JSON.parse(await page.evaluate(id => window.network.control_record(id), report.sourceRecord.record));
  assert.deepEqual(report.baselineRecordAfterAbortReopen, report.receivedRecord);
  assert.deepEqual(canonicalState(JSON.parse(await page.evaluate(() => window.network.control_state()))), canonicalState(report.beforeAbortState));
  report.finalBrowser = JSON.parse(await page.evaluate(() => window.network.rtc_invalidation_diagnostics()));
  assert.equal(report.finalBrowser.activeAssociations, 0); assert.equal(report.finalBrowser.activePumpTasks, 0);
  report.nativeFirstQuiescence = await native.command('status', 'status');
  let status = report.nativeFirstQuiescence;
  while (status.socketCount !== ready.socketCount || status.activeRtcAssociations !== 0 || status.activeRtcPumps !== 0) {
    await bounded(new Promise(resolve => setTimeout(resolve, 10)));
    status = await native.command('status', 'status');
  }
  report.nativeFinalQuiescence = status;
  assert.deepEqual(status.publishedRtcRoutes, []);
  save();
}
