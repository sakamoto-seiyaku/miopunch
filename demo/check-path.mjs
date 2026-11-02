// Real SDK/UI regression: repeated healthy RTC exploration and failed ICE.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { chromium } from '../browser-probe/harness/node_modules/playwright-core/index.mjs';

const output = process.env.DEMO_CHECK_OUTPUT;
mkdirSync(output, { recursive: true });
const entry = readFileSync(process.env.DEMO_ENTRY_FILE, 'utf8').trim();
const unavailable = process.env.DEMO_PATH_CASE === 'unavailable';
let executable;
for (const directory of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(x => x.startsWith('chromium-')).sort().reverse()) {
  for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
    const candidate = join(homedir(), '.cache/ms-playwright', directory, suffix);
    if (existsSync(candidate)) executable ||= candidate;
  }
}
const browser = process.env.DEMO_BROWSER_CDP_URL
  ? await chromium.connectOverCDP(process.env.DEMO_BROWSER_CDP_URL)
  : await chromium.launch({ executablePath: executable, headless: true, args: ['--no-sandbox'] });
const context = await browser.newContext();
if (unavailable) await context.addInitScript(() => {
  // Deliberate test-only network failure: RTC APIs remain available, but ICE
  // requires a TURN candidate and this application configures no TURN server.
  const Original = RTCPeerConnection;
  class NoDirectCandidates extends Original {
    constructor(configuration) { super({ ...configuration, iceTransportPolicy: 'relay' }); }
  }
  Object.defineProperty(window, 'RTCPeerConnection', { value: NoDirectCandidates, configurable: true });
});
const page = await context.newPage();
const report = { origin: new URL(entry).origin, case: unavailable ? 'test-only-unavailable-ICE' : 'healthy-RTC', checks: [] };
try {
  await page.goto(entry); await page.locator('#connect').click();
  await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 90000 });
  report.before = await page.evaluate(() => window.demo.node.diagnostics());
  assert.equal(report.before.network.rtcCapability.effective, true);
  assert.equal(report.before.session.paths.some(path => path.selected && path.isRtc), !unavailable);
  await page.getByRole('button', { name: '重新探测直连', exact: true }).click();
  await page.waitForFunction(() => Array.from(document.querySelectorAll('button'))
    .some(button => button.textContent === '重新探测直连' && !button.disabled), undefined, { timeout: 70000 });
  report.after = await page.evaluate(() => window.demo.node.diagnostics());
  report.pathLabel = await page.locator('#path-status').textContent();
  assert.ok(!report.pathLabel.includes('正在'));
  report.errors = await page.locator('[role=alert]').allTextContents();
  assert.equal(report.after.native.stableId, report.before.native.stableId);
  assert.equal((await page.evaluate(() => window.demo.rpc({ op: 'info' }))).name, 'r_proxy Linux');
  assert.equal(await page.getByRole('button', { name: '重新探测直连', exact: true }).isEnabled(), true);
  assert.ok(report.after.browserRtc.length > 0);
  if (unavailable) {
    assert.equal(report.after.rtcAttempt.ok, false);
    assert.ok(report.errors.length > 0);
    assert.match(report.pathLabel, /Relay/);
    assert.ok(report.after.session.paths.some(path => path.selected && path.isRelay));
    assert.ok(!report.after.session.paths.some(path => path.selected && path.isRtc));
    assert.ok(report.after.browserRtc.some(pc => pc.states.some(state => state.gathering === 'complete') && pc.candidates.length === 0));
    report.checks.push('available RTC APIs with unavailable ICE fail honestly; current relay and member RPC survive');
  } else {
    assert.deepEqual(report.errors, []);
    assert.equal(report.after.rtcAttempt.result.outcome, 'already-direct');
    assert.match(report.pathLabel, /WebRTC/);
    assert.equal(report.after.browserRtc.length, report.before.browserRtc.length);
    assert.ok(report.after.session.paths.some(path => path.selected && path.isRtc));
    report.checks.push('repeated healthy RTC request reuses the current association without changing member identity');
  }
  report.checks.push('UI settles after exploration; latest attempt and real path diagnostics are refreshed');
  report.passed = true;
} catch (error) {
  report.passed = false;
  report.failure = String(error).replaceAll(entry, new URL(entry).origin + '/[private-entry]');
} finally {
  await page.evaluate(() => window.demo?.node.close()).catch(() => {});
  await context.close(); await browser.close();
  writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
}
console.log(JSON.stringify({ passed: report.passed, case: report.case, checks: report.checks, failure: report.failure }));
if (!report.passed) process.exitCode = 1;
