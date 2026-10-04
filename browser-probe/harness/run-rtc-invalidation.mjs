import assert from 'node:assert/strict';

// Capture the actual WASM-installed event handlers. The test keeps JS references
// only; it does not preserve or invent a Rust callback after its owner drops.
async function captureLateEvents(page) {
  await page.evaluate(() => {
    window.ownedLateRtc = [];
    const Original = window.RTCPeerConnection;
    const descriptor = (object, name) => {
      for (let prototype = Object.getPrototypeOf(object); prototype; prototype = Object.getPrototypeOf(prototype)) {
        const value = Object.getOwnPropertyDescriptor(prototype, name);
        if (value?.set && value?.get) return value;
      }
      throw new Error(`actual RTC event descriptor absent: ${name}`);
    };
    function capture(object, name, ticket) {
      const property = descriptor(object, name);
      Object.defineProperty(object, name, {
        configurable: true,
        get() { return property.get.call(this); },
        set(handler) {
          property.set.call(this, handler);
          if (handler && !ticket.handlers[name]) ticket.handlers[name] = handler;
        },
      });
    }
    window.RTCPeerConnection = class extends Original {
      constructor(...args) {
        super(...args);
        const ticket = { peer: this, channels: [], handlers: {}, sentPackets: 0, sentBytes: 0, description: null };
        this.ownedLateTicket = ticket;
        window.ownedLateRtc.push(ticket);
        capture(this, 'onicegatheringstatechange', ticket);
      }
      createDataChannel(...args) {
        const channel = super.createDataChannel(...args);
        const ticket = this.ownedLateTicket;
        ticket.channels.push(channel);
        capture(channel, 'onopen', ticket);
        const send = channel.send;
        channel.send = function(bytes) {
          const result = send.call(this, bytes);
          ticket.sentPackets++;
          ticket.sentBytes += bytes.byteLength ?? bytes.length ?? 0;
          return result;
        };
        return channel;
      }
      setRemoteDescription(description) {
        this.ownedLateTicket.description = { type: description.type, sdp: description.sdp };
        return super.setRemoteDescription(description);
      }
    };
  });
}

async function oldTicketSnapshot(page, index) {
  return page.evaluate(index => {
    const ticket = window.ownedLateRtc[index];
    return { peerClosed: ticket.peer.connectionState === 'closed', peerState: ticket.peer.connectionState, channelStates: ticket.channels.map(channel => channel.readyState),
      sentPackets: ticket.sentPackets, sentBytes: ticket.sentBytes,
      actualOpenHandlerCaptured: typeof ticket.handlers.onopen === 'function',
      actualGatherHandlerCaptured: typeof ticket.handlers.onicegatheringstatechange === 'function',
      fullDescriptionHadCandidates: Boolean(ticket.description?.sdp.includes('a=candidate:')),
      callbacksDetached: ticket.peer.onicegatheringstatechange === null && ticket.peer.oniceconnectionstatechange === null && ticket.peer.onconnectionstatechange === null &&
        ticket.channels.every(channel => channel.onopen === null && channel.onmessage === null && channel.onclose === null && channel.onerror === null && channel.onbufferedamountlow === null),
    };
  }, index);
}

async function replayOldEvents(page, index) {
  return page.evaluate(async index => {
    const ticket = window.ownedLateRtc[index];
    const invoke = (name, object, event) => {
      const handler = ticket.handlers[name];
      if (typeof handler !== 'function') throw new Error(`actual installed ${name} was not captured`);
      try { handler.call(object, new Event(event)); return { invoked: true, disposed: false }; }
      catch (error) { return { invoked: false, disposed: /closure|destroyed|recursive/i.test(String(error)), rejection: String(error) }; }
    };
    const before = { packets: ticket.sentPackets, bytes: ticket.sentBytes };
    const open = invoke('onopen', ticket.channels[0], 'open');
    const gathered = invoke('onicegatheringstatechange', ticket.peer, 'icegatheringstatechange');
    let description;
    if (!ticket.description) throw new Error('actual full remote SDP was not captured');
    try { await ticket.peer.setRemoteDescription(ticket.description); description = { applied: true }; }
    catch (error) { description = { applied: false, rejectionName: error.name, rejection: String(error) }; }
    return { scope: 'captured installed callbacks and actual closed-PC full SDP API; no trickle ICE',
      open, gathered, description, before, after: { packets: ticket.sentPackets, bytes: ticket.sentBytes },
      peerClosed: ticket.peer.connectionState === 'closed',
      channelClosed: ticket.channels.every(channel => channel.readyState === 'closed'),
    };
  }, index);
}

