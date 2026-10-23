// Technical host only: independent real Chromium profiles and WASM nodes.
// Stdout is a private controller pipe; the controller redacts secret returns.
import { chromium } from 'playwright-core';
import http from 'node:http';
import readline from 'node:readline';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import path from 'node:path';

const [packagePath, executablePath, privateDirectory] = process.argv.slice(2);
const packageRoot = path.resolve(packagePath);
const entry = (await readdir(packageRoot)).find(name => name.endsWith('.js'));
if (!entry) throw new Error('missing package entry');
const actors = new Map();
const output = value => process.stdout.write(JSON.stringify(value, (_, item) => typeof item === 'bigint' ? Number(item) : item) + '\n');
const server = http.createServer(async (request, response) => {
  try {
    if (request.url === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html' });
      response.end('<!doctype html><title>miopunch technical onboarding host</title>');
      return;
    }
    const file = path.resolve(packageRoot, '.' + new URL(request.url, 'http://localhost').pathname);
    if (!file.startsWith(packageRoot + path.sep)) throw new Error('outside owned package');
    const content = await readFile(file);
    response.writeHead(200, { 'Content-Type': file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript' });
    response.end(content);
  } catch {
    response.writeHead(404); response.end();
  }
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
}

async function dispatch(command) {
  const { actor: name, op, args = [] } = command;
  if (op === 'close' && name === 'runner') {
    const cleanup = [];
    for (const [name, actor] of actors) {
      try {
        await actor.page.evaluate(async () => { await window.node?.close(); });
        await actor.context.close();
        cleanup.push({ actor: name, closed: true });
      } catch { cleanup.push({ actor: name, closed: false }); }
    }
    actors.clear();
    await new Promise(resolve => server.close(resolve));
    return { cleanup };
  }
  if (op === 'open') {
    if (actors.has(name)) throw new Error('actor already exists');
    const context = await chromium.launchPersistentContext(path.join(privateDirectory, name), {
      executablePath, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false,
    });
    const actor = { context, page: context.pages()[0] ?? await context.newPage() };
    actors.set(name, actor);
    await load(actor);
    return { browserVersion: context.browser().version(), normalPublicTrust: true };
  }
  const actor = actors.get(name);
  if (!actor) throw new Error('unknown actor');
  if (op === 'duplicate_start') {
    const page = await actor.context.newPage();
    try {
      await load({ page });
      return await page.evaluate(async () => {
        try { const node = await window.api.BrowserNetwork.start('{}'); await node.close(); return { rejected: false }; }
        catch { return { rejected: true }; }
      });
    } finally { await page.close(); }
  }
  if (op === 'reload') {
    await actor.page.evaluate(async () => { await window.node?.close(); });
    await load(actor);
    return true;
  }
  if (op === 'api') {
    return actor.page.evaluate(() => ({
      create: typeof window.api.BrowserNetwork.create === 'function',
      join: typeof window.api.BrowserNetwork.join === 'function',
      pair: typeof window.api.BrowserNetwork.pair === 'function',
    }));
  }
  if (op === 'start') {
    return actor.page.evaluate(async ([action, configuration, invitation]) => {
      window.node = await window.api.BrowserNetwork[action](configuration, invitation);
      return JSON.parse(window.node.status());
    }, args);
  }
  if (op === 'inspect_recovery_card') {
    return actor.page.evaluate(values => JSON.parse(window.api.BrowserNetwork.inspect_recovery_card(...values)), args);
  }
  if (op === 'fail_next_save') {
    return actor.page.evaluate(() => {
      const original = IDBObjectStore.prototype.put;
      IDBObjectStore.prototype.put = function (...args) {
        IDBObjectStore.prototype.put = original;
        this.transaction.abort();
        throw new DOMException('owned save-failure injection', 'QuotaExceededError');
      };
      return true;
    });
  }
  if (op === 'member_accept') {
    return actor.page.evaluate(async () => {
      const attempt = window.node.member_accept_attempt(12000);
      try { window.member = await attempt.accept(); return JSON.parse(window.member.session_diagnostics()); }
      finally { attempt.free(); }
    });
  }
  if (op === 'member_connect') {
    return actor.page.evaluate(async target => {
      window.member = await window.node.open_member_connection(target);
      return JSON.parse(window.member.session_diagnostics());
    }, args[0]);
  }
  if (op === 'member_call') {
    return actor.page.evaluate(async ([method, values]) => window.member[method](...values), args);
  }
  return actor.page.evaluate(async ([method, values]) => {
    if (['precode_invitation', 'issue_precode'].includes(method)) values = values.map(value => value == null ? value : BigInt(value));
    const result = await window.node[method](...values);
    if (method === 'finish_pairing') { window.node.free(); window.node = result; return JSON.parse(result.status()); }
    if (['status', 'admission_requests', 'admission_material_receipt', 'pairing_status', 'resource_configuration', 'path_diagnostics', 'control_state', 'sync_replica'].includes(method)) return JSON.parse(result);
    return result ?? null;
  }, [op, args]);
}

const input = readline.createInterface({ input: process.stdin });
let closing = false;
input.on('line', line => {
  let command;
  try { command = JSON.parse(line); }
  catch { output({ id: null, ok: false, failureClass: 'InvalidCommand' }); return; }
  dispatch(command).then(result => output({ id: command.id, ok: true, result }), error => {
    // Error text is kept only in the private pipe, never a published receipt.
    output({ id: command.id, ok: false, failureClass: error.name, privateError: String(error) });
  }).finally(() => {
    if (command.op === 'close' && command.actor === 'runner') { closing = true; input.close(); process.stdin.pause(); }
  });
});
input.on('close', async () => {
  if (!closing) { closing = true; await dispatch({ actor: 'runner', op: 'close' }); }
});
output({ event: 'ready', localTechnicalHost: true });
