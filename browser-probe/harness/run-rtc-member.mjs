import assert from 'node:assert/strict';

function continuity(value, sequence) {
  assert.equal(value.sequence, sequence);
  assert.equal(value.stableUnchanged, true);
  assert.equal(value.bindingUnchanged, true);
  assert.equal(value.native.bindingUnchanged, true);
}
function channel(value) {
  assert.equal(value.ordered, false);
  assert.equal(value.maxRetransmits, 0);
  assert.equal(value.maxPacketLifeTime, null);
  assert.equal(value.negotiated, true);
  assert.equal(value.id, 0);
}
const diagnostics = async (page, member) => JSON.parse(await page.evaluate(name => window[name].rtc_path_diagnostics(), member));

export async function pumpFailure({ page, native, ready, report, bounded, save }) {
  await bounded(page.evaluate(async id => { window.pumpMember = await window.network.open_member_connection(id); }, ready.id));
  report.pumpFailureBefore = await diagnostics(page, 'pumpMember');
  const ended = native.wait('rtc-pump-ended', true);
  const cleanup = native.wait('rtc-path-cleanup', true);
  ended.catch(() => {}); cleanup.catch(() => {});
  await page.evaluate(() => { window.pumpAttempt = window.pumpMember.rtc_native_pump_failure_probe(6000); });
  report.pumpEnd = await ended;
  save();
  assert.equal(report.pumpEnd.active, false, 'native pump terminal must immediately retire its exact route');
  assert.equal(report.pumpEnd.qualified, false);
  const started = Date.now();
  report.pumpAttempt = JSON.parse(await bounded(page.evaluate(() => window.pumpAttempt)));
  report.pumpTerminalReturnMilliseconds = Date.now()-started;
  assert(report.pumpTerminalReturnMilliseconds < 500);
  assert.equal(report.pumpAttempt.switched, false);
  assert.equal(report.pumpAttempt.relayStillPresent, true);
  report.pumpCleanup = await cleanup;
  assert.equal(report.pumpCleanup.switched, false);
  assert.match(report.pumpCleanup.failure, /injected native RTC pump failure/);
  report.pumpContinuation = JSON.parse(await bounded(page.evaluate(() => window.pumpMember.rtc_continuity_exchange(2n))));
  continuity(report.pumpContinuation, 2);
  assert.equal(await bounded(page.evaluate(() => window.pumpMember.exchange('original relay DATA after native pump failure'))), 'original relay DATA after native pump failure');
  report.pumpFailureAfter = await diagnostics(page, 'pumpMember');
  assert.equal(report.pumpFailureBefore.memberStableId, report.pumpFailureAfter.memberStableId);
  assert(report.pumpFailureAfter.paths.some(path => path.isRelay && path.selected));
  await page.evaluate(() => window.pumpMember.close());
  save();
}

