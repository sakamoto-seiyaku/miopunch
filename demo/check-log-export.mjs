// Export an actual browser download before connection and after an ICE failure.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { chromium } from '../browser-probe/harness/node_modules/playwright-core/index.mjs';
const output = process.env.DEMO_CHECK_OUTPUT;
mkdirSync(output, { recursive: true });
const entry = readFileSync(process.env.DEMO_ENTRY_FILE, 'utf8').trim();
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
const context = await browser.newContext({ acceptDownloads: true });
await context.addInitScript(() => {
  const Original = RTCPeerConnection;
  class NoDirectCandidates extends Original {
    constructor(configuration) { super({ ...configuration, iceTransportPolicy: 'relay' }); }
  }
  Object.defineProperty(window, 'RTCPeerConnection', { value: NoDirectCandidates, configurable: true });
});
const page = await context.newPage();
const report = { case: 'browser-log-export-with-test-only-unavailable-ICE', checks: [] };
async function download(name) {
  const pending = page.waitForEvent('download');
  await page.locator('#export-log').click();
  const download = await pending;
  assert.match(download.suggestedFilename(), /^miopunch-diagnostics-.*\.json$/);
  const file = join(output, name + '.json'); await download.saveAs(file);
  const text = readFileSync(file, 'utf8');
  assert.ok(!text.includes(new URL(entry).hash.slice(1)) || !new URL(entry).hash);
  const access = new URLSearchParams(new URL(entry).hash.slice(1)).get('access');
  if (access) assert.ok(!text.includes(access));
  assert.ok(!/a=ice-(?:pwd|ufrag):/.test(text));
  return JSON.parse(text);
}
try {
  await page.goto(entry);
  const disconnected = await download('disconnected');
  assert.equal(disconnected.connection.connected, false); assert.equal(disconnected.paths, null);
  report.checks.push('download works before connection without access credential');
  await page.locator('#connect').click();
  await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 90000 });
  await page.getByRole('button', { name: '重新探测直连', exact: true }).click();
  await page.waitForFunction(() => Array.from(document.querySelectorAll('button'))
    .some(button => button.textContent === '重新探测直连' && !button.disabled), undefined, { timeout: 70000 });
  const failed = await download('failed-ice');
  assert.match(failed.connection.error, /DataPathUnavailable|DeadlineExceeded/);
  assert.ok(failed.events.some(event => event.event === 'improve-failed' && event.detail === failed.connection.error));
  assert.equal(failed.paths.rtcAttempt.ok, false);
  assert.equal(failed.paths.network.rtcCapability.effective, true);
  assert.ok(failed.paths.browserRtc.some(pc => pc.states.some(state => state.gathering === 'complete')));
  assert.ok(failed.paths.session.paths.some(path => path.isRelay && path.selected));
  assert.ok(failed.paths.native.stableId);
  report.checks.push('failed ICE export retains exact error, timestamped attempt, browser ICE and both current paths');
  // Lose the UI broker: a bounded export still downloads its cached ICE record.
  await page.evaluate(() => {
    const marker = 'do-not-export-business-content';
    document.querySelector('textarea').value = marker;
    window.demo.node.worker.port.close();
  });
  const started = Date.now(); const offline = await download('broker-unavailable');
  assert.ok(Date.now() - started < 10000); assert.match(offline.snapshotError, /状态采集超时/);
  assert.equal(offline.paths.rtcAttempt.ok, false);
  assert.ok(!JSON.stringify(offline).includes('do-not-export-business-content'));
  report.checks.push('unavailable broker exports cached diagnostics within a bounded wait, without business content');
  report.passed = true;
} catch (error) {
  report.passed = false; report.failure = String(error).replaceAll(entry, '[private-entry]');
} finally {
  await context.close(); await browser.close();
  writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
}
console.log(JSON.stringify(report));
if (!report.passed) process.exitCode = 1;
