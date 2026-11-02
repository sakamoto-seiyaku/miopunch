// Real HTTP browser: missing RTC API fallback and exclusive host loss.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { chromium } from '../browser-probe/harness/node_modules/playwright-core/index.mjs';

const output = process.env.DEMO_CHECK_OUTPUT;
mkdirSync(output, { recursive: true });
const entry = readFileSync(process.env.DEMO_ENTRY_FILE, 'utf8').trim();
const browser = await chromium.connectOverCDP(process.env.DEMO_BROWSER_CDP_URL);
const report = { origin: new URL(entry).origin, checks: [] };
let context;
async function connect(page) {
  await page.goto(entry); await page.locator('#connect').click();
  await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 90000 });
}
try {
  context = await browser.newContext();
  // A negative capability check, never used for the positive RTC acceptance.
  await context.addInitScript(() => Object.defineProperty(window, 'RTCPeerConnection', { value: undefined }));
  let page = await context.newPage(); await connect(page);
  const diagnostics = await page.evaluate(() => window.demo.node.diagnostics());
  assert.equal(diagnostics.network.rtcCapability.effective, false);
  assert.ok(!diagnostics.session.paths.some(path => path.selected && path.isRtc));
  assert.equal((await page.evaluate(() => window.demo.rpc({ op: 'info' }))).name, 'r_proxy Linux');
  report.missingApi = diagnostics;
  report.checks.push('missing RTC API is reported and actual member RPC uses fallback');
  await page.evaluate(() => window.demo.node.close()); await context.close();

  context = await browser.newContext();
  page = await context.newPage(); await connect(page);
  const originalId = await page.evaluate(() => window.demo.node.id);
  const other = await context.newPage(); await connect(other);
  assert.equal(await other.evaluate(() => window.demo.node.id), originalId);
  // Abruptly close the actual SDK-owning Window while another view survives.
  await page.close();
  const failure = await other.evaluate(async () => {
    try { await window.demo.rpc({ op: 'info' }); return null; }
    catch (error) { return String(error); }
  });
  assert.match(failure, /宿主|关闭/);
  const reconnect = await other.evaluate(async () => {
    try { await window.demo.node.connect(sessionStorage.getItem('miopunch-demo-access'), () => {}); return null; }
    catch (error) { return String(error); }
  });
  assert.match(reconnect, /关闭/);
  report.ownerLoss = { originalId, failure, reconnect };
  report.checks.push('owner Window loss fails closed; surviving view cannot start another runtime');
  await context.close();
  report.passed = true;
} catch (error) { report.passed = false; report.failure = String(error); }
finally {
  await context?.close().catch(() => {}); await browser.close();
  writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
}
console.log(JSON.stringify({ passed: report.passed, checks: report.checks, failure: report.failure }));
if (!report.passed) process.exitCode = 1;
