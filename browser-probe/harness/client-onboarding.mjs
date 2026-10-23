// Technical host only: independent real Chromium profiles and WASM nodes.
// Stdout is a private controller pipe; the controller redacts secret returns.
import { chromium } from 'playwright-core';
import http from 'node:http';
import readline from 'node:readline';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';

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
    return actor.page.evaluate(async caller => {
      const attempt = window.node.member_accept_attempt(caller ?? 12000);
      try { window.member = await attempt.accept(); return JSON.parse(window.member.session_diagnostics()); }
      finally { attempt.free(); }
    }, args[0]);
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
  if (op === 'data_open' || op === 'data_accept') {
    return actor.page.evaluate(async operation => {
      window.dataStream = await window.member[operation === 'data_open' ? 'open_data_stream' : 'accept_data_stream']();
      return true;
    }, op);
  }
  if (op === 'data_write') {
    return actor.page.evaluate(async ([size, value]) => {
      await window.dataStream.write_chunk(new Uint8Array(size).fill(value));
      return { written: size };
    }, args);
  }
  if (op === 'data_read') {
    return actor.page.evaluate(async value => {
      const bytes = await window.dataStream.read_chunk();
      return { length: bytes.length, matches: bytes.every(byte => byte === value) };
    }, args[0]);
  }
  if (op === 'data_cancel') {
    return actor.page.evaluate(() => { window.dataStream.cancel(); return true; });
  }
  if (op === 'data_drop') {
    return actor.page.evaluate(() => { window.dataStream.free(); delete window.dataStream; return true; });
  }
  if (op === 'file_api') {
    return actor.page.evaluate(() => ({
      send: typeof window.node.send_file === 'function',
      receive: typeof window.node.enable_files === 'function',
      stream: typeof window.api.BrowserDataStream === 'function',
      opfs: typeof navigator.storage?.getDirectory === 'function',
    }));
  }
  if (op === 'file_make') {
    return actor.page.evaluate(([name, size]) => {
      const chunk = new Uint8Array(65536);
      for (let i = 0; i < chunk.length; i++) chunk[i] = i % 251;
      const parts = [];
      for (let left = size; left > 0; left -= chunk.length)
        parts.push(chunk.subarray(0, Math.min(chunk.length, left)));
      const file = new File(parts, name, { type: 'application/octet-stream' });
      window.sourceFiles ??= new Map();
      window.sourceFiles.set(name, file);
      return { name: file.name, size: file.size };
    }, args);
  }
  if (op === 'file_send_source') {
    return actor.page.evaluate(([target, name]) => window.node.send_file(target, window.sourceFiles.get(name)), args);
  }
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
  if (op === 'file_list' || op === 'file_remove') {
    return actor.page.evaluate(async ([operation, args]) => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle('miopunch-files-' + window.node.id());
      if (operation === 'file_remove') { await dir.removeEntry(args[0]); return true; }
      const names = [];
      for await (const name of dir.keys()) names.push(name);
      return names.sort();
    }, [op, args]);
  }
  if (op === 'file_hold_save' || op === 'file_abort_save') {
    return actor.page.evaluate(operation => {
      const prototype = FileSystemWritableFileStream.prototype;
      const original = prototype.close;
      prototype.close = async function () {
        prototype.close = original;
        if (operation === 'file_abort_save') {
          await this.abort();
          throw new DOMException('owned OPFS write aborted before commit', 'AbortError');
        }
        await new Promise(resolve => { window.releaseFileSave = resolve; });
        return original.call(this);
      };
      return true;
    }, op);
  }
  if (op === 'file_release_save') {
    return actor.page.evaluate(() => { window.releaseFileSave(); delete window.releaseFileSave; return true; });
  }
  if (op === 'file_hold_abort') {
    return actor.page.evaluate(() => {
      const prototype = FileSystemWritableFileStream.prototype;
      const original = prototype.abort;
      prototype.abort = async function (...values) {
        prototype.abort = original;
        await new Promise(resolve => { window.releaseFileAbort = resolve; });
        return original.apply(this, values);
      };
      return true;
    });
  }
  if (op === 'file_release_abort') {
    return actor.page.evaluate(() => { window.releaseFileAbort(); delete window.releaseFileAbort; return true; });
  }
  if (op === 'file_abort_pending') {
    return actor.page.evaluate(() => typeof window.releaseFileAbort === 'function');
  }
  if (op === 'file_hold_publish') {
    return actor.page.evaluate(() => {
      const prototype = FileSystemFileHandle.prototype;
      const original = prototype.move;
      if (typeof original !== 'function') throw new Error('actual OPFS move prerequisite unavailable');
      prototype.move = async function (...values) {
        prototype.move = original;
        const result = await original.apply(this, values);
        await new Promise(resolve => { window.releaseFilePublish = resolve; });
        return result;
      };
      return true;
    });
  }
  if (op === 'file_publish_pending') {
    return actor.page.evaluate(() => typeof window.releaseFilePublish === 'function');
  }
  if (op === 'file_release_publish') {
    return actor.page.evaluate(() => { window.releaseFilePublish(); delete window.releaseFilePublish; return true; });
  }
  return actor.page.evaluate(async ([method, values]) => {
    if (['precode_invitation', 'issue_precode'].includes(method)) values = values.map(value => value == null ? value : BigInt(value));
    const result = await window.node[method](...values);
    if (method === 'finish_pairing') { window.node.free(); window.node = result; return JSON.parse(result.status()); }
    if (['status', 'admission_requests', 'admission_material_receipt', 'pairing_status', 'resource_configuration', 'path_diagnostics', 'control_state', 'sync_replica', 'file_status'].includes(method)) return JSON.parse(result);
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
