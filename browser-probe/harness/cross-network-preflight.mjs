import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { writeFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const service = process.env.MIO_SERVICE_IP;
const expected = process.env.MIO_BROWSER_OUTER_IP;
const report = { scope: 'owned browser TLS/WSS and STUN gathering prerequisite; no selected peer pair or member data',
  normalVerification: true, certificateBypassFlags: [], iceServers: [`stun:${service}:3478`], turnServers: [], passed: false };
let context;
const web = createServer((_, response) => response.end('<!doctype html><title>Owned cross-network prerequisite</title>'));
const timeout = setTimeout(() => { console.error('owned browser prerequisite deadline'); process.exitCode = 1; }, 12000);
try {
  await new Promise(resolve => web.listen(38080, '127.0.0.1', resolve));
  context = await chromium.launchPersistentContext('/fixture/runtime/profile', {
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE, headless: true,
    args: ['--no-sandbox', '--disable-features=WebRtcHideLocalIpsWithMdns'],
    env: { ...process.env, LD_LIBRARY_PATH: '/opt/hostlib' }, ignoreHTTPSErrors: false,
  });
  report.browserVersion = context.browser().version();
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:38080/');
  report.gather = await page.evaluate(async ({ service }) => {
    const socket = new WebSocket('wss://mio-mqtt.test:8084/mqtt/normal', ['mqtt']);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error('normally verified WSS failed')); });
    const websocket = { protocol: socket.protocol, readyState: socket.readyState };
    socket.close();
    await new Promise(resolve => { socket.onclose = resolve; });
    const peer = new RTCPeerConnection({ iceServers: [{ urls: `stun:${service}:3478` }] });
    const channel = peer.createDataChannel('owned-preflight', { negotiated: true, id: 0, ordered: false, maxRetransmits: 0 });
    const candidates = [];
    let complete;
    const gathering = new Promise(resolve => { complete = resolve; });
    peer.onicecandidate = event => {
      if (!event.candidate) return complete();
      const value = event.candidate;
      candidates.push({ address: value.address, port: value.port, protocol: value.protocol, type: value.type,
        relatedAddress: value.relatedAddress, relatedPort: value.relatedPort });
    };
    await peer.setLocalDescription(await peer.createOffer());
    await gathering;
    const state = peer.iceGatheringState;
    peer.onicecandidate = null; channel.close(); peer.close();
    return { websocket, candidates, state, peerClosed: peer.connectionState === 'closed', documentVisibility: document.visibilityState };
  }, { service });
  assert.equal(report.gather.websocket.protocol, 'mqtt');
  assert.equal(report.gather.state, 'complete');
  assert(report.gather.candidates.some(value => value.type === 'srflx' && value.address === expected && value.protocol === 'udp'));
  assert.equal(report.gather.peerClosed, true);
  report.passed = true;
} catch (error) {
  report.failure = String(error); process.exitCode = 1;
} finally {
  clearTimeout(timeout);
  await context?.close();
  web.closeAllConnections(); await new Promise(resolve => web.close(resolve));
  writeFileSync('/evidence/browser-prerequisite.json', JSON.stringify(report, null, 2)+'\n');
  console.log(JSON.stringify({ event: 'browser-prerequisite', passed: report.passed }));
}