const diagnostics = async page => JSON.parse(await page.evaluate(() => window.network.rtc_invalidation_diagnostics()));

async function checkpoint(page, stage, bounded) {
  await bounded(page.waitForFunction(stage => {
    const state = JSON.parse(window.network.rtc_invalidation_diagnostics());
    return state.checkpoint?.held && state.checkpoint.stage === stage;
  }, stage));
  return diagnostics(page);
}

function assertRetired(value, localRoute) {
  assert.equal(value.activeAssociations, 0);
  assert.equal(value.activePumpTasks, 0);
  assert.equal(value.checkpoint.held, false);
  assert.equal(value.checkpoint.originalOpen, false);
  assert.equal(value.checkpoint.route.active, false);
  assert.equal(value.checkpoint.route.qualified, false);
  assert.equal(value.checkpoint.route.tx_bytes + value.checkpoint.route.rx_bytes, 0);
  assert(!value.checkpoint.publishedRoutes.includes(localRoute));
}

export async function runRtcInvalidation({ page, native, ready, report, bounded, save, mode }) {
  report.scope = 'STEP8; actual saved-member WSS Control RTC attempt invalidation; owned one-shot boundaries';
  report.invalidationMode = mode;
  report.invalidationStartedUtc = new Date().toISOString();
  report.totalBudgetSeconds = 60;
  report.productCallerBudgetMs = 12000;
  await captureLateEvents(page);
  if (mode !== 'cancel') return invalidateQualifiedControl({ page, native, ready, report, bounded, save, mode });
  await page.evaluate(() => window.network.rtc_invalidation_arm('channel-ready'));
  const cleanup = native.wait('rtc-path-cleanup', true); cleanup.catch(() => {});
  await page.evaluate(id => {
    window.ownedAttempt = window.network.member_connection_attempt(id, 12000);
    window.ownedPendingMember = window.ownedAttempt.connect().then(member => { window.ownedUnexpectedMember = member; return { returnedHandle: true }; }, error => ({ returnedHandle: false, failure: String(error) }));
  }, ready.id);
  report.cancelCheckpoint = await checkpoint(page, 'channel-ready', bounded);
  const old = report.cancelCheckpoint.checkpoint.metadata;
  report.oldActualTicket = await oldTicketSnapshot(page, 0);
  assert.equal(report.oldActualTicket.actualOpenHandlerCaptured, true);
  assert.equal(report.oldActualTicket.actualGatherHandlerCaptured, true);
  assert.equal(report.oldActualTicket.fullDescriptionHadCandidates, true);
  report.cancelled = await bounded(page.evaluate(async () => {
    const started = performance.now(); window.ownedAttempt.cancel();
    return { ...(await window.ownedPendingMember), observedMilliseconds: performance.now()-started };
  }));
  assert.equal(report.cancelled.returnedHandle, false);
  assert.match(report.cancelled.failure, /cancel/i);
  assert(report.cancelled.observedMilliseconds < 1000);
  report.nativeCancelledCleanup = await cleanup;
  assert.equal(report.nativeCancelledCleanup.switched, false);
  assert.equal(report.nativeCancelledCleanup.cleanup.pcClosed, true);
  assert.equal(report.nativeCancelledCleanup.cleanup.ownedPumpTasks, 0);
  await bounded(page.waitForFunction(() => {
    const value = JSON.parse(window.network.rtc_invalidation_diagnostics());
    return value.activeAssociations === 0 && value.activePumpTasks === 0;
  }));
  report.cancelledDiagnostics = await diagnostics(page);
  assertRetired(report.cancelledDiagnostics, old.localRoute);
  report.savedAfterCancellation = JSON.parse(await page.evaluate(() => window.network.status()));
  assert.equal(report.savedAfterCancellation.running, true);
  assert.equal(report.savedAfterCancellation.stage, 'Complete');
  assert.equal(report.savedAfterCancellation.id, report.browserIdentity);
  report.oldTicketAfterRetirement = await oldTicketSnapshot(page, 0);
  assert.equal(report.oldTicketAfterRetirement.peerClosed, true);
  assert.equal(report.oldTicketAfterRetirement.callbacksDetached, true);
  report.lateEvents = await replayOldEvents(page, 0);
  assert.equal(report.lateEvents.description.applied, false);
  assert.equal(report.lateEvents.peerClosed, true);
  assert.deepEqual(report.lateEvents.after, report.lateEvents.before);
  assertRetired(await diagnostics(page), old.localRoute);
  await page.evaluate(() => { window.network.rtc_invalidation_clear(); window.ownedAttempt.free(); });
  await bounded(page.evaluate(async id => { window.retryMember = await window.network.open_member_connection(id); }, ready.id));
  report.retry = JSON.parse(await page.evaluate(() => window.retryMember.rtc_path_diagnostics()));
  assert.notEqual(report.retry.memberStableId, old.stableId, 'new attempt is a distinct M, not old M resurrection');
  assert.notEqual(report.retry.bootstrap.route.original_stable_id, old.stableId);
  assert.equal(report.retry.bootstrap.freshAck, true);
  assert.equal(report.retry.bootstrap.native.freshAck, true);
  assert(report.retry.paths.some(path => path.isRtc && path.selected));
  assert(report.retry.paths.every(path => path.isRtc && !path.isRelay));
  assert(report.retry.paths.every(path => !path.address.includes(old.remoteRoute)));
  report.retryPublishedRoutes = (await diagnostics(page)).publishedRoutes;
  assert(report.retryPublishedRoutes.length === 1 && !report.retryPublishedRoutes.includes(old.localRoute));
  assert.equal(await bounded(page.evaluate(() => window.retryMember.exchange('new bounded RTC M after explicit cancel'))), 'new bounded RTC M after explicit cancel');
  const retryCleanup = native.wait('rtc-path-cleanup', true); retryCleanup.catch(() => {});
  await page.evaluate(() => window.retryMember.close());
  report.nativeRetryCleanup = await retryCleanup;
  assert.equal(report.nativeRetryCleanup.cleanup.pcClosed, true);
  report.finalNative = await native.command('status', 'status');
  assert.equal(report.finalNative.activeRtcAssociations, 0);
  assert.equal(report.finalNative.activeRtcPumps, 0);
  assert.deepEqual(report.finalNative.publishedRtcRoutes, []);
  report.finalBrowser = await diagnostics(page);
  assert.equal(report.finalBrowser.activeAssociations, 0);
  assert.equal(report.finalBrowser.activePumpTasks, 0);
  // The retired ticket remains distinct even after the new association worked.
  report.lateEventsAfterNewAttempt = await replayOldEvents(page, 0);
  assert.deepEqual(report.lateEventsAfterNewAttempt.after, report.lateEventsAfterNewAttempt.before);
  assert.equal(report.lateEventsAfterNewAttempt.description.applied, false);
  report.finalTickets = await page.evaluate(() => window.ownedLateRtc.map(ticket => ({ closed: ticket.peer.connectionState === 'closed', sentPackets: ticket.sentPackets, sentBytes: ticket.sentBytes })));
  assert(report.finalTickets.every(ticket => ticket.closed));
  save();
}

