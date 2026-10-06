// Thin owned Playwright lifecycle only. SDK control remains on BrowserPage RPC.
import { chromium } from 'playwright-core';
import { networkObservation } from './relay-cdp-observer.mjs';
import readline from 'node:readline';

const [executablePath, profile, address] = process.argv.slice(2);
const url = new URL(address);
if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || !url.port
    || !url.searchParams.get('actor')) throw new Error('exact owned localhost page required');
const output = value => process.stdout.write(JSON.stringify(value) + '\n');
let context, page, launch, stopping = false;
const sockets = new Map(), observations = [];
const input = readline.createInterface({ input: process.stdin });
async function close() {
  if (stopping) return;
  stopping = true;
  try {
    if (launch) await launch.catch(() => {});
    if (page) await page.close();
    if (context) await context.close();
    output({ event: 'closed', ownedTargetAbsent: true, contextClosed: true, observations });
  } catch (error) {
    output({ event: 'close-failed', failureClass: error.name }); process.exitCode = 1;
  } finally { input.close(); process.stdin.pause(); }
}
input.on('line', line => { if (line === 'close') close(); });
input.on('close', () => { if (!stopping) close(); });
process.on('SIGTERM', close);
process.on('SIGINT', close);
try {
  launch = chromium.launchPersistentContext(profile, {
    executablePath, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false,
  });
  context = await launch;
  if (!stopping) {
    // Only this fresh persistent context is owned. Observe its single test page.
    page = context.pages()[0] ?? await context.newPage();
    const cdp = await context.newCDPSession(page);
    for (const method of ['Network.webSocketCreated', 'Network.webSocketWillSendHandshakeRequest',
      'Network.webSocketHandshakeResponseReceived', 'Network.webSocketClosed',
      'Network.webSocketFrameError', 'Network.loadingFailed']) {
      cdp.on(method, params => {
        const observation = networkObservation({ method, params }, sockets);
        if (observation && observations.length < 512) {
          observations.push(observation); output({ event: 'network-observation', observation });
        }
      });
    }
    await cdp.send('Network.enable');
    await page.goto(address, { waitUntil: 'domcontentloaded', timeout: 20000 });
    output({ event: 'ready', browserVersion: context.browser().version(), normalPublicTrust: true });
  }
} catch (error) {
  output({ event: 'setup-failed', failureClass: error.name }); process.exitCode = 1;
  await close();
}
