// Real remote acceptance: local Chromium/WASM -> deployed Linux native host.
import assert from 'node:assert/strict';
import { createHash, randomBytes, X509Certificate } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { chromium } from '../browser-probe/harness/node_modules/playwright-core/index.mjs';

const output = process.env.DEMO_CHECK_OUTPUT || '/tmp/miopunch-client-demo-20261021/check';
mkdirSync(output, { recursive: true });
const entry = readFileSync(process.env.DEMO_ENTRY_FILE || '/tmp/miopunch-client-demo-20261021/entry-url', 'utf8').trim();
const url = new URL(entry);
const report = { started: new Date().toISOString(), origin: url.origin, checks: [], errors: [], browserDirectTargetRequests: [], ids: {}, cleanup: {} };
const driver = readFileSync(fileURLToPath(import.meta.url));
writeFileSync(join(output, 'driver.mjs'), driver);
report.driverSHA256 = createHash('sha256').update(driver).digest('hex');
const certificateFile = process.env.DEMO_TEST_CERT_FILE;
const cdpURL = process.env.DEMO_BROWSER_CDP_URL;
const requireRTC = process.env.DEMO_REQUIRE_RTC === '1';
const browserSSH = process.env.DEMO_BROWSER_SSH_HOST;
const remoteArtifacts = browserSSH ? `/tmp/miopunch-demo-rtc-downloads-${Date.now()}` : undefined;
function remotePython(code, maxBuffer = 1024 * 1024) {
  const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
  return execFileSync('ssh', ['-o', 'BatchMode=yes', browserSSH, 'python3 -c ' + quote(code)], { maxBuffer });
}
const certificatePin = certificateFile ? createHash('sha256').update(new X509Certificate(readFileSync(certificateFile)).publicKey.export({ type: 'spki', format: 'der' })).digest('base64') : undefined;
report.tls = certificatePin ? { mode: 'temporary test browser trusts only the exact owned certificate public key', spkiSHA256: certificatePin } : { mode: url.protocol === 'http:' ? 'normal remote HTTP; no secure-context or TLS exceptions' : 'normal browser certificate validation' };
const check = (name, details = {}) => { report.checks.push({ name, passed: true, ...details }); console.log(name); };
function executable() {
  if (existsSync(chromium.executablePath())) return chromium.executablePath();
  for (const directory of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(x => x.startsWith('chromium-')).sort().reverse()) {
    for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const path = join(homedir(), '.cache/ms-playwright', directory, suffix);
      if (existsSync(path)) return path;
    }
  }
  throw new Error('Local Playwright Chromium is required');
}
let browser, context, page;
const createdForwards = [];
const createdTerminals = [];
const rpc = request => page.evaluate(request => window.demo.rpc(request), request);
async function connect() {
  await page.goto(entry);
  await page.locator('#connect').click();
  await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 60000 });
  assert.equal(await page.locator('[role=alert]').count(), 0);
}
const selectedRTC = paths => paths.some(path => path.selected && path.isRtc);
const rtcBytes = paths => paths.filter(path => path.isRtc).reduce((sum, path) => ({ sent: sum.sent + path.sentBytes, received: sum.received + path.receivedBytes }), { sent: 0, received: 0 });
try {
  if (remoteArtifacts) {
    remotePython(`from pathlib import Path; Path(${JSON.stringify(remoteArtifacts)}).mkdir(mode=0o700)`);
    report.remoteBrowserArtifacts = remoteArtifacts;
  }
  browser = cdpURL ? await chromium.connectOverCDP(cdpURL, { artifactsDir: remoteArtifacts }) : await chromium.launch({ executablePath: executable(), headless: true, args: ['--no-sandbox', ...(certificatePin ? [`--ignore-certificate-errors-spki-list=${certificatePin}`] : [])] });
  context = await browser.newContext({ acceptDownloads: true, permissions: ['clipboard-read', 'clipboard-write'], viewport: { width: 1280, height: 960 } });
  if (requireRTC) await context.addInitScript(() => {
    const NativeWebSocket = WebSocket;
    const sockets = new Set(); let blocked = false;
    const NativePeerConnection = RTCPeerConnection;
    const peers = [];
    const candidates = description => (description?.sdp || '').split('\n').filter(line => line.startsWith('a=candidate:')).map(line => {
      const fields = line.trim().split(/\s+/);
      return { protocol: fields[2], address: fields[4], port: Number(fields[5]), type: fields[7] };
    });
    class ObservedPeerConnection extends NativePeerConnection {
      constructor(configuration) {
        super(configuration);
        const entry = { pc: this, configuration, started: performance.now(), events: [] };
        peers.push(entry);
        const record = type => {
          entry.local = candidates(this.localDescription); entry.remote = candidates(this.remoteDescription);
          entry.events.push({ type, elapsedMs: performance.now() - entry.started, gathering: this.iceGatheringState, ice: this.iceConnectionState, connection: this.connectionState, signaling: this.signalingState });
        };
        this.__acceptanceRecord = record;
        for (const type of ['icegatheringstatechange', 'iceconnectionstatechange', 'connectionstatechange', 'signalingstatechange']) this.addEventListener(type, () => record(type));
        this.addEventListener('icecandidateerror', event => entry.events.push({ type: 'icecandidateerror', elapsedMs: performance.now() - entry.started, code: event.errorCode, text: event.errorText, url: event.url }));
        this.addEventListener('icecandidate', () => record('icecandidate'));
        record('created');
      }
      close() { this.__acceptanceRecord('before-close'); super.close(); }
    }
    Object.defineProperty(window, 'RTCPeerConnection', { value: ObservedPeerConnection });
    class ObservedWebSocket extends NativeWebSocket {
      constructor(...args) {
        if (blocked) throw new DOMException('Relay disabled by owned RTC acceptance check', 'NetworkError');
        super(...args); sockets.add(this); this.addEventListener('close', () => sockets.delete(this));
      }
    }
    Object.defineProperty(window, 'WebSocket', { value: ObservedWebSocket });
    Object.assign(window, { __rtcAcceptance: {
      dropRelay() { blocked = true; const count = sockets.size; for (const socket of sockets) socket.close(1000, 'RTC-only acceptance'); return count; },
      restoreRelay() { blocked = false; },
      peers() { return peers.map(({ pc, ...entry }) => entry); },
      async liveStats() {
        const facts = [];
        for (const { pc } of peers) {
          if (pc.connectionState !== 'connected') continue;
          const values = []; (await pc.getStats()).forEach(value => values.push(value));
          const transport = values.find(value => value.type === 'transport' && value.selectedCandidatePairId);
          const pair = values.find(value => value.id === transport?.selectedCandidatePairId);
          const candidate = id => {
            const value = values.find(value => value.id === id);
            return value && Object.fromEntries(['address', 'port', 'protocol', 'candidateType', 'relayProtocol'].filter(key => value[key] !== undefined).map(key => [key, value[key]]));
          };
          facts.push({ connectionState: pc.connectionState, iceConnectionState: pc.iceConnectionState,
            pair: pair && { state: pair.state, nominated: pair.nominated, bytesSent: pair.bytesSent, bytesReceived: pair.bytesReceived },
            local: candidate(pair?.localCandidateId), remote: candidate(pair?.remoteCandidateId) });
        }
        return facts;
      },
    } });
  });
  page = await context.newPage();
  page.on('pageerror', error => report.errors.push(String(error)));
  page.on('request', request => { if (new URL(request.url()).hostname === '127.0.0.1') report.browserDirectTargetRequests.push(request.url()); });
  await connect();
  if (requireRTC) {
    report.rtcBefore = await page.evaluate(() => window.demo.node.diagnostics());
    assert.equal(report.rtcBefore.network.rtcCapability.effective, true);
    assert.ok(selectedRTC(report.rtcBefore.session.paths), 'Browser member must select an actual RTC path');
    assert.ok(selectedRTC(report.rtcBefore.native.paths), 'Native member must select an actual RTC path');
    check('both endpoints select real WebRTC for the ordinary member connection');
  }
  if (url.protocol === 'http:') { assert.equal(await page.evaluate(() => window.isSecureContext), false); check('remote HTTP browser node starts without secure-context overrides'); }
  const ids = await page.evaluate(() => ({ browser: window.demo.node.id, host: window.demo.node.configuration.peer, saved: window.demo.node.status }));
  assert.notEqual(ids.browser, ids.host); assert.equal(ids.saved.authorization_saved, true); assert.equal(ids.saved.secret_saved, true); report.ids = ids;
  check('independent browser identity, real admission and member RPC');
  const unauthenticated = await page.evaluate(async () => (await fetch('/__config')).status);
  assert.equal(unauthenticated, 401); check('invitation endpoint refuses missing access capability');

  const bytes = randomBytes(100123); const sha256 = createHash('sha256').update(bytes).digest('hex');
  const name = `remote-check-${Date.now()}.bin`;
  await page.locator('input[type=file]').setInputFiles({ name, mimeType: 'application/octet-stream', buffer: bytes });
  await page.getByText(`${name} 已保存，远端 SHA-256 一致`, { exact: true }).waitFor({ timeout: 60000 });
  const metadata = (await rpc({ op: 'file.list' })).files.find(x => x.name === name);
  assert.equal(metadata.sha256, sha256); assert.equal(metadata.size, bytes.length);
  const downloadEvent = page.waitForEvent('download');
  await page.locator('li').filter({ hasText: name }).getByRole('button', { name: '下载', exact: true }).click();
  const download = await downloadEvent; const path = join(output, name);
  if (remoteArtifacts) {
    assert.equal(await download.failure(), null);
    const remotePath = await download.path();
    assert.ok(remotePath.startsWith(remoteArtifacts + '/') && !remotePath.slice(remoteArtifacts.length + 1).includes('/'));
    const content = remotePython(`from pathlib import Path; import sys; sys.stdout.buffer.write(Path(${JSON.stringify(remotePath)}).read_bytes())`, bytes.length + 1024);
    writeFileSync(path, content);
  } else await download.saveAs(path);
  assert.equal(createHash('sha256').update(readFileSync(path)).digest('hex'), sha256);
  check('UI upload, durable native save and download hash', { bytes: bytes.length, sha256 });
  const partial = await rpc({ op: 'file.begin', name: 'retry-check.txt', size: 3, sha256: createHash('sha256').update('abc').digest('hex') });
  const write = { op: 'file.write', id: partial.id, offset: 0, data: Buffer.from('abc').toString('base64') };
  assert.equal((await rpc(write)).offset, 3); assert.equal((await rpc(write)).offset, 3);
  await assert.rejects(() => rpc({ ...write, data: Buffer.from('xyz').toString('base64') }));
  await rpc({ op: 'file.finish', id: partial.id });
  check('exact file chunk retry accepted; changed retry refused');

  const text = `浏览器 → Linux 文本缓冲 → 浏览器 ${Date.now()}`;
  await page.getByRole('textbox', { name: '剪贴板文本', exact: true }).fill(text);
  await page.getByRole('button', { name: '发送文本', exact: true }).click();
  await page.getByText('已写入 Linux 文本缓冲', { exact: true }).waitFor();
  assert.equal((await rpc({ op: 'clipboard.get' })).text, text);
  await page.getByRole('button', { name: '取回并复制到本机', exact: true }).click();
  await page.getByText('已写入本机系统剪贴板', { exact: true }).waitFor();
  if (await page.evaluate(() => Boolean(navigator.clipboard))) assert.equal(await page.evaluate(() => navigator.clipboard.readText()), text);
  else { const input = page.getByRole('textbox', { name: '剪贴板文本', exact: true }); await input.fill(''); await input.focus(); await page.keyboard.press('Control+V'); assert.equal(await input.inputValue(), text); }
  check('headless buffer and actual browser system clipboard write');

  const config = await page.evaluate(() => window.demo.node.configuration);
  for (const protocol of ['tcp', 'udp']) {
    const rule = await rpc({ op: 'forward.create', protocol, host: '127.0.0.1', port: protocol === 'tcp' ? config.echoTcp : config.echoUdp }); createdForwards.push(rule.id);
    const data = Buffer.from(`real-${protocol}-${Date.now()}`).toString('base64');
    assert.equal((await rpc({ op: 'forward.probe', id: rule.id, data })).data, data);
    await rpc({ op: 'forward.stop', id: rule.id }); createdForwards.splice(createdForwards.indexOf(rule.id), 1);
    check(`native ${protocol.toUpperCase()} listener forwards an actual socket roundtrip`, { listen: rule.listen, target: rule.target });
  }
  await assert.rejects(() => rpc({ op: 'forward.create', protocol: 'tcp', host: '1.1.1.1', port: 80 }));
  await assert.rejects(() => rpc({ op: 'web.open', url: 'http://1.1.1.1/' }));
  check('public network destinations refused by demo target scope');

  await page.locator('#open-web').click();
  const frame = page.frameLocator('iframe[title="通过节点打开的网页"]');
  await frame.locator('#api').filter({ hasText: 'API: r_proxy' }).waitFor({ timeout: 30000 });
  assert.equal(await frame.locator('body').evaluate(() => window.parentReadable), false);
  assert.equal(await frame.locator('body').evaluate(() => typeof window.demo), 'undefined');
  const contentPost = await frame.locator('body').evaluate(async () => {
    const response = await fetch('/content-rpc-injection', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ op: 'clipboard.set', text: 'content-escalation' }) });
    return response.status;
  });
  assert.equal(contentPost, 404);
  assert.equal((await rpc({ op: 'clipboard.get' })).text, text);
  const mailboxNoAuth = await page.evaluate(async () => (await fetch('/__web/next?channel=not-a-capability')).status);
  assert.equal(mailboxNoAuth, 401);
  check('content RPC-shaped POST stays an HTTP target request; mailbox refuses missing access capability');
  await frame.locator('#next').click(); await frame.locator('#next-page').waitFor();
  await page.locator('#open-web').click(); await frame.locator('#login').waitFor();
  await frame.locator('input[name=user]').fill('demo'); await frame.locator('#login').click();
  await frame.locator('#private-result').waitFor({ timeout: 30000 });
  await page.screenshot({ path: join(output, 'connected-web.png'), fullPage: true });
  await page.getByRole('textbox', { name: '内网网页地址', exact: true }).fill(config.exampleWeb + 'large');
  await page.locator('#open-web').click(); await frame.getByRole('heading', { name: '大响应验证' }).waitFor({ timeout: 30000 });
  assert.equal(await frame.locator('p').textContent(), 'm'.repeat(100000));
  assert.deepEqual(report.browserDirectTargetRequests, []);
  check('isolated in-app Web: Fetch, navigation, login/Cookie, 100 KB response and no direct browser target request');

  const names = [`term-check-${Date.now().toString(36)}`, `term-other-${Date.now().toString(36)}`];
  for (const name of names) { await rpc({ op: 'terminal', action: 'attach', name }); createdTerminals.push(name); }
  const command = "export MIO_DEMO_STATE=retained; printf 'MIO_EXEC_%s\\n' \"$((6*7))\"; printf 'MIO_UID_%s\\n' \"$(id -u)\"; if test -r /state/access-token; then printf 'MIO_SECRET_%s\\n' readable; else printf 'MIO_SECRET_%s\\n' denied; fi\n";
  await rpc({ op: 'terminal', action: 'write', name: names[0], data: Buffer.from(command).toString('base64') });
  let outputText = ''; let offset = 0;
  for (let attempt = 0; attempt < 40 && !outputText.includes('MIO_SECRET_denied'); attempt++) {
    const value = await rpc({ op: 'terminal', action: 'poll', name: names[0], offset }); offset = value.offset; outputText += Buffer.from(value.data, 'base64').toString();
    if (!outputText.includes('MIO_SECRET_denied')) await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(outputText.includes('MIO_EXEC_42')); assert.ok(outputText.includes('MIO_UID_1000')); assert.ok(outputText.includes('MIO_SECRET_denied'));
  await rpc({ op: 'terminal', action: 'detach', name: names[0] });
  await rpc({ op: 'terminal', action: 'attach', name: names[0] });
  await rpc({ op: 'terminal', action: 'write', name: names[0], data: Buffer.from("printf 'MIO_REATTACH_%s\\n' \"$MIO_DEMO_STATE\"\n").toString('base64') });
  outputText = ''; offset = 0;
  for (let attempt = 0; attempt < 40 && !outputText.includes('MIO_REATTACH_retained'); attempt++) {
    const value = await rpc({ op: 'terminal', action: 'poll', name: names[0], offset }); offset = value.offset; outputText += Buffer.from(value.data, 'base64').toString();
    if (!outputText.includes('MIO_REATTACH_retained')) await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(outputText.includes('MIO_REATTACH_retained'));
  const listed = (await rpc({ op: 'terminal', action: 'list' })).sessions;
  for (const name of names) assert.ok(listed.includes(name));
  check('two real PTY/tmux workspaces; detach/reattach retains shell state; UID 1000 cannot read node credentials');
  await page.getByRole('button', { name: '刷新工作区', exact: true }).click();
  await page.getByRole('button', { name: `${names[0]} · 附着`, exact: true }).click();
  await page.getByText('已附着', { exact: true }).waitFor();
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type("printf 'MIO_UI_%s\\n' \"$((5*9))\""); await page.keyboard.press('Enter');
  await page.waitForFunction(() => document.querySelector('.xterm-rows')?.textContent.includes('MIO_UI_45'), { timeout: 20000 });
  await page.screenshot({ path: join(output, 'terminal.png'), fullPage: true });
  check('xterm keyboard input executes in real PTY and output returns to the UI');
  for (const name of [...createdTerminals]) { await rpc({ op: 'terminal', action: 'end', name }); createdTerminals.splice(createdTerminals.indexOf(name), 1); }

  if (requireRTC) {
    report.rtcAfter = await page.evaluate(() => window.demo.node.diagnostics());
    report.rtcIce = await page.evaluate(() => window.__rtcAcceptance.liveStats());
    assert.ok(report.rtcIce.some(value => value.pair?.state === 'succeeded' && value.local?.candidateType !== 'relay' && value.remote?.candidateType !== 'relay'));
    assert.equal(report.rtcAfter.native.stableId, report.rtcBefore.native.stableId);
    assert.ok(selectedRTC(report.rtcAfter.session.paths)); assert.ok(selectedRTC(report.rtcAfter.native.paths));
    const before = rtcBytes(report.rtcBefore.session.paths); const after = rtcBytes(report.rtcAfter.session.paths);
    assert.ok(after.sent > before.sent); assert.ok(after.received > before.received);
    check('file, clipboard, sockets, in-app Web and terminal increase RTC traffic on the same member session', { before, after });
    const dropped = await page.evaluate(() => window.__rtcAcceptance.dropRelay());
    assert.ok(dropped > 0, 'the owned check must close a real relay WebSocket');
    assert.equal((await rpc({ op: 'info' })).name, 'r_proxy Linux');
    const chunk = await rpc({ op: 'file.read', id: metadata.id, offset: 0 });
    assert.deepEqual(Buffer.from(chunk.data, 'base64'), bytes.subarray(0, Buffer.from(chunk.data, 'base64').length));
    report.rtcWithoutRelay = await page.evaluate(() => window.demo.node.diagnostics());
    assert.equal(report.rtcWithoutRelay.native.stableId, report.rtcBefore.native.stableId);
    assert.ok(selectedRTC(report.rtcWithoutRelay.session.paths));
    check('same member session returns real file bytes while relay WebSockets are closed and reconnects are blocked');
    await page.evaluate(() => window.__rtcAcceptance.restoreRelay());
  }

  const previous = ids.browser;
  if (url.protocol === 'http:') {
    const other = await context.newPage();
    await other.goto(entry); await other.locator('#connect').click(); await other.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 60000 });
    assert.equal(await other.evaluate(() => window.demo.node.id), previous);
    assert.equal((await rpc({ op: 'info' })).name, 'r_proxy Linux');
    await page.evaluate(() => window.demo.node.close());
    assert.equal((await other.evaluate(() => window.demo.rpc({ op: 'info' }))).name, 'r_proxy Linux');
    await other.evaluate(() => window.demo.node.close()); await other.close();
    check('two HTTP views reuse one browser identity; detaching one keeps the other connected');
  }
  await page.evaluate(() => window.demo.node.close());
  await page.reload(); await page.locator('#connect').click(); await page.locator('#status').filter({ hasText: '已连接' }).waitFor({ timeout: 60000 });
  assert.equal(await page.evaluate(() => window.demo.node.id), previous);
  assert.equal((await rpc({ op: 'clipboard.get' })).text, text);
  check('browser reopen keeps identity, membership and saved application state');
  assert.deepEqual(report.errors, []);
  report.passed = true;
} catch (error) {
  report.passed = false; report.failure = String(error); console.error(error);
  if (requireRTC && page) report.rtcPeersAtFailure = await page.evaluate(() => window.__rtcAcceptance?.peers()).catch(() => undefined);
  if (page) await page.screenshot({ path: join(output, 'failure.png'), fullPage: true }).catch(() => {});
} finally {
  for (const id of createdForwards) await rpc({ op: 'forward.stop', id }).catch(() => {});
  for (const name of createdTerminals) await rpc({ op: 'terminal', action: 'end', name }).catch(() => {});
  if (page) await page.evaluate(() => window.demo?.node.close()).catch(() => {});
  await context?.close(); await browser?.close();
  report.cleanup = { checkBrowserClosed: true, checkForwardsRemoved: true, checkTerminalsEnded: true, deployedDemoLeftRunning: true };
  report.finished = new Date().toISOString(); writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
}
if (!report.passed) process.exitCode = 1;
