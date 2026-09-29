import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, readdirSync } from 'node:fs';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const probe = join(root, 'target/debug/browser-probe-native');
const browserRoot = join(root, 'browser-probe');
const children = [];

function withTimeout(promise, label) {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out`)), 20000);
    }),
  ]).finally(() => clearTimeout(timer));
}

function start(args, pattern) {
  const child = spawn(probe, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child);
  child.stderr.on('data', data => process.stderr.write(data));
  const lines = createInterface({ input: child.stdout });
  return new Promise((resolveReady, reject) => {
    const timeout = setTimeout(() => reject(new Error(`${args[0]} timed out`)), 30000);
    child.on('exit', code => reject(new Error(`${args[0]} exited ${code}`)));
    lines.on('line', line => {
      console.log(`${args[0]}: ${line}`);
      const match = line.match(pattern);
      if (match) {
        clearTimeout(timeout);
        resolveReady(match[1]);
      }
    });
  });
}

function chromiumPath() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE) return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
  const pinned = chromium.executablePath();
  if (existsSync(pinned)) return pinned;
  const cache = join(homedir(), '.cache/ms-playwright');
  if (existsSync(cache)) {
    for (const directory of readdirSync(cache).filter(name => name.startsWith('chromium-')).sort().reverse()) {
      for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
        const path = join(cache, directory, suffix);
        if (existsSync(path)) return path;
      }
    }
  }
  throw new Error('Chromium unavailable; run node browser-probe/harness/node_modules/playwright-core/cli.js install chromium or set PLAYWRIGHT_CHROMIUM_EXECUTABLE');
}

function staticServer() {
  const files = new Map([
    ['/', ['index.html', 'text/html']],
    ['/pkg/miopunch_browser_probe.js', ['pkg/miopunch_browser_probe.js', 'text/javascript']],
    ['/pkg/miopunch_browser_probe_bg.wasm', ['pkg/miopunch_browser_probe_bg.wasm', 'application/wasm']],
  ]);
  return createServer((request, response) => {
    const [path, type] = files.get(new URL(request.url, 'http://localhost').pathname) ?? [];
    if (!path) return response.writeHead(404).end();
    response.setHeader('Content-Type', type);
    createReadStream(join(browserRoot, path)).on('error', () => response.writeHead(500).end()).pipe(response);
  });
}

let browser;
const web = staticServer();
try {
  assert(existsSync(probe), 'build browser-probe-native first');
  assert(existsSync(join(browserRoot, 'pkg/miopunch_browser_probe_bg.wasm')), 'run wasm-bindgen build first');
  const relayAddr = await start(['relay', '0'], /^RELAY (\S+)/);
  const relay = `http://${relayAddr}/`;
  const nativeId = await start(['serve', relay], /^READY (\S+)/);
  await new Promise(resolveListening => web.listen(0, '127.0.0.1', resolveListening));

  browser = await chromium.launch({ executablePath: chromiumPath(), headless: true, args: ['--no-sandbox'] });
  const context = await browser.newContext();
  const page = await context.newPage();
  const requests = [];
  page.on('request', request => requests.push(request.url()));
  page.on('websocket', socket => requests.push(socket.url()));
  page.on('console', message => console.log(`browser console: ${message.text()}`));
  page.on('pageerror', error => console.error(`browser error: ${error}`));
  const url = `http://127.0.0.1:${web.address().port}/?relay=${encodeURIComponent(relay)}`;
  await page.goto(url);
  const first = await withTimeout(page.evaluate(() => window.ready), 'first browser start');
  assert.notEqual(first, nativeId, 'browser needs its own node identity');
  assert.equal(await withTimeout(page.evaluate(target => window.node.ping(target), nativeId), 'first exchange'), 'PONG');
  console.log(`FIRST_EXCHANGE browser=${first} native=${nativeId} relay=${relay}`);

  await page.reload();
  const restored = await withTimeout(page.evaluate(() => window.ready), 'reloaded browser start');
  assert.equal(restored, first, 'reload must restore browser identity');
  assert.equal(await withTimeout(page.evaluate(target => window.node.ping(target), nativeId), 'reloaded exchange'), 'PONG');
  console.log(`RELOAD_IDENTITY ${restored}`);

  await page.evaluate(() => localStorage.clear());
  await page.reload();
  const cleared = await withTimeout(page.evaluate(() => window.ready), 'cleared browser start');
  assert.notEqual(cleared, first, 'cleared storage must not impersonate old identity');
  assert.equal(await withTimeout(page.evaluate(target => window.node.ping(target), nativeId), 'cleared exchange'), 'PONG');
  console.log(`CLEARED_IDENTITY ${cleared}`);

  assert(requests.every(value => value.startsWith(`http://127.0.0.1:${web.address().port}/`) || value.startsWith(`ws://${relayAddr}/`) || value.startsWith(`http://${relayAddr}/`)), `unexpected external request: ${requests}`);
  assert(requests.some(value => value.startsWith(`ws://${relayAddr}/`)), 'browser must use the test relay');
  console.log(`BROWSER_WEBSOCKET ${requests.find(value => value.startsWith(`ws://${relayAddr}/`))}`);
  console.log('PASS: real Chromium, local relay, authenticated native echo, reload and clear');
} finally {
  await browser?.close();
  await new Promise(resolveClose => web.close(resolveClose));
  for (const child of children) child.kill('SIGINT');
}
