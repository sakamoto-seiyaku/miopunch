// Owned headless technical host for real WASM forwarding. No system clipboard operations.
import { chromium } from 'playwright-core';
import http from 'node:http';
import readline from 'node:readline';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createHash } from 'node:crypto';

const [packagePath, executablePath, privateDirectory] = process.argv.slice(2);
const root = path.resolve(packagePath);
const entry = (await readdir(root)).find(name => name.endsWith('.js'));
if (!entry) throw new Error('missing immutable package entry');
const actors = new Map();
const output = value => process.stdout.write(JSON.stringify(value, (_, item) => typeof item === 'bigint' ? Number(item) : item) + '\n');
const server = http.createServer(async (request, response) => {
  try {
    if (request.url === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html' });
      response.end('<!doctype html><title>miopunch forwarding technical host</title>');
      return;
    }
    const file = path.resolve(root, '.' + new URL(request.url, 'http://localhost').pathname);
    if (!file.startsWith(root + path.sep)) throw new Error('outside package');
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = `http://127.0.0.1:${server.address().port}/`;
await mkdir(privateDirectory, { recursive: true, mode: 0o700 });
async function load(actor) {
  await actor.page.goto(address, { waitUntil: 'domcontentloaded' });
  await actor.page.evaluate(async entry => {
    window.api = await import('/' + entry);
    await window.api.default();
  }, entry);
  await actor.page.bringToFront();
}
async function bounded(promise, milliseconds) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('owned cleanup deadline')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}
let closingPromise;
function closeAll() {
  return closingPromise ??= (async () => {
    const cleanup = [];
    for (const [name, actor] of actors) {
      const failures = [];
      // Every owner is attempted even if one page operation or exit fails.
      try {
        if (actor.page) {
          const ended = await bounded(actor.page.evaluate(async () => {
            let clean = true;
            for (const operation of [() => window.node?.stop_forwarding(), () => window.node?.close()]) {
              try { await operation(); } catch { clean = false; }
            }
            return clean;
          }), 10000);
          if (!ended) failures.push('SDKClose');
        }
      } catch { failures.push('PageClose'); }
      try { await bounded(actor.cdp?.send('Browser.close') ?? Promise.resolve(), 5000); }
      catch { /* Chrome may close CDP before acknowledging Browser.close. */ }
      try {
        if (actor.process.exitCode === null && actor.process.signalCode === null) {
          await bounded(once(actor.process, 'exit'), 10000);
        }
      } catch { failures.push('ProcessExit'); }
      const terminal = actor.process.exitCode !== null || actor.process.signalCode !== null;
      if (!terminal || actor.process.exitCode !== 0 || actor.process.signalCode !== null) failures.push('NonzeroOrLiveProcess');
      try { await bounded(actor.browser?.close() ?? Promise.resolve(), 5000); }
      catch { if (!terminal) failures.push('AutomationClose'); }
      cleanup.push({ actor: name, pid: actor.process.pid, terminal, exit: actor.process.exitCode,
        closed: terminal && failures.length === 0, failures });
    }
    let server_closed = false;
    try {
      await bounded(new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())), 5000);
      server_closed = true;
    } catch { server.closeAllConnections(); }
    // Keep failed actor metadata until the caller has recorded the obligation.
    if (cleanup.every(item => item.terminal)) actors.clear();
    return { cleanup, server_closed };
  })();
}
async function dispatch({ actor: name, op, args = [] }) {
  if (name === 'runner' && op === 'close') return closeAll();
  if (op === 'open') {
    if (actors.has(name)) throw new Error('actor already exists');
    const profile = path.join(privateDirectory, name);
    await mkdir(profile, { recursive: true, mode: 0o700 });
    // Launch a normal owned browser first. The documented noDefaults attachment
    // avoids creating Playwright's original focus override in any CDP session.
    const child = spawn(executablePath, ['--no-sandbox', '--no-first-run', '--no-default-browser-check',
      '--headless=new',
      '--disable-background-networking', '--disable-component-update', '--disable-sync', '--no-pings', '--metrics-recording-only',
      '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'],
      { stdio: ['ignore', 'ignore', 'pipe'] });
    child.stderr.pipe(process.stderr);
    const actor = { process: child };
    actors.set(name, actor);
    let port;
    const deadline = Date.now() + 10000;
    while (Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('owned browser exited during startup');
      try { port = Number((await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); } catch { /* still starting */ }
      if (Number.isInteger(port) && port > 0) break;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (!port) throw new Error('owned browser did not publish its local debug port');
    actor.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { noDefaults: true, isLocal: true });
    actor.context = actor.browser.contexts()[0];
    actor.page = actor.context.pages()[0] ?? await actor.context.newPage();
    actor.cdp = await actor.context.newCDPSession(actor.page);
    await load(actor);
    return { browserVersion: actor.browser.version(), headless: true, normalPublicTrust: true,
      automationDefaultsApplied: false, focusEmulation: false, ownedBrowserPID: child.pid };
  }
  const actor = actors.get(name);
  if (!actor) throw new Error('unknown actor');
  if (op === 'reload') {
    await actor.page.evaluate(async () => { await window.node?.close(); });
    await load(actor);
    return true;
  }
  if (op === 'start') return actor.page.evaluate(async ([action, config, invitation]) => {
    window.node = await window.api.BrowserNetwork[action](config, invitation);
    return JSON.parse(window.node.status());
  }, args);
  if (op === 'begin_serve_admission') return actor.page.evaluate(() => {
    window.pendingAdmission = window.node.serve_admission(false);
    return { scheduled: true };
  });
  if (op === 'finish_serve_admission') return actor.page.evaluate(async () => await window.pendingAdmission);
  if (op === 'begin_forward_exchange') return actor.page.evaluate(([index, peer, protocol, target, size, seed]) => {
    const payload = Uint8Array.from({length: size}, (_, offset) => (offset + seed) % 251);
    window.pendingForwards ??= new Map();
    window.pendingForwards.set(index, window.node.forward_exchange(peer, protocol, target, payload, size)
      .then(response => ({ok: true, length: response.length}), () => ({ok: false})));
    return { scheduled: true };
  }, args);
  if (op === 'finish_forward_exchange') return actor.page.evaluate(async index => {
    const result = await window.pendingForwards.get(index);
    window.pendingForwards.delete(index);
    return result;
  }, args[0]);
  if (op === 'file_make') return actor.page.evaluate(([name, size]) => {
    const chunk = Uint8Array.from({length: 65536}, (_, index) => index % 251);
    const parts = [];
    for (let left = size; left > 0; left -= chunk.length) parts.push(chunk.subarray(0, Math.min(left, chunk.length)));
    window.sourceFiles ??= new Map();
    window.sourceFiles.set(name, new File(parts, name, {type: 'application/octet-stream'}));
    return {name, size};
  }, args);
  if (op === 'file_send_source') return actor.page.evaluate(([peer, name]) => window.node.send_file(peer, window.sourceFiles.get(name)), args);
  if (op === 'file_hash') {
    const size = await actor.page.evaluate(async ([name, source]) => {
      window.checksumFile = source ? window.sourceFiles.get(name) : await window.node.received_file(name);
      return window.checksumFile.size;
    }, args);
    const hash = createHash('sha256');
    for (let offset = 0; offset < size; offset += 65536) {
      const encoded = await actor.page.evaluate(async offset => {
        const bytes = new Uint8Array(await window.checksumFile.slice(offset, offset + 65536).arrayBuffer());
        return btoa(String.fromCharCode(...bytes));
      }, offset);
      hash.update(Buffer.from(encoded, 'base64'));
    }
    await actor.page.evaluate(() => { delete window.checksumFile; });
    return {size, sha256: hash.digest('hex')};
  }
  if (op === 'forward_exchange') return actor.page.evaluate(async ([peer, protocol, target, size, seed]) => {
    const payload = Uint8Array.from({length: size}, (_, index) => (index + seed) % 251);
    const response = await window.node.forward_exchange(peer, protocol, target, payload, size);
    return {length: response.length, exact: response.length === payload.length && response.every((value, index) => value === payload[index])};
  }, args);
  return actor.page.evaluate(async ([method, values]) => {
    if (method === 'issue_service_guest') values[2] = BigInt(values[2]);
    const result = await window.node[method](...values);
    if (['status', 'service_status', 'forwarding_status', 'sync_services', 'redeem_guest', 'guest_status', 'end_guest', 'file_status', 'admission_requests'].includes(method)) return JSON.parse(result);
    return result ?? null;
  }, [op, args]);
}
const input = readline.createInterface({ input: process.stdin });
let closing = false;
input.on('line', line => {
  let command;
  try { command = JSON.parse(line); }
  catch { output({ id: null, ok: false, failureClass: 'InvalidCommand' }); return; }
  dispatch(command).then(result => output({ id: command.id, ok: true, result }), failure => {
    output({ id: command.id, ok: false, failureClass: failure?.name ?? 'OperationFailed', privateError: String(failure) });
  }).finally(() => {
    if (command.actor === 'runner' && command.op === 'close') {
      closing = true; input.close(); process.stdin.pause();
    }
  });
});
input.on('close', async () => { if (!closing) { closing = true; await dispatch({ actor: 'runner', op: 'close' }); } });
output({ event: 'ready', localTechnicalHost: true });
