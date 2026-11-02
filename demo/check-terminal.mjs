// Reproduce terminal control replies leaking from historical PTY output.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { chromium } from '../browser-probe/harness/node_modules/playwright-core/index.mjs';

const output = process.env.DEMO_CHECK_OUTPUT;
mkdirSync(output, { recursive: true });
const entry = readFileSync(process.env.DEMO_ENTRY_FILE, 'utf8').trim();
const browser = await chromium.connectOverCDP(process.env.DEMO_BROWSER_CDP_URL);
const context = await browser.newContext({ viewport: { width: 1280, height: 960 } });
const page = await context.newPage();
const name = `term-replay-${Date.now().toString(36)}`;
const rpc = request => page.evaluate(request => window.demo.rpc(request), request);
const report = { origin: new URL(entry).origin, session: name };
try {
  await page.goto(entry); await page.locator('#connect').click();
  await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 90000 });
  const diagnostics = await page.evaluate(() => window.demo.node.diagnostics());
  assert.equal(diagnostics.session.memberAuthorized, true);
  report.rtcSelected = diagnostics.session.paths.some(path => path.selected && path.isRtc);
  await rpc({ op: 'terminal', action: 'attach', name });
  await new Promise(resolve => setTimeout(resolve, 1500));
  const historical = await rpc({ op: 'terminal', action: 'poll', name, offset: 0 });
  const initial = Buffer.from(historical.data, 'base64');
  writeFileSync(join(output, 'initial-pty.bin'), initial);
  report.historicalDeviceQuery = /\x1b\[[?>]?[0-9;]*c/.test(initial.toString());
  assert.ok(report.historicalDeviceQuery, 'the real old PTY output must contain the device query');
  await page.getByRole('button', { name: '刷新工作区', exact: true }).click();
  await page.getByRole('button', { name: `${name} · 附着`, exact: true }).click();
  await page.getByText('已附着', { exact: true }).waitFor();
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('echo $((6*7))');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => {
    const lines = Array.from(document.querySelectorAll('.xterm-rows > div'), row => row.textContent.trim());
    return lines.includes('42') || lines.some(line => line.includes('command not found'));
  }, undefined, { timeout: 10000 });
  const lines = await page.locator('.xterm-rows > div').allTextContents();
  report.screen = lines.join('\n');
  assert.ok(lines.some(line => line.trim() === '42'), 'historical device replies must not become shell input');
  report.passed = true;
} catch (error) {
  report.passed = false; report.failure = String(error);
  report.screen = await page.locator('.xterm-rows').textContent().catch(() => undefined);
} finally {
  await page.screenshot({ path: join(output, 'terminal.png'), fullPage: true }).catch(() => {});
  await rpc({ op: 'terminal', action: 'end', name }).catch(() => {});
  await page.evaluate(() => window.demo?.node.close()).catch(() => {});
  await context.close(); await browser.close();
  writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
}
console.log(JSON.stringify(report));
if (!report.passed) process.exitCode = 1;
