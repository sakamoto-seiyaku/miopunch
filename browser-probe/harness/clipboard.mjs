// Minimal technical host. Real headed Chromium, real WASM and system clipboard.
import { chromium } from 'playwright-core';
import http from 'node:http';
import readline from 'node:readline';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

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
      response.end('<!doctype html><title>miopunch clipboard technical host</title>');
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
            for (const operation of [() => window.node?.stop_clipboard(),
              () => window.pendingClipboardStart, () => window.node?.close()]) {
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
      '--ozone-platform=x11',
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
    return { browserVersion: actor.browser.version(), headed: true, normalPublicTrust: true,
      automationDefaultsApplied: false, focusEmulation: false, ownedBrowserPID: child.pid };
  }
  const actor = actors.get(name);
  if (!actor) throw new Error('unknown actor');
  if (op === 'grant_clipboard') {
    await actor.context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: address });
    await actor.page.bringToFront();
    return { actualContextPermissions: true };
  }
  if (op === 'revoke_clipboard') {
    await actor.context.clearPermissions();
    return { actualContextPermissionsCleared: true };
  }
  if (op === 'document_context') return actor.page.evaluate(() => ({
    focused: document.hasFocus(), visibility: document.visibilityState, secure: isSecureContext,
  }));
  if (op === 'copy_owned_png') return actor.page.evaluate(async () => {
    const bytes = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAX+XDSwAAAABJRU5ErkJggg=='), character => character.charCodeAt(0));
    await navigator.clipboard.write([new ClipboardItem({ 'image/png': new Blob([bytes], { type: 'image/png' }) })]);
    return { actual_owned_image_write: true };
  });
  if (op === 'begin_clipboard_start') return actor.page.evaluate(target => {
    window.pendingClipboardSettled = false;
    window.pendingClipboardStart = window.node.start_clipboard(target).then(result => {
      window.pendingClipboardSettled = true;
      return { ok: true, status: JSON.parse(result) };
    }, () => {
      window.pendingClipboardSettled = true;
      return { ok: false };
    });
    return { scheduled: true };
  }, args[0]);
  if (op === 'clipboard_start_pending') return actor.page.evaluate(() => !window.pendingClipboardSettled);
  if (op === 'finish_clipboard_start') return actor.page.evaluate(async () => await window.pendingClipboardStart);
  if (op === 'reload') {
    await actor.page.evaluate(async () => { await window.node?.close(); });
    await load(actor);
    return true;
  }
  if (op === 'start') return actor.page.evaluate(async ([action, config, invitation]) => {
    window.node = await window.api.BrowserNetwork[action](config, invitation);
    return JSON.parse(window.node.status());
  }, args);
  return actor.page.evaluate(async ([method, values]) => {
    if (method === 'issue_guest') values[1] = BigInt(values[1]);
    if (method === 'issue_service_guest') values[2] = BigInt(values[2]);
    const result = await window.node[method](...values);
    if (['status', 'service_status', 'clipboard_status', 'enable_clipboard', 'disable_clipboard',
      'start_clipboard', 'stop_clipboard', 'sync_services', 'redeem_guest', 'guest_status', 'end_guest'].includes(method)) return JSON.parse(result);
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