export async function runMemberRtc({ page, native, ready, report, bounded, save }) {
  if (process.argv.includes('--pump-failure-only')) return pumpFailure({ page, native, ready, report, bounded, save });
  const originalCallerStarted = Date.now();
  // This second admitted M exists before the original M gains an RTC route.
  await bounded(page.evaluate(async id => { window.secondMember = await window.network.open_member_connection(id); }, ready.id));
  report.before = await diagnostics(page, 'member');
  report.secondBefore = await diagnostics(page, 'secondMember');
  assert.notEqual(report.before.memberStableId, report.secondBefore.memberStableId);
  assert(report.before.paths.some(path => path.isRelay && path.selected));
  report.rtc = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_relay_member_probe(false, 12000))));
  save();
  assert.equal(report.rtc.switched, true, JSON.stringify(report.rtc));
  await native.wait('rtc-path');
  for (const value of [report.rtc.browser, report.rtc.native]) {
    assert.equal(value.stableUnchanged, true);
    assert.equal(value.bindingUnchanged, true);
    assert.equal(value.freshAck, true);
    assert(value.freshAckMs >= value.freshProbeStartedMs);
    assert(value.pathId);
    assert(Number.isSafeInteger(value.probeWatermark));
    assert.deepEqual(value.beforeRetainFrames, { streamSent: 0, streamReceived: 0, datagramSent: 0, datagramReceived: 0 });
    assert(value.dataChannel.actualSentPackets > 0);
    assert(value.dataChannel.actualReceivedPackets > 0);
    assert(value.route.tx_peak_bytes <= 65536 && value.route.rx_peak_bytes <= 65536);
  }
  assert.equal(report.rtc.browser.quicRole, 'client');
  assert.equal(report.rtc.browser.rtcRole, 'offerer');
  assert.equal(report.rtc.native.quicRole, 'server');
  assert.equal(report.rtc.native.rtcRole, 'answerer');
  channel(report.rtc.browser.rtc.channel);
  channel(report.rtc.native.channel);
  assert(report.rtc.browser.rtc.maxMessageSize >= 2048);
  assert(report.rtc.native.maxMessageSize >= 2048);
  assert.notEqual(report.rtc.native.selectedPair.local.typ, 'relay');
  assert.notEqual(report.rtc.native.selectedPair.remote.typ, 'relay');
  report.rtc.continuity.forEach((value, index) => continuity(value, index));
  assert.equal(report.rtc.continuity.length, 3);
  assert.equal(new Set(report.rtc.continuity.map(value => value.streamId)).size, 1);
  report.after = await diagnostics(page, 'member');
  report.afterReturn = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_continuity_exchange(3n))));
  continuity(report.afterReturn, 3);
  assert.equal(report.afterReturn.streamId, report.rtc.continuity[0].streamId);
  assert.equal(await bounded(page.evaluate(() => window.member.exchange('ordinary DATA after RTC upgrade'))), 'ordinary DATA after RTC upgrade');
  report.afterBusiness = await diagnostics(page, 'member');
  const rtcBefore = report.after.paths.find(path => path.isRtc && path.selected);
  const rtcAfter = report.afterBusiness.paths.find(path => path.isRtc && path.selected);
  assert(rtcBefore && rtcAfter);
  assert(rtcAfter.streamFramesSent > rtcBefore.streamFramesSent);
  assert(rtcAfter.streamFramesReceived > rtcBefore.streamFramesReceived);
  assert(report.afterBusiness.paths.every(path => !path.isRelay));
  assert.equal(report.afterBusiness.memberStableId, report.before.memberStableId);
  assert.equal(await bounded(page.evaluate(() => window.secondMember.exchange('second M stays relay'))), 'second M stays relay');
  report.secondAfter = await diagnostics(page, 'secondMember');
  assert.equal(report.secondAfter.memberStableId, report.secondBefore.memberStableId);
  assert(report.secondAfter.paths.some(path => path.isRelay && path.selected));
  assert(report.secondAfter.paths.every(path => !path.isRtc));
  // A new M must also bootstrap over relay while the peer's preferred RTC
  // address remains live but belongs exclusively to the original M.
  await bounded(page.evaluate(async id => { window.newMember = await window.network.open_member_connection(id); }, ready.id));
  assert.equal(await bounded(page.evaluate(() => window.newMember.exchange('new M must bootstrap over relay'))), 'new M must bootstrap over relay');
  report.newAfterRtc = await diagnostics(page, 'newMember');
  assert(report.newAfterRtc.paths.some(path => path.isRelay && path.selected));
  assert(report.newAfterRtc.paths.every(path => !path.isRtc));
  await page.evaluate(() => window.newMember.close());
  await bounded(new Promise(resolve => setTimeout(resolve, Math.max(0, 12500-(Date.now()-originalCallerStarted)))));
  report.afterOriginalDeadline = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_continuity_exchange(4n))));
  continuity(report.afterOriginalDeadline, 4);
  assert.equal(report.afterOriginalDeadline.streamId, report.rtc.continuity[0].streamId);
  report.retainedLifetimeMilliseconds = Date.now()-originalCallerStarted;
  assert(report.retainedLifetimeMilliseconds > 12000);
  report.afterOriginalDeadlinePaths = await diagnostics(page, 'member');
  assert(report.afterOriginalDeadlinePaths.paths.some(path => path.isRtc && path.selected));
  const normalCleanup = native.wait('rtc-path-cleanup', true);
  await page.evaluate(() => window.member.close());
  report.normalCleanup = await normalCleanup;
  report.normalBrowserCleanup = await diagnostics(page, 'member');
  assert.equal(report.normalBrowserCleanup.activePumpTasks, 0);
  assert.equal(report.normalBrowserCleanup.activeAssociations, 0);
  assert.equal(report.normalCleanup.cleanup.pcClosed, true);
  assert.equal(report.normalCleanup.cleanup.ownedPumpTasks, 0);
  assert.equal(report.normalCleanup.cleanup.txBytes + report.normalCleanup.cleanup.rxBytes, 0);
  save();

  await bounded(page.evaluate(async id => { window.failedMember = await window.network.open_member_connection(id); }, ready.id));
  report.failureBefore = await diagnostics(page, 'failedMember');
  const failureCleanup = native.wait('rtc-path-cleanup', true);
  failureCleanup.catch(() => {});
  const probeStarted = native.wait('rtc-probe-started', true);
  probeStarted.catch(() => {});
  await page.evaluate(() => { window.failedAttempt = window.failedMember.rtc_relay_member_probe(true, 10000); });
  report.lossPhase = await probeStarted;
  // A separate ordinary business caller must progress while RTC cannot get ACKs.
  const businessStarted = Date.now();
  assert.equal(await bounded(page.evaluate(() => window.failedMember.exchange('business while RTC probes are dropped'))), 'business while RTC probes are dropped');
  report.concurrentBusinessMilliseconds = Date.now()-businessStarted;
  report.failedRtc = JSON.parse(await bounded(page.evaluate(() => window.failedAttempt)));
  report.failureCleanup = await failureCleanup;
  save();
  assert.equal(report.failedRtc.switched, false);
  assert.equal(report.failedRtc.freshAck, false);
  assert.equal(report.failedRtc.relayStillPresent, true);
  assert.equal(report.failedRtc.ownedPumpTasks, 0);
  assert.equal(report.failedRtc.pcClosed, true);
  assert.equal(report.failedRtc.callbacksDetached, true);
  assert(report.failedRtc.pathId, 'loss must occur after actual RTC QUIC path establishment');
  assert(report.failedRtc.dataChannel.droppedQuicPackets > 0);
  assert(report.failedRtc.attemptMs <= report.failedRtc.callerBudgetMs + 250);
  assert.equal(report.failureCleanup.cleanup.pcClosed, true);
  assert.equal(report.failureCleanup.cleanup.ownedPumpTasks, 0);
  assert.equal(report.failureCleanup.cleanup.txBytes + report.failureCleanup.cleanup.rxBytes, 0);
  report.afterFailure = JSON.parse(await bounded(page.evaluate(() => window.failedMember.rtc_continuity_exchange(2n))));
  continuity(report.afterFailure, 2);
  assert.equal(report.afterFailure.streamId, report.failedRtc.continuity[0].streamId);
  assert.equal(await bounded(page.evaluate(() => window.failedMember.exchange('relay DATA after RTC failure'))), 'relay DATA after RTC failure');
  report.failureAfter = await diagnostics(page, 'failedMember');
  assert.equal(report.failureAfter.memberStableId, report.failureBefore.memberStableId);
  assert(report.failureAfter.paths.some(path => path.isRelay && path.selected));
  // A maintenance caller supplies two seconds, never a renewed phase budget.
  const shortCleanup = native.wait('rtc-path-cleanup', true);
  shortCleanup.catch(() => {});
  report.shortDeadline = JSON.parse(await bounded(page.evaluate(() => window.secondMember.rtc_relay_member_probe(true, 2000))));
  assert.equal(report.shortDeadline.switched, false);
  assert.equal(report.shortDeadline.freshAck, false);
  assert.equal(report.shortDeadline.callerBudgetMs, 2000);
  assert(report.shortDeadline.attemptMs <= 2250);
  if (report.shortDeadline.continuity.length > 1) report.shortNativeCleanup = await shortCleanup;
  report.shortContinuation = JSON.parse(await bounded(page.evaluate(() => window.secondMember.rtc_continuity_exchange(2n))));
  continuity(report.shortContinuation, 2);
  assert.equal(report.shortContinuation.streamId, report.shortDeadline.continuity[0].streamId);
  await pumpFailure({ page, native, ready, report, bounded, save });
  report.finalPumpDiagnostics = await diagnostics(page, 'pumpMember');
  assert.equal(report.finalPumpDiagnostics.activePumpTasks, 0);
  assert.equal(report.finalPumpDiagnostics.activeAssociations, 0);
  report.browserCleanup = await page.evaluate(() => ({
    peersCreated: window.rtcObjects.length,
    peersClosed: window.rtcObjects.filter(peer => peer.connectionState === 'closed').length,
    callbacksDetached: window.rtcObjects.every(peer => peer.onicegatheringstatechange === null && peer.oniceconnectionstatechange === null && peer.onconnectionstatechange === null),
  }));
  assert.equal(report.browserCleanup.peersCreated, report.browserCleanup.peersClosed);
  assert.equal(report.browserCleanup.callbacksDetached, true);
  save();
}
