// Owned headless technical host for real WASM forwarding. No system clipboard operations.
import { chromium } from 'playwright-core';
import http from 'node:http';
import https from 'node:https';
import readline from 'node:readline';
import { readFile, readdir, mkdir, open, readlink } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createHash } from 'node:crypto';

const [packagePath, executablePath, privateDirectory, webOptionsPath] = process.argv.slice(2);
const webOptions = webOptionsPath ? JSON.parse(await readFile(webOptionsPath, 'utf8')) : null;
if (webOptions && (!/^content-[0-9a-f]+\.m119\.test$/.test(new URL(webOptions.contentOrigin).hostname)
  || !/^management-[0-9a-f]+\.m119\.test$/.test(webOptions.managementHost))) throw new Error('content origins must be owned test names');
const root = path.resolve(packagePath);
const entry = (await readdir(root)).find(name => name.endsWith('.js'));
if (!entry) throw new Error('missing immutable package entry');
const actors = new Map();
async function processIdentity(pid) {
  const stat = await readFile(`/proc/${pid}/stat`, 'utf8');
  return { pid, startTicks: stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19],
    commandSHA256: createHash('sha256').update(await readFile(`/proc/${pid}/cmdline`)).digest('hex') };
}
async function journalActor(actor, phase, details = {}) {
  const file = await open(path.join(actor.profile, 'm119-process-owner.jsonl'), 'a', 0o600);
  try {
    await file.writeFile(JSON.stringify({ phase, pid: actor.process.pid ?? null, monotonic: performance.now(), ...details }) + '\n');
    await file.sync();
  } finally { await file.close(); }
}
async function qualifyActor(actor) {
  await journalActor(actor, 'spawn_created', { expectedCommandSHA256: actor.expectedCommandSHA256, startTicks: null });
  const original = await processIdentity(actor.process.pid);
  await journalActor(actor, 'launch_observed', original);
  const deadline = performance.now() + 2000;
  let previous;
  while (performance.now() < deadline) {
    if (actor.process.exitCode !== null || actor.process.signalCode !== null) throw new Error('owned browser exited during qualification');
    const current = await processIdentity(actor.process.pid);
    if (current.startTicks !== original.startTicks) throw new Error('owned browser launch identity changed');
    if (current.commandSHA256 === actor.expectedCommandSHA256 && previous?.commandSHA256 === current.commandSHA256) {
      actor.identity = current;
      await journalActor(actor, 'qualified', { ...current, networkNamespace: await readlink(`/proc/${current.pid}/ns/net`) });
      return;
    }
    previous = current;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  throw new Error('owned browser launch never qualified');
}
async function normalBrowserClose(actor) {
  let current;
  try { current = await processIdentity(actor.process.pid); }
  catch (error) {
    await journalActor(actor, 'normal_close_identity_read_failed', { code: error.code ?? null, errorClass: error.name });
    throw error;
  }
  await journalActor(actor, 'normal_close_identity_observed', { expected: actor.identity, observed: current });
  if (!actor.identity || current.startTicks !== actor.identity.startTicks
    || actor.process.exitCode !== null || actor.process.signalCode !== null) {
    throw new Error('owned browser identity differs before normal close');
  }
  // Chromium can rewrite argv after startup. Keep the registered command
  // identity immutable; this is normal protocol shutdown of the original
  // live ChildProcess, never an OS signal or adoption of another PID.
  await journalActor(actor, 'normal_close_original_child_proof', {
    sameOriginalStart: true, commandChanged: current.commandSHA256 !== actor.identity.commandSHA256,
    osSignalPermitted: false,
  });
  if (actor.cdp) {
    await journalActor(actor, 'normal_cdp_close_intent', current);
    await bounded(actor.cdp.send('Browser.close'), 5000);
    return;
  }
  // A browser can publish its own private endpoint before the automation
  // attachment succeeds. Close this same ChildProcess through that endpoint.
  const [port, route] = (await readFile(path.join(actor.profile, 'DevToolsActivePort'), 'utf8')).split('\n');
  if (!Number.isInteger(Number(port)) || Number(port) < 1 || Number(port) > 65535 || !route?.startsWith('/devtools/browser/')) {
    throw new Error('owned private browser endpoint unavailable');
  }
  if (port !== actor.privateEndpoint?.port || route !== actor.privateEndpoint?.route) {
    throw new Error('private endpoint differs from original child launch');
  }
  await journalActor(actor, 'normal_private_cdp_close_intent', current);
  const socket = new WebSocket(`ws://127.0.0.1:${port}${route}`);
  try {
    await bounded(new Promise((resolve, reject) => {
      socket.onopen = () => socket.send(JSON.stringify({ id: 1, method: 'Browser.close' }));
      socket.onmessage = event => { if (JSON.parse(event.data).id === 1) resolve(); };
      socket.onclose = () => resolve();
      socket.onerror = () => reject(new Error('owned browser normal-close endpoint failed'));
    }), 5000);
  } finally { socket.close(); }
}
const output = value => process.stdout.write(JSON.stringify(value, (_, item) => typeof item === 'bigint' ? Number(item) : item) + '\n');
const serve = async (request, response) => {
  try {
    if (request.url === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html', ...(webOptions ? {
        'Content-Security-Policy': `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' ws://127.0.0.1:*; frame-src ${webOptions.contentOrigin}; object-src 'none'`,
      } : {}) });
      response.end('<!doctype html><title>miopunch technical host</title>' + (webOptions ? '<iframe id="content" sandbox="allow-scripts allow-forms allow-same-origin"></iframe>' : ''));
      return;
    }
    const file = path.resolve(root, '.' + new URL(request.url, 'http://localhost').pathname);
    if (!file.startsWith(root + path.sep)) throw new Error('outside package');
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
};
const server = webOptions ? https.createServer({ key: await readFile(webOptions.key), cert: await readFile(webOptions.certificate) }, serve) : http.createServer(serve);
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = `${webOptions ? 'https' : 'http'}://${webOptions ? webOptions.managementHost : '127.0.0.1'}:${server.address().port}/`;
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
            for (const operation of [() => window.node?.stop_web?.(), () => window.node?.stop_forwarding(), () => window.node?.close()]) {
              try { await operation(); } catch { clean = false; }
            }
            return clean;
          }), 10000);
          if (!ended) failures.push('SDKClose');
        }
      } catch { failures.push('PageClose'); }
      try { if (actor.process.exitCode === null && actor.process.signalCode === null) await normalBrowserClose(actor); }
      catch (error) {
        await journalActor(actor, 'normal_close_attempt_failed', { code: error.code ?? null, errorClass: error.name });
        // The process wait below determines whether a close reply was merely
        // interrupted by normal exit or the browser is still outstanding.
      }
      try {
        if (actor.process.exitCode === null && actor.process.signalCode === null) {
          await bounded(once(actor.process, 'exit'), 10000);
        }
      } catch { failures.push('ProcessExit'); }
      const terminal = actor.process.exitCode !== null || actor.process.signalCode !== null;
      await journalActor(actor, 'exit_observed', { terminal, exit: actor.process.exitCode, signal: actor.process.signalCode });
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
    const child = spawn(executablePath, [...(webOptions ? [`--ignore-certificate-errors-spki-list=${webOptions.contentSPKI}`, '--host-resolver-rules=MAP *.m119.test 127.0.0.1'] : ['--no-sandbox']), '--no-first-run', '--no-default-browser-check',
      '--headless=new',
      '--disable-background-networking', '--disable-component-update', '--disable-sync', '--no-pings', '--metrics-recording-only',
      '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'],
      { stdio: ['ignore', 'ignore', 'pipe'] });
    child.stderr.pipe(process.stderr);
    const actor = { process: child, profile, phase: 'spawned',
      expectedCommandSHA256: createHash('sha256').update(Buffer.from(child.spawnargs.join('\0') + '\0')).digest('hex') };
    actors.set(name, actor);
    await qualifyActor(actor);
    actor.phase = 'waiting_private_endpoint';
    let port;
    const deadline = performance.now() + 10000;
    while (performance.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('owned browser exited during startup');
      try {
        const [portText, route] = (await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n');
        port = Number(portText);
        if (Number.isInteger(port) && port > 0 && port < 65536 && route?.startsWith('/devtools/browser/')) {
          actor.privateEndpoint = { port: portText, route };
          await journalActor(actor, 'private_endpoint_pinned', {
            endpointSHA256: createHash('sha256').update(JSON.stringify(actor.privateEndpoint)).digest('hex'),
          });
          break;
        }
      } catch { /* still starting */ }
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (!actor.privateEndpoint) throw new Error('owned browser did not publish its local debug port');
    actor.phase = 'private_endpoint_ready';
    if (args[0]?.attach === false) return { ownedBrowserPID: child.pid, cdpAttached: false,
      processQualified: true, browserSandboxDisabledByHarness: !webOptions };
    actor.phase = 'attaching_cdp';
    actor.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { noDefaults: true, isLocal: true });
    actor.context = actor.browser.contexts()[0];
    actor.page = actor.context.pages()[0] ?? await actor.context.newPage();
    actor.cdp = await actor.context.newCDPSession(actor.page);
    actor.phase = 'loading_owned_package';
    await load(actor);
    actor.phase = 'ready';
    return { browserVersion: actor.browser.version(), headless: true, normalPublicTrust: !webOptions,
      contentCertificatePinned: !!webOptions, browserSandboxDisabledByHarness: !webOptions,
      automationDefaultsApplied: false, focusEmulation: false, ownedBrowserPID: child.pid };
  }
  const actor = actors.get(name);
  if (!actor) throw new Error('unknown actor');
  if (webOptions && op === 'content_open') {
    if (new URL(args[0]).origin !== webOptions.contentOrigin) throw new Error('different content origin');
    await actor.page.locator('#content').evaluate((frame, url) => { frame.src = url; }, args[0]);
    return { opened: true };
  }
  if (webOptions && op === 'content_evaluate') {
    const frame = actor.page.frames().find(frame => frame.parentFrame() === actor.page.mainFrame() && frame.url().startsWith(webOptions.contentOrigin + '/'));
    if (!frame) return null;
    return frame.evaluate(args[0]);
  }
  if (webOptions && op === 'management_prepare') return actor.page.evaluate(async value => {
    localStorage.setItem('M119-management-canary', value);
    document.cookie = 'M119-management-cookie=' + value + '; SameSite=Strict; Path=/';
    await new Promise((resolve, reject) => {
      const opening = indexedDB.open('M119-management-private', 1);
      opening.onupgradeneeded = () => opening.result.createObjectStore('secret');
      opening.onerror = () => reject(new Error('owned canary storage'));
      opening.onsuccess = () => {
        const database = opening.result;
        const transaction = database.transaction('secret', 'readwrite');
        transaction.objectStore('secret').put(value, 'canary');
        transaction.oncomplete = () => { database.close(); resolve(); };
        transaction.onerror = () => { database.close(); reject(new Error('owned canary transaction')); };
      };
    });
    return { saved: localStorage.getItem('M119-management-canary') === value };
  }, args[0]);
  if (webOptions && op === 'management_check') return actor.page.evaluate(async value => {
    const indexedDBUnchanged = await new Promise((resolve, reject) => {
      const opening = indexedDB.open('M119-management-private', 1);
      opening.onerror = () => reject(new Error('owned canary storage'));
      opening.onsuccess = () => {
        const database = opening.result;
        const reading = database.transaction('secret').objectStore('secret').get('canary');
        reading.onsuccess = () => { database.close(); resolve(reading.result === value); };
        reading.onerror = () => { database.close(); reject(new Error('owned canary read')); };
      };
    });
    return { storageUnchanged: localStorage.getItem('M119-management-canary') === value,
      cookieUnchanged: document.cookie.includes('M119-management-cookie=' + value), indexedDBUnchanged,
      nodePresent: !!window.node };
  }, args[0]);
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
    if (['status', 'service_status', 'forwarding_status', 'sync_services', 'redeem_guest', 'guest_status', 'end_guest', 'file_status', 'admission_requests', 'request_web', 'web_status', 'open_web_socket', 'read_web_socket', 'close_web_socket'].includes(method)) return JSON.parse(result);
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
    output({ id: command.id, ok: false, failureClass: failure?.name ?? 'OperationFailed',
      failurePhase: actors.get(command.actor)?.phase ?? 'dispatch', privateError: String(failure) });
  }).finally(() => {
    if (command.actor === 'runner' && command.op === 'close') {
      closing = true; input.close(); process.stdin.pause();
    }
  });
});
input.on('close', async () => { if (!closing) { closing = true; await dispatch({ actor: 'runner', op: 'close' }); } });
output({ event: 'ready', localTechnicalHost: true });
