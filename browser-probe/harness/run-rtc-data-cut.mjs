import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { readFileSync, readlinkSync, writeFileSync } from 'node:fs';
import { isIP } from 'node:net';
import { join } from 'node:path';

const run = (program, args, input) => execFileSync('rtk', ['proxy', program, ...args], { encoding: 'utf8', input, timeout: 1500 });
const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
function counters(value) {
  return value.nftables.flatMap(entry => entry.rule?.expr ?? []).filter(entry => entry.counter)
    .reduce((sum, entry) => ({ packets: sum.packets + entry.counter.packets, bytes: sum.bytes + entry.counter.bytes }), { packets: 0, bytes: 0 });
}
function continuity(value, sequence, original) {
  assert.equal(value.sequence, sequence);
  assert.equal(value.stableUnchanged, true);
  assert.equal(value.bindingUnchanged, true);
  assert.equal(value.native.bindingUnchanged, true);
  if (original) {
    assert.equal(value.streamId, original.streamId);
    assert.equal(value.localStableId, original.localStableId);
    assert.equal(value.native.stableId, original.native.stableId);
  }
}
function captureProof(path, directions) {
  const bytes = readFileSync(path);
  assert(bytes.length > 24, 'capture must contain actual packets, not only the pcap header');
  assert.equal(bytes.readUInt32LE(0), 0xa1b2c3d4);
  assert.equal(bytes.readUInt32LE(16), 64);
  assert.equal(bytes.readUInt32LE(20), 276, 'owned any-interface capture must use Linux SLL2');
  const tuples = [];
  for (let offset = 24; offset < bytes.length;) {
    assert(offset+16 <= bytes.length);
    const length = bytes.readUInt32LE(offset+8); offset += 16;
    assert(length <= 64 && length >= 48 && offset+length <= bytes.length);
    const packet = bytes.subarray(offset, offset+length); offset += length;
    assert.equal(packet.readUInt16BE(0), 0x0800);
    assert.equal(packet[20] >> 4, 4); assert.equal(packet[20] & 15, 5);
    assert.equal(packet[29], 17);
    const tuple = { source: { address: [...packet.subarray(32, 36)].join('.'), port: packet.readUInt16BE(40) },
      destination: { address: [...packet.subarray(36, 40)].join('.'), port: packet.readUInt16BE(42) } };
    assert(directions.some(([from, to]) => from.address === tuple.source.address && from.port === tuple.source.port
      && to.address === tuple.destination.address && to.port === tuple.destination.port));
    tuples.push(tuple);
  }
  assert(tuples.length > 0);
  return { packets: tuples.length, snaplen: 64, linkType: 'LINUX_SLL2', allMatchSelectedUdpTuple: true, tuples };
}
async function livePair(page) {
  return page.evaluate(async () => {
    const pc = window.rtcObjects.find(peer => peer.connectionState === 'connected');
    if (!pc) throw new Error('live RTC peer absent');
    const rows = [...(await pc.getStats()).values()];
    const transport = rows.find(row => row.type === 'transport');
    const pair = rows.find(row => row.id === transport.selectedCandidatePairId);
    return { transport, pair, local: rows.find(row => row.id === pair.localCandidateId), remote: rows.find(row => row.id === pair.remoteCandidateId) };
  });
}

