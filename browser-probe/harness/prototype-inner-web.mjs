// THROWAWAY real-node Web proof. Run through ../prototype-web/run.py.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, existsSync, readFileSync, readdirSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const assets = join(root, 'browser-probe/prototype-web');
const inputPath = process.argv[process.argv.indexOf('--inputs') + 1];
const inputs = JSON.parse(readFileSync(inputPath));
const output = inputs.output;
const automatic = process.argv.includes('--check');
const privateDirectory = join(output, 'private');
mkdirSync(privateDirectory, { mode: 0o700 });
for (const [path, digest] of Object.entries({ ...inputs.browser.artifacts, ...inputs.native.artifacts })) {
  assert.equal(createHash('sha256').update(readFileSync(path)).digest('hex'), digest, `changed runtime artifact: ${path}`);
}
const report = { scope: 'owned local Browser WASM -> admitted Iroh member stream -> Native HTTP -> isolated content origin',
  publicTraffic: false, inputs: inputPath, checks: [], providerHttp: [], browserRequests: [], cleanup: {} };
let provider, relay, browser, page, configuration;
const children = [];
const servers = [];
const bounded = (promise, label, milliseconds = 30000) => {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), milliseconds);
  })]).finally(() => clearTimeout(timer));
};
function child(name, args) {
  const process = spawn(inputs.native.programs[name], args, { stdio: ['pipe', 'pipe', 'pipe'] });
  children.push(process);
  const lines = [];
  const waiters = [];
  let stderr = '';
  const exited = new Promise(resolve => process.once('exit', (code, signal) => resolve({ code, signal })));
  createInterface({ input: process.stdout }).on('line', line => {
    lines.push(line);
    let safe = line;
    try {
      const value = JSON.parse(line);
      if (value.code) value.code = '[private invitation]';
      safe = JSON.stringify(value);
      if (value.event === 'http') report.providerHttp.push(value);
    } catch {}
    console.log(`${name}: ${safe}`);
    for (const waiter of [...waiters]) if (waiter.pattern.test(line)) {
      waiters.splice(waiters.indexOf(waiter), 1);
      waiter.resolve(line);
    }
  });
  process.stderr.on('data', bytes => { stderr += String(bytes); });
  process.on('error', error => { for (const waiter of waiters) waiter.reject(error); });
  return { process, exited, stderr: () => stderr, wait(pattern) {
    const found = lines.find(line => pattern.test(line));
    if (found) return Promise.resolve(found);
    return bounded(new Promise((resolve, reject) => waiters.push({ pattern, resolve, reject })), `${name} ${pattern}`);
  }};
}
async function listen(server) {
  servers.push(server);
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  return `http://127.0.0.1:${server.address().port}`;
}
function send(response, status, type, body, headers = {}) {
  response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store', ...headers });
  response.end(body);
}
const cookie = randomUUID();
const fixtureRequests = [];
const fixture = createServer(async (request, response) => {
  const path = new URL(request.url, 'http://node-only.invalid').pathname;
  fixtureRequests.push({ path, method: request.method, cookiePresent: Boolean(request.headers.cookie), agent: request.headers['user-agent'] });
  if (path === '/style.css') return send(response, 200, 'text/css', 'body{font:16px system-ui;background:rgb(246,250,255);padding:28px;color:#183653}a{color:#245bba}button,input{font:inherit;padding:8px}img{width:48px;height:48px}');
  if (path === '/app.js') return send(response, 200, 'text/javascript', 'document.querySelector("#script").textContent="脚本已加载";fetch("/api/status").then(r=>r.json()).then(v=>document.querySelector("#api").textContent=v.status);');
  if (path === '/mark.svg') return send(response, 200, 'image/svg+xml', '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" rx="12" fill="#245bba"/><path d="M12 25l8 8 17-19" fill="none" stroke="white" stroke-width="4"/></svg>');
  if (path === '/api/status') return send(response, 200, 'application/json', JSON.stringify({ status: 'node-route-ok' }));
  if (path === '/login' && request.method === 'POST') {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    assert.equal(new URLSearchParams(Buffer.concat(chunks).toString()).get('name'), 'demo');
    return send(response, 303, 'text/plain', '', { location: '/private', 'set-cookie': `prototype_session=${cookie}; Path=/; HttpOnly; SameSite=Lax` });
  }
  if (path === '/private') {
    if (!request.headers.cookie?.includes(`prototype_session=${cookie}`)) return send(response, 401, 'text/plain', 'login required');
    return send(response, 200, 'text/html; charset=utf-8', '<!doctype html><link rel="stylesheet" href="/style.css"><h1 id="logged-in">已登录</h1><p>表单、跳转和会话 cookie 已经由提供节点处理。</p><a id="home" href="/">返回首页</a>');
  }
  if (path === '/next') return send(response, 200, 'text/html; charset=utf-8', '<!doctype html><link rel="stylesheet" href="/style.css"><h1 id="next-page">第二页</h1><a id="home" href="/">返回首页</a>');
  if (path === '/') return send(response, 200, 'text/html; charset=utf-8', '<!doctype html><html lang="zh-CN"><meta charset="utf-8"><link rel="stylesheet" href="/style.css"><img id="mark" src="/mark.svg"><h1>节点内网站点</h1><p>此页面通过 miopunch 的成员连接加载。</p><p id="script">加载脚本…</p><p id="api">加载 API…</p><p><a id="next" href="/next">打开第二页</a></p><form action="/login" method="post"><input name="name" aria-label="测试用户" value="demo"><button id="login">测试登录</button></form><script src="/app.js"></script></html>');
  return send(response, 404, 'text/plain', 'missing owned fixture path');
});
const content = createServer((request, response) => {
  const path = new URL(request.url, 'http://localhost').pathname;
  if (path === '/__bridge.html') return send(response, 200, 'text/html; charset=utf-8', readFileSync(join(assets, 'bridge.html')));
  if (path === '/__sw.js') return send(response, 200, 'text/javascript', readFileSync(join(assets, 'service-worker.js')), { 'service-worker-allowed': '/' });
  return send(response, 503, 'text/plain', 'The provider route is not ready.');
});
const management = createServer((request, response) => {
  const path = new URL(request.url, 'http://localhost').pathname;
  if (path === '/__config') return send(response, 200, 'application/json', JSON.stringify(configuration));
  if (path === '/') return send(response, 200, 'text/html; charset=utf-8', readFileSync(join(assets, 'index.html')));
  if (path.startsWith('/pkg/')) {
    const file = resolve(inputs.browser.pkg, path.slice('/pkg/'.length));
    if (!file.startsWith(resolve(inputs.browser.pkg) + '/')) return send(response, 403, 'text/plain', 'refused');
    response.setHeader('content-type', file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
    return createReadStream(file).on('error', () => response.writeHead(404).end()).pipe(response);
  }
  return send(response, 404, 'text/plain', 'missing');
});
function executable() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE) return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
  if (existsSync(chromium.executablePath())) return chromium.executablePath();
  for (const name of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(name => name.startsWith('chromium-')).sort().reverse()) {
    for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const candidate = join(homedir(), '.cache/ms-playwright', name, suffix);
      if (existsSync(candidate)) return candidate;
    }
  }
  throw new Error('A local Playwright Chromium executable is required.');
}