async function invalidateQualifiedControl({ page, native, ready, report, bounded, save, mode }) {
  if (!['member', 'secret', 'secret-standard', 'resource'].includes(mode)) throw new Error('unknown independent invalidation story');
  const credentialMode = mode === 'secret' || mode === 'secret-standard';
  report.beforeSource = await native.command('control-status', 'control-status');
  const holdCleanup = mode === 'secret';
  await native.command(holdCleanup ? 'arm-control-hold-cleanup' : 'arm-control', 'control-armed');
  const nativeCleanupRetired = holdCleanup ? native.wait('rtc-native-cleanup-retired', true) : null;
  nativeCleanupRetired?.catch(() => {});
  await page.evaluate(() => window.network.rtc_invalidation_arm('before-release'));
  const nativeCheckpoint = native.wait('rtc-control-checkpoint', true); nativeCheckpoint.catch(() => {});
  const cleanup = native.wait('rtc-path-cleanup', true); cleanup.catch(() => {});
  const withheld = native.wait('handle-withheld', true); withheld.catch(() => {});
  await page.evaluate(id => {
    window.ownedAttempt = window.network.member_connection_attempt(id, 12000);
    window.ownedReturnedHandle = false;
    window.ownedPendingMember = window.ownedAttempt.connect().then(member => { window.ownedReturnedHandle = true; window.ownedUnexpectedMember = member; return { returnedHandle: true }; }, error => ({ returnedHandle: false, failure: String(error) }));
  }, ready.id);
  report.beforeRelease = await checkpoint(page, 'before-release', bounded);
  report.nativeBeforeRelease = (await nativeCheckpoint).checkpoint;
  const browserMetadata = report.beforeRelease.checkpoint.metadata;
  const nativeMetadata = report.nativeBeforeRelease.metadata;
  assert.equal(browserMetadata.attempt, nativeMetadata.attempt);
  for (const metadata of [browserMetadata, nativeMetadata]) {
    assert.equal(metadata.freshAck, true); assert.equal(metadata.bothConfirmed, true);
    assert(metadata.probeWatermark > 0 && metadata.pathId);
    assert(metadata.freshAckMs >= metadata.freshProbeStartedMs);
  }
  assert.equal(await page.evaluate(() => window.ownedReturnedHandle), false);
  assert(!report.native.some(entry => entry.event === 'member-released'));
  report.nativeImmediatelyBeforeApply = await native.command('control-status', 'control-status');
  if (mode === 'member') {
    report.applied = await native.command(`remove-member ${report.browserIdentity}`, 'qualification-applied');
    assert.equal(report.applied.kind, 'verified-applied-member-removal');
    assert.equal(report.applied.effectiveAccessAbsent, true);
    assert(!report.applied.members.includes(report.browserIdentity));
    assert(report.applied.after.authorizationEpoch > report.applied.before.authorizationEpoch);
    assert(report.applied.after.revision > report.applied.before.revision);
    assert.equal(report.applied.after.secretGeneration, report.beforeSource.secretGeneration);
    assert.equal(report.applied.resourceRevision, report.beforeSource.resourceRevision);
    // The pending operation owns a separate immutable Attempt object. Therefore
    // real SDK receipt updates Network IDB/gate while its M is withheld.
    report.browserAppliedRecord = await bounded(page.evaluate(header => window.network.ingest_management_record(header, undefined), report.applied.header));
    assert.equal(report.browserAppliedRecord, report.applied.record);
    report.browserAppliedSource = await diagnostics(page);
    assert(report.browserAppliedSource.sourceRevision > report.beforeRelease.sourceRevision);
    assert.equal(report.browserAppliedSource.secretGeneration, report.beforeRelease.secretGeneration);
  } else if (credentialMode) {
    report.applied = await native.command('credential-invalidate', 'qualification-applied');
    assert.equal(report.applied.kind, 'credential-generation-invalidation');
    assert.equal(report.applied.generation, report.beforeSource.secretGeneration + 1);
    assert.equal(report.applied.durableCurrent, true);
    assert.equal(report.applied.after.secretGeneration, report.applied.generation);
    assert(report.applied.after.revision > report.applied.before.revision);
    assert.equal(report.applied.after.authorizationEpoch, report.applied.before.authorizationEpoch);
    assert.deepEqual([...report.applied.members].sort(), [...report.beforeSource.members].sort());
    assert.equal(report.applied.resourceRevision, report.beforeSource.resourceRevision);
    assert.equal(report.applied.resourceAllowed, report.beforeSource.resourceAllowed);
    report.credentialFaultScope = 'native durable current material install/store pin -> real gate source; not browser or network-wide rotation';
    if (!holdCleanup) report.credentialEvidenceScope = 'standard actual SDK credential invalidation; not delayed-notification or controlled-PC-open causal proof';
    report.browserAfterNativeCredentialApply = await diagnostics(page);
    assert.equal(report.browserAfterNativeCredentialApply.secretGeneration, report.beforeRelease.secretGeneration);
    assert.equal(report.browserAfterNativeCredentialApply.sourceRevision, report.beforeRelease.sourceRevision);
  } else {
    report.applied = await native.command('rtc-resource-disable', 'qualification-applied');
    assert.equal(report.applied.kind, 'scoped-rtc-carrier-permission');
    assert.equal(report.applied.previousResourceRevision, report.beforeSource.resourceRevision);
    assert.equal(report.applied.resourceRevision, report.beforeSource.resourceRevision + 1);
    assert.equal(report.applied.resourceAllowed, false);
    for (const field of ['revision', 'authorizationEpoch', 'secretGeneration', 'memberCount', 'removedCount']) {
      assert.equal(report.applied.after[field], report.applied.before[field], `resource disable keeps SDK ${field}`);
    }
    assert.equal(report.applied.checkpoint.route.active, false);
    assert.equal(report.applied.checkpoint.route.qualified, false);
    assert.equal(report.applied.checkpoint.route.tx_bytes + report.applied.checkpoint.route.rx_bytes, 0);
    assert(!report.applied.checkpoint.publishedRoutes.includes(nativeMetadata.localRoute));
    report.resourceFaultScope = 'actual installed native RtcTransport carrier/registry permission; MQTT signaling and durable SDK membership/materials unchanged';
    report.browserAfterNativeResourceDisable = await diagnostics(page);
    assert.equal(report.browserAfterNativeResourceDisable.resourceRevision, report.beforeRelease.resourceRevision);
    assert.equal(report.browserAfterNativeResourceDisable.resourceAllowed, true);
    assert.equal(report.browserAfterNativeResourceDisable.secretGeneration, report.beforeRelease.secretGeneration);
    assert.equal(report.browserAfterNativeResourceDisable.sourceRevision, report.beforeRelease.sourceRevision);
  }
  if (holdCleanup) {
    report.nativeCleanupRetired = await nativeCleanupRetired;
    assert.equal(report.nativeCleanupRetired.details.originalOpen, false);
    assert.equal(report.nativeCleanupRetired.details.route.active, false);
    assert.equal(report.nativeCleanupRetired.details.route.owned_pump_tasks, 0);
    assert.equal(report.nativeCleanupRetired.details.channel.readyState, 'Open');
    assert.equal(report.nativeCleanupRetired.details.sctpState, 'Connected');
    while (true) {
      report.nativeCleanupHeld = await native.command('control-status', 'control-status');
      if (report.nativeCleanupHeld.checkpoint.cleanupHeld) break;
      await bounded(new Promise(resolve => setTimeout(resolve, 5)));
    }
    assert.equal(report.nativeCleanupHeld.checkpoint.originalOpen, false);
    assert.equal(report.nativeCleanupHeld.activeRtcAssociations, 0);
    assert.equal(report.nativeCleanupHeld.activeRtcPumps, 0);
    report.browserWhileNativeCleanupHeld = await diagnostics(page);
    report.physicalChannelWhileNativeCleanupHeld = await oldTicketSnapshot(page, 0);
    assert.equal(report.browserWhileNativeCleanupHeld.checkpoint.originalOpen, true, 'fixture must reach still-live local M for final probe regression');
    assert.equal(report.physicalChannelWhileNativeCleanupHeld.peerState, 'connected');
    assert.deepEqual(report.physicalChannelWhileNativeCleanupHeld.channelStates, ['open']);
  }
  report.appliedAtUtc = new Date().toISOString();
  save();
  report.nativeReleaseRefused = await native.command(`control-release ${nativeMetadata.attempt}`, 'control-release');
  assert.equal(report.nativeReleaseRefused.released, false);
  report.browserRelease = await page.evaluate(attempt => {
    try { window.network.rtc_invalidation_release(attempt); return { released: true }; }
    catch (error) { return { released: false, rejection: String(error) }; }
  }, browserMetadata.attempt);
  if (holdCleanup) {
    assert.equal(report.browserRelease.released, true, 'cleanup timing must not substitute for the final probe guard');
    await bounded(page.waitForFunction(() => window.ownedReturnedHandle || JSON.parse(window.network.rtc_invalidation_diagnostics()).checkpoint?.releaseProbe));
    if (await page.evaluate(() => window.ownedReturnedHandle)) {
      report.pendingOutcome = await page.evaluate(() => window.ownedPendingMember);
      save();
      assert.equal(report.pendingOutcome.returnedHandle, false, 'stale private final cannot release business while the peer SDK M/IO is retired');
    }
    report.finalProbeAttempt = (await diagnostics(page)).checkpoint.releaseProbe;
    assert(report.finalProbeAttempt.watermark > browserMetadata.probeWatermark, 'new exact-path release watermark');
    assert.equal(report.finalProbeAttempt.pathId, browserMetadata.pathId);
    assert.equal(report.finalProbeAttempt.acknowledged, false);
    const beforePackets = report.browserWhileNativeCleanupHeld.checkpoint.dataChannel.actualSentPackets;
    await bounded(page.waitForFunction(beforePackets => window.ownedReturnedHandle || window.ownedLateRtc[0].sentPackets > beforePackets, beforePackets));
    assert.equal(await page.evaluate(() => window.ownedReturnedHandle), false);
    report.finalProbeWhilePeerRetired = await diagnostics(page);
    assert.equal(report.finalProbeWhilePeerRetired.checkpoint.releaseProbe.acknowledged, false);
    report.nativeWhileNewProbe = await native.command('control-status', 'control-status');
    assert.equal(report.nativeWhileNewProbe.checkpoint.originalOpen, false);
    assert.equal(report.nativeWhileNewProbe.checkpoint.route.active, false);
    assert.equal(report.nativeWhileNewProbe.checkpoint.cleanupHeld, true);
    report.validationTrafficScope = 'new QUIC path-validation PING on restricted original M; no application/business handle';
    report.cleanupReleased = await native.command(`cleanup-release ${nativeMetadata.attempt}`, 'cleanup-release');
  }
  report.pendingOutcome = await bounded(page.evaluate(() => window.ownedPendingMember));
  assert.equal(report.pendingOutcome.returnedHandle, false);
  report.nativeWithheld = await withheld;
  report.nativeCleanup = await cleanup;
  assert.equal(report.nativeCleanup.switched, false);
  assert.equal(report.nativeCleanup.cleanup.pcClosed, true);
  assert.equal(report.nativeCleanup.cleanup.ownedPumpTasks, 0);
  await bounded(page.waitForFunction(() => {
    const value = JSON.parse(window.network.rtc_invalidation_diagnostics());
    return value.activeAssociations === 0 && value.activePumpTasks === 0;
  }));
  report.afterRetirement = await diagnostics(page);
  assertRetired(report.afterRetirement, browserMetadata.localRoute);
  report.nativeAfterRetirement = await native.command('control-status', 'control-status');
  assert.equal(report.nativeAfterRetirement.checkpoint.originalOpen, false);
  assert.equal(report.nativeAfterRetirement.checkpoint.route.active, false);
  assert.equal(report.nativeAfterRetirement.activeRtcAssociations, 0);
  assert.equal(report.nativeAfterRetirement.activeRtcPumps, 0);
  if (mode === 'resource') {
    assert.equal(report.nativeAfterRetirement.resourceAllowed, false);
    assert.equal(report.nativeAfterRetirement.resourceRevision, report.applied.resourceRevision);
    assert.deepEqual([...report.nativeAfterRetirement.members].sort(), [...report.beforeSource.members].sort());
    assert.equal(report.nativeAfterRetirement.secretGeneration, report.beforeSource.secretGeneration);
    assert.equal(report.nativeAfterRetirement.durableCurrent, true);
  }
  assert.equal(report.nativeAfterRetirement.checkpoint.route.tx_bytes + report.nativeAfterRetirement.checkpoint.route.rx_bytes, 0);
  report.oldTicketAfterRetirement = await oldTicketSnapshot(page, 0);
  assert.equal(report.oldTicketAfterRetirement.callbacksDetached, true);
  assert.equal(report.oldTicketAfterRetirement.fullDescriptionHadCandidates, true);
  report.lateEvents = await replayOldEvents(page, 0);
  assert.equal(report.lateEvents.description.applied, false);
  assert.equal(report.lateEvents.peerClosed, true);
  assert.deepEqual(report.lateEvents.after, report.lateEvents.before);
  report.afterLateEvents = await diagnostics(page);
  assertRetired(report.afterLateEvents, browserMetadata.localRoute);
  for (const field of ['actualSentPackets', 'actualSentBytes']) assert.equal(report.afterLateEvents.checkpoint.dataChannel[field], report.afterRetirement.checkpoint.dataChannel[field]);
  report.nativeAfterLateEvents = await native.command('control-status', 'control-status');
  for (const field of ['actualSentPackets', 'actualSentBytes']) {
    assert.equal(report.nativeAfterLateEvents.checkpoint.dataChannel[field], report.nativeAfterRetirement.checkpoint.dataChannel[field]);
    assert.equal(report.nativeAfterRetirement.checkpoint.dataChannel[field], report.applied.checkpoint.dataChannel[field], 'physical send counter freezes after applied source retirement');
  }
  assert.equal(report.nativeAfterLateEvents.checkpoint.originalOpen, false);
  assert(!report.nativeAfterLateEvents.checkpoint.publishedRoutes.includes(nativeMetadata.localRoute));
  assert.equal(await page.evaluate(() => window.ownedReturnedHandle), false);
  assert(!report.native.some(entry => entry.event === 'member-released'));
  report.savedAfter = JSON.parse(await page.evaluate(() => window.network.status()));
  assert.equal(report.savedAfter.authorization_saved, true);
  assert.equal(report.savedAfter.secret_saved, true);
  assert.equal(report.savedAfter.running, true);
  await page.evaluate(() => { window.network.rtc_invalidation_clear(); window.ownedAttempt.free(); });
  await native.command('control-clear', 'control-cleared');
  save();
}