export async function runRtcDataCut({ page, native, ready, report, bounded, save, evidence, relayBackup }) {
  assert.equal(process.env.MIO_PRIVATE_NSS_NAMESPACE, '1', 'cut runs only inside the owned namespace runner');
  assert.match(process.env.MIO_OWNED_CUT_ID ?? '', /^mio-rtc-s7-[a-z0-9-]+$/);
  report.scope = 'STEP7; real kernel selected RTC UDP tuple cut, one owned namespace; not NAT/WAN qualification';
  report.mode = relayBackup ? 'permitted-relay' : 'no-data-fallback';
  report.namespace = readlinkSync('/proc/self/ns/net');
  await bounded(page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, ready.id));
  report.member = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
  assert(report.member.paths.some(path => path.isRtc && path.selected));
  assert(report.member.paths.every(path => path.isRtc && !path.isRelay));
  report.capabilities = await native.wait('path-capabilities');
  assert(report.capabilities.initialPaths.every(path => path.isMqtt && !path.isRelay && !path.isIp));
  for (const side of [report.member.bootstrap, report.member.bootstrap.native]) {
    assert.equal(side.stableUnchanged, true); assert.equal(side.bindingUnchanged, true);
    assert.equal(side.freshAck, true);
    assert(Object.values(side.beforeRetainFrames).every(value => value === 0));
  }
  if (relayBackup) {
    report.dormantRelayBeforeRelease = await page.evaluate(() => window.ownedRelayEvents);
    assert(report.dormantRelayBeforeRelease.every(event => event.kind === 'deferred'));
  }
  report.pairBefore = await livePair(page);
  const { local, remote } = report.pairBefore;
  assert.equal(local.protocol, 'udp'); assert.equal(remote.protocol, 'udp');
  assert.notEqual(local.candidateType, 'relay'); assert.notEqual(remote.candidateType, 'relay');
  for (const candidate of [local, remote]) {
    assert.equal(isIP(candidate.address), 4, 'first controlled tuple profile requires observed IPv4');
    assert(Number.isInteger(candidate.port) && candidate.port > 0 && candidate.port <= 65535);
  }
  const nativePair = report.member.bootstrap.native.selectedPair;
  assert.equal(nativePair.local.address, remote.address); assert.equal(nativePair.local.port, remote.port);
  assert.equal(nativePair.remote.address, local.address); assert.equal(nativePair.remote.port, local.port);
  report.socketsBefore = run('ss', ['-uapn']);
  assert(report.socketsBefore.includes(String(local.port)) && report.socketsBefore.includes(String(remote.port)));
  const table = `mio_cut_${Date.now()}`;
  const directions = [[local, remote], [remote, local]];
  const rules = `table ip ${table} {\n${['output', 'input'].map(hook => `chain ${hook} { type filter hook ${hook} priority -10; policy accept;\n${directions.map(([from, to]) => `ip saddr ${from.address} ip daddr ${to.address} udp sport ${from.port} udp dport ${to.port} counter drop`).join('\n')}\n}`).join('\n')}\n}\n`;
  const filter = `udp and ((src host ${local.address} and src port ${local.port} and dst host ${remote.address} and dst port ${remote.port}) or (src host ${remote.address} and src port ${remote.port} and dst host ${local.address} and dst port ${local.port}))`;
  report.cut = { table, rules, filter, tuple: { local: { address: local.address, port: local.port }, remote: { address: remote.address, port: remote.port }, protocol: 'udp' } };
  let capture;
  let installed = false;
  let captureLog = '';
  try {
    // SLL/SLL2 + IPv4/UDP headers and at most the STUN header, not ICE
    // credential attributes or business payload, are sufficient tuple proof.
    capture = spawn('rtk', ['proxy', 'tcpdump', '--immediate-mode', '-U', '-n', '-i', 'any', '-s', '64', '-c', '300', '-w', join(evidence, 'rtc-selected-tuple.pcap'), filter], { stdio: ['ignore', 'ignore', 'pipe'] });
    const captureExit = new Promise(resolve => capture.once('exit', (code, signal) => resolve({ code, signal })));
    capture.stderr.on('data', bytes => { captureLog += String(bytes).slice(0, 4096); });
    await bounded(new Promise((resolve, reject) => {
      capture.stderr.on('data', bytes => { if (String(bytes).includes('listening on')) resolve(); });
      capture.once('error', reject); capture.once('exit', code => reject(new Error(`capture stopped before ready: ${code}`)));
    }));
    report.streamBeforeCut = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_continuity_start())));
    continuity(report.streamBeforeCut, 0);
    report.beforeCut = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    report.carriersBeforeCut = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
    run('nft', ['-f', '-'], rules); installed = true;
    report.cut.startedUtc = new Date().toISOString();
    await page.evaluate(() => {
      window.continuitySettled = false;
      window.pendingContinuity = window.member.rtc_continuity_exchange(1n).then(value => {
        window.continuitySettled = true; return { completed: true, value: JSON.parse(value) };
      }, cause => { window.continuitySettled = true; return { completed: false, failure: String(cause) }; });
    });
    // This is a fixture observation window, not an SDK timeout alteration.
    await bounded(wait(150));
    report.cut.snapshot = JSON.parse(run('nft', ['-j', 'list', 'table', 'ip', table]));
    report.cut.dropped = counters(report.cut.snapshot);
    assert(report.cut.dropped.packets > 0 && report.cut.dropped.bytes > 0);
    assert.equal(await page.evaluate(() => window.continuitySettled), false, 'post-cut original stream must actually lose progress');
    report.pairDuringCut = await livePair(page);
    assert.equal(report.pairDuringCut.transport.selectedCandidatePairId, report.pairBefore.transport.selectedCandidatePairId);
    report.duringCut = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    report.savedBeforeRepair = JSON.parse(await page.evaluate(() => window.network.status()));
    save();
    if (relayBackup) {
      // Keep the approved relay dormant while exercising cancellation and the
      // shorter caller clock through actual authenticated repair controls.
      for (const [name, budget] of [['cancelledRepair', 2000], ['shortCallerRepair', 120]]) {
        let cancellationRequested;
        const startedControl = name === 'cancelledRepair' ? native.wait('repair-control', true) : null;
        const endedControl = name === 'cancelledRepair' ? native.wait('repair-control-ended', true) : null;
        startedControl?.catch(() => {}); endedControl?.catch(() => {});
        await page.evaluate(budget => {
          window.repairSettled = false;
          window.pendingRepair = window.member.recover_path(budget).then(value => {
            window.repairSettled = true; return JSON.parse(value);
          });
        }, budget);
        if (name === 'cancelledRepair') {
          report.cancelledControl = await startedControl;
          save();
          await bounded(wait(40));
          assert.equal(await page.evaluate(() => window.repairSettled), false);
          cancellationRequested = Date.now();
          await page.evaluate(() => window.member.cancel_path_recovery());
        }
        report[name] = await bounded(page.evaluate(() => window.pendingRepair));
        if (cancellationRequested) {
          report.cancellationObservedMilliseconds = Date.now()-cancellationRequested;
          assert(report.cancellationObservedMilliseconds <= 350);
        }
        save();
        if (endedControl) report.cancelledControlEnded = await endedControl;
        assert.equal(report[name].outcome, 'unreachable');
        assert.equal(report[name].stableId, report.member.memberStableId);
        assert.match(report[name].failure, name === 'cancelledRepair' ? /cancelled/ : /deadline/);
        assert(report[name].elapsedMs <= (name === 'cancelledRepair' ? 700 : 350));
        assert.equal(await page.evaluate(() => window.member.is_closed()), false);
        assert.equal(await page.evaluate(() => window.continuitySettled), false);
      }
      // A deadline can cancel before any native control is accepted. Do not
      // require a server event which the caller budget never promised.
      await bounded(wait(150));
      report.nativeAfterBoundedAttempts = await native.command('status', 'status');
      report.relayOpenedAfterOriginalRtcRelease = await page.evaluate(() => { window.releaseOwnedRelay(); return true; });
      save();
    }
    const control = native.wait('repair-control', true); control.catch(() => {});
    const endedControl = native.wait('repair-control-ended', true); endedControl.catch(() => {});
    report.repair = JSON.parse(await bounded(page.evaluate(() => window.member.recover_path(2000))));
    report.control = await control;
    assert.equal(report.control.restricted, true);
    assert.equal(report.control.alpn, 'miopunch/connection-path-recovery/1');
    assert(report.control.paths.every(path => path.isMqtt && !path.isRelay && !path.isIp && !path.isRtc));
    report.controlEnded = await endedControl;
    report.afterRepair = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    assert.equal(report.afterRepair.memberStableId, report.member.memberStableId);
    assert(report.afterRepair.paths.every(path => !path.isSignal));
    assert(report.repair.elapsedMs <= 2250);
    if (relayBackup) {
      assert.equal(report.repair.outcome, 'recovered', JSON.stringify(report.repair));
      assert.equal(report.repair.freshAck, true); assert.equal(report.repair.peerConfirmed, true);
      assert.equal(report.controlEnded.succeeded, true, JSON.stringify(report.controlEnded));
      report.streamAfterRepair = await bounded(page.evaluate(() => window.pendingContinuity));
      assert.equal(report.streamAfterRepair.completed, true);
      continuity(report.streamAfterRepair.value, 1, report.streamBeforeCut);
      report.streamLater = JSON.parse(await bounded(page.evaluate(() => window.member.rtc_continuity_exchange(2n))));
      continuity(report.streamLater, 2, report.streamBeforeCut);
      assert(report.afterRepair.paths.some(path => path.isRelay && path.selected));
      report.afterData = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
      assert(report.afterData.paths.some(path => path.isRelay && path.streamFramesSent > 0 && path.streamFramesReceived > 0));
      assert.equal(report.afterData.activeAssociations, 0);
      report.nativeRtcCleanup = await native.wait('rtc-path-cleanup');
      assert.equal(report.nativeRtcCleanup.stableId, report.streamBeforeCut.native.stableId);
      assert.equal(report.nativeRtcCleanup.cleanup.pcClosed, true);
      assert.equal(report.nativeRtcCleanup.cleanup.ownedPumpTasks, 0);
      assert.equal(report.nativeRtcCleanup.cleanup.txBytes, 0);
      assert.equal(report.nativeRtcCleanup.cleanup.rxBytes, 0);
      report.nativeImmediateCleanup = await native.command('status', 'status');
      await bounded(wait(100));
      report.nativeLateCleanup = await native.command('status', 'status');
      for (const value of [report.nativeImmediateCleanup, report.nativeLateCleanup]) {
        assert.equal(value.activeRtcAssociations, 0); assert.equal(value.activeRtcPumps, 0);
      }
      report.socketsAfterRtcRetirement = run('ss', ['-uapn']);
      assert(!report.socketsAfterRtcRetirement.includes(`:${remote.port} `), 'native selected RTC socket must close while original relay M stays live');
    } else {
      assert.equal(report.repair.outcome, 'unreachable', JSON.stringify(report.repair));
      assert.equal(await page.evaluate(() => window.member.is_closed()), false);
      assert.equal(await page.evaluate(() => window.continuitySettled), false);
      assert(report.afterRepair.paths.every(path => !path.isRelay));
      report.stillAliveUnreachable = true;
    }
    report.carriersAfterRepair = JSON.parse(await page.evaluate(() => window.network.path_diagnostics())).carriers;
    report.savedAfterRepair = JSON.parse(await page.evaluate(() => window.network.status()));
    assert.deepEqual(report.savedAfterRepair, report.savedBeforeRepair);
    assert(report.carriersAfterRepair.every(carrier => carrier.ready));
    assert(report.carriersAfterRepair[0].traffic.sent_bytes > report.carriersBeforeCut[0].traffic.sent_bytes, 'actual restricted repair must use the surviving signal facility');
    report.cut.finalSnapshot = JSON.parse(run('nft', ['-j', 'list', 'table', 'ip', table]));
    report.cut.finalDropped = counters(report.cut.finalSnapshot);
    // A real SDK close is a separate terminal story; do not claim idle expiry.
    await page.evaluate(() => window.member.close());
    report.terminal = { cause: 'explicit original SDK close after cut classification', closed: await page.evaluate(() => window.member.is_closed()), atUtc: new Date().toISOString() };
    assert.equal(report.terminal.closed, true);
    run('nft', ['delete', 'table', 'ip', table]); installed = false;
    report.cut.removedUtc = new Date().toISOString();
    report.afterCarrierReturns = JSON.parse(await bounded(page.evaluate(() => window.member.recover_path(2000))));
    assert.equal(report.afterCarrierReturns.outcome, 'terminal');
    assert.equal(report.afterCarrierReturns.stableId, report.member.memberStableId);
    if (!relayBackup) {
      report.failedOriginalStream = await bounded(page.evaluate(() => window.pendingContinuity));
      assert.equal(report.failedOriginalStream.completed, false);
    }
    report.finalBrowser = JSON.parse(await page.evaluate(() => window.member.rtc_path_diagnostics()));
    await bounded(page.waitForFunction(() => window.rtcObjects.every(peer => peer.connectionState === 'closed')));
    report.browserCleanup = await page.evaluate(() => ({ created: window.rtcObjects.length, closed: window.rtcObjects.filter(peer => peer.connectionState === 'closed').length,
      callbacksDetached: window.rtcObjects.every(peer => peer.onconnectionstatechange === null && peer.oniceconnectionstatechange === null && peer.onicegatheringstatechange === null) }));
    assert.equal(report.browserCleanup.created, report.browserCleanup.closed);
    assert.equal(report.browserCleanup.callbacksDetached, true);
    save();
    capture.kill('SIGINT');
    report.captureExit = await bounded(captureExit);
    report.captureProof = captureProof(join(evidence, 'rtc-selected-tuple.pcap'), directions);
  } finally {
    if (installed) {
      try { report.cut.finalSnapshot = JSON.parse(run('nft', ['-j', 'list', 'table', 'ip', table])); } catch (error) { report.cut.collectionError = String(error); }
      try { run('nft', ['delete', 'table', 'ip', table]); installed = false; } catch (error) { report.cut.cleanupError = String(error); }
    }
    if (capture && capture.exitCode === null && capture.signalCode === null) { capture.kill('SIGINT'); await wait(100); if (capture.exitCode === null && capture.signalCode === null) capture.kill('SIGKILL'); }
    report.cut.tableRemoved = !installed;
    writeFileSync(join(evidence, 'rtc-selected-tuple-capture.log'), captureLog);
    save();
  }
}