try {
  await listen(fixture);
  relay = child('browser-probe-native', ['relay', '0']);
  const relayUrl = `http://${(await relay.wait(/^RELAY /)).slice(6)}/`;
  provider = child('prototype_inner_web', [join(privateDirectory, 'native'), relayUrl, String(fixture.address().port)]);
  const ready = JSON.parse(await provider.wait(/"event":"ready"/));
  const contentOrigin = await listen(content);
  configuration = { peer: ready.id, code: ready.code, relay: relayUrl, contentOrigin };
  const address = await listen(management);
  report.managementOrigin = address;
  report.contentOrigin = contentOrigin;
  report.providerId = ready.id;
  report.nativeHttpFixturePort = fixture.address().port;
  if (!automatic) {
    console.log(`Prototype ready: ${address}\nOpen this URL, connect the node, then open http://node-only.invalid/. Ctrl+C stops the owned fixtures.`);
    await new Promise(resolve => process.once('SIGINT', resolve));
    report.interactiveReady = true;
  } else {
    browser = await chromium.launch({ executablePath: executable(), headless: true, args: ['--no-sandbox'] });
    const context = await browser.newContext();
    page = await context.newPage();
    page.on('request', request => report.browserRequests.push(request.url()));
    const browserErrors = [];
    page.on('pageerror', error => browserErrors.push(String(error)));
    await page.goto(address);
    await page.waitForFunction(() => window.ready !== undefined);
    await bounded(page.evaluate(() => window.ready), 'independent browser node');
    await assert.rejects(() => page.evaluate(id => window.prototype.network().open_member_connection(id), ready.id));
    assert.equal(fixtureRequests.length, 0);
    report.checks.push('unadmitted node cannot open member business');
    await page.locator('#connect').click();
    try {
      await page.waitForFunction(() => !document.querySelector('#open').disabled, null, { timeout: 30000 });
    } catch (error) {
      throw new Error(`${error}; UI status: ${await page.locator('#status').textContent()}; errors: ${browserErrors.join('; ')}`);
    }
    report.browserId = await page.evaluate(() => window.prototype.network().id());
    assert.notEqual(report.browserId, report.providerId);
    report.session = await page.evaluate(() => JSON.parse(window.prototype.member().session_diagnostics()));
    assert.equal(report.session.memberAuthorized, true);
    assert.equal(report.session.expectedPeerMatched, true);
    await page.locator('#open').click();
    const frame = page.frameLocator('#content');
    await frame.locator('#api').getByText('node-route-ok').waitFor();
    assert.equal(await frame.locator('#script').textContent(), '脚本已加载');
    assert.equal(await frame.locator('body').evaluate(element => getComputedStyle(element).backgroundColor), 'rgb(246, 250, 255)');
    assert.equal(await frame.locator('#mark').evaluate(element => element.complete && element.naturalWidth > 0), true);
    report.checks.push('HTML, CSS, script, image and Fetch API all traverse member connection');
    const isolation = await frame.locator('body').evaluate(() => {
      let blocked = false;
      try { void parent.prototype.network(); } catch { blocked = true; }
      return { managementReadBlocked: blocked, ownOrigin: location.origin, managementBridgeAbsent: !window.prototype };
    });
    assert.equal(isolation.managementReadBlocked, true);
    assert.equal(isolation.managementBridgeAbsent, true);
    assert.notEqual(isolation.ownOrigin, address);
    report.isolation = isolation;
    report.checks.push('content cannot read management window or its node handle');
    await page.screenshot({ path: join(output, 'connected-page.png'), fullPage: true });
    await frame.locator('#next').click();
    await frame.locator('#next-page').waitFor();
    await frame.locator('#home').click();
    await frame.locator('#login').click();
    await frame.locator('#logged-in').waitFor();
    assert(fixtureRequests.some(request => request.path === '/private' && request.cookiePresent));
    report.checks.push('navigation, POST, redirect and provider-side HTTP cookie session');
    await page.screenshot({ path: join(output, 'logged-in-page.png'), fullPage: true });
    const countBeforeRefusal = fixtureRequests.length;
    const refused = await page.evaluate(async () => JSON.parse(await window.prototype.member().exchange(JSON.stringify({ path: '//outside.invalid/', method: 'GET' }))));
    assert.match(refused.error, /refused/);
    assert.equal(fixtureRequests.length, countBeforeRefusal);
    report.checks.push('provider rejects out-of-scope target before HTTP');
    assert(report.providerHttp.length >= 10);
    assert(report.browserRequests.every(url => !url.includes('node-only.invalid') && !url.includes(`:${fixture.address().port}/`)));
    assert(fixtureRequests.every(request => request.agent?.startsWith('curl/')));
    report.checks.push('browser never requests target directly; HTTP requests originate at provider');
    await new Promise(resolve => fixture.close(resolve));
    const unavailable = await page.evaluate(async () => JSON.parse(await window.prototype.member().exchange(JSON.stringify({ path: '/', method: 'GET' }))));
    assert.match(unavailable.error, /provider HTTP failed/);
    report.checks.push('unreachable target returns an explicit request failure');
    await page.evaluate(() => window.closePrototype());
    const closed = await page.evaluate(async () => {
      try { await window.prototype.member().exchange('{}'); return false; } catch { return true; }
    });
    assert(closed);
    report.checks.push('closed member connection cannot continue requests');
    report.requestCount = await page.evaluate(() => window.prototype.requestCount());
    report.browserErrors = browserErrors;
    assert.equal(browserErrors.length, 0);
    report.passed = true;
  }
} catch (error) {
  report.passed = false;
  report.failure = String(error.stack || error);
  if (page) await page.screenshot({ path: join(output, 'failure.png'), fullPage: true }).catch(() => {});
  process.exitCode = 1;
} finally {
  if (page) await page.evaluate(() => window.closePrototype?.()).catch(() => {});
  await browser?.close();
  for (const [name, actor] of [['provider', provider], ['relay', relay]]) {
    if (!actor) continue;
    if (actor.process.exitCode === null) {
      if (name === 'provider') actor.process.stdin.end('stop\n');
      else actor.process.kill('SIGINT');
    }
    try {
      report.cleanup[name] = await bounded(actor.exited, `${name} shutdown`, 10000);
      if (report.cleanup[name].code !== 0) { report.passed = false; process.exitCode = 1; }
    }
    catch (error) { report.cleanup[name] = { error: String(error), forced: true }; actor.process.kill('SIGKILL'); await actor.exited; report.passed = false; process.exitCode = 1; }
    writeFileSync(join(output, `${name}.stderr.log`), actor.stderr());
  }
  for (const server of servers) if (server.listening) await new Promise(resolve => server.close(resolve));
  rmSync(privateDirectory, { recursive: true, force: true });
  report.cleanup.privateStateRemoved = !existsSync(privateDirectory);
  report.fixtureRequests = fixtureRequests;
  writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ passed: report.passed, checks: report.checks, failure: report.failure, evidence: output, cleanup: report.cleanup }, null, 2));
}
