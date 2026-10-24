// Minimal technical host for ordinary public service/guest/file operations.
// Each actor has its own real Chromium profile, WASM identity and storage.
import { chromium } from 'playwright-core';
import http from 'node:http';
import readline from 'node:readline';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';

const [packagePath, executablePath, privateDirectory] = process.argv.slice(2);
const packageRoot = path.resolve(packagePath);
const entry = (await readdir(packageRoot)).find(name => name.endsWith('.js'));
if (!entry) throw new Error('missing immutable package entry');
const actors = new Map();
const output = value => process.stdout.write(JSON.stringify(value, (_, item) => typeof item === 'bigint' ? Number(item) : item) + '\n');
const server = http.createServer(async (request, response) => {
  try {
    if (request.url === '/') {
      response.writeHead(200, { 'Content-Type': 'text/html' });
      response.end('<!doctype html><title>miopunch technical device host</title>');
      return;
    }
    const file = path.resolve(packageRoot, '.' + new URL(request.url, 'http://localhost').pathname);
    if (!file.startsWith(packageRoot + path.sep)) throw new Error('outside owned package');
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
}

async function dispatch({ actor: name, op, args = [] }) {
  if (op === 'close' && name === 'runner') {
    const cleanup = [];
    for (const [name, actor] of actors) {
      let closed = true;
      try { await actor.page.evaluate(async () => { await window.node?.close(); }); } catch { closed = false; }
      try { await actor.context.close(); } catch { closed = false; }
      cleanup.push({ actor: name, closed });
    }
    actors.clear(); await new Promise(resolve => server.close(resolve)); return { cleanup };
  }
  if (op === 'open') {
    if (actors.has(name)) throw new Error('actor already exists');
    const context = await chromium.launchPersistentContext(path.join(privateDirectory, name), {
      executablePath, headless: true, args: ['--no-sandbox'], ignoreHTTPSErrors: false,
    });
    const actor = { context, page: context.pages()[0] ?? await context.newPage() };
    actors.set(name, actor); await load(actor);
    return { browserVersion: context.browser().version(), normalPublicTrust: true };
  }
  const actor = actors.get(name);
  if (!actor) throw new Error('unknown actor');
  if (op === 'reload') {
    await actor.page.evaluate(async () => { await window.node?.close(); });
    await load(actor); return true;
  }
  if (op === 'api') return actor.page.evaluate(() => Object.fromEntries(['create', 'join', 'pair'].map(name => [name, typeof window.api.BrowserNetwork[name] === 'function'])));
  if (op === 'start') return actor.page.evaluate(async ([action, config, invitation]) => {
    window.node = await window.api.BrowserNetwork[action](config, invitation);
    return JSON.parse(window.node.status());
  }, args);
  if (op === 'start_recovery') return actor.page.evaluate(async values => {
    window.node = await window.api.BrowserNetwork.prepare_management_recovery(...values);
    return JSON.parse(window.node.status());
  }, args);
  if (op === 'start_replacement') return actor.page.evaluate(async values => {
    window.node = await window.api.BrowserNetwork.restore_node_replacement(...values);
    return JSON.parse(window.node.status());
  }, args);
  if (op === 'static_call') return actor.page.evaluate(async ([method, ...values]) => {
    const result = await window.api.BrowserNetwork[method](...values);
    if (['finalize_node_replacement'].includes(method)) return JSON.parse(result);
    return result ?? null;
  }, args);
  if (op === 'close_node') return actor.page.evaluate(async () => { await window.node.close(); return {closed:true}; });
  if (op === 'file_make') return actor.page.evaluate(([name, size]) => {
    const chunk = new Uint8Array(65536);
    for (let i = 0; i < chunk.length; i++) chunk[i] = i % 251;
    const parts = [];
    for (let left = size; left > 0; left -= chunk.length) parts.push(chunk.subarray(0, Math.min(chunk.length, left)));
    const file = new File(parts, name, { type: 'application/octet-stream' });
    window.sourceFiles ??= new Map(); window.sourceFiles.set(name, file);
    return { name: file.name, size: file.size };
  }, args);
  if (op === 'file_send_source') return actor.page.evaluate(([target, name]) => window.node.send_file(target, window.sourceFiles.get(name)), args);
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
    return { size, sha256: hash.digest('hex') };
  }
  if (op === 'file_list' || op === 'file_remove') return actor.page.evaluate(async ([op, args]) => {
    const root = await navigator.storage.getDirectory();
    let directory;
    try { directory = await root.getDirectoryHandle('miopunch-files-' + window.node.id()); }
    catch (error) { if (op === 'file_list' && error.name === 'NotFoundError') return []; throw error; }
    if (op === 'file_remove') { await directory.removeEntry(args[0]); return true; }
    const names = []; for await (const name of directory.keys()) names.push(name); return names.sort();
  }, [op, args]);
  return actor.page.evaluate(async ([method, values]) => {
    if (method === 'issue_guest') values[1] = BigInt(values[1]);
    if (['issue_precode', 'precode_invitation'].includes(method)) values = values.map(value => value == null ? value : BigInt(value));
    const result = await window.node[method](...values);
    if (['status', 'admission_requests', 'file_status', 'service_status', 'guest_status', 'sync_services', 'redeem_guest', 'end_guest', 'propose_device_operation', 'device_operations', 'confirm_device_operation', 'confirm_device_operation_offline', 'execute_device_operation', 'request_device_confirmation', 'collect_device_result', 'credential_custody', 'propose_credential_custody', 'accept_credential_custody', 'transfer_credential_custody', 'check_other_credential_custodians', 'stop_credential_custody', 'management_recovery_status', 'restore_management_authority', 'node_replacement_status', 'complete_node_replacement'].includes(method)) return JSON.parse(result);
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
    output({ id: command.id, ok: false, failureClass: error.name, privateError: String(error) });
  }).finally(() => {
    if (command.op === 'close' && command.actor === 'runner') { closing = true; input.close(); process.stdin.pause(); }
  });
});
input.on('close', async () => { if (!closing) { closing = true; await dispatch({ actor: 'runner', op: 'close' }); } });
output({ event: 'ready', localTechnicalHost: true });
