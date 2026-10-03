// Concrete owned IRC fixture utilities shared by the independent carrier,
// restricted-budget and mixed-retry entry points. No network work on import.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, readdirSync, writeFileSync } from 'node:fs';
import { createServer as httpServer } from 'node:http';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';
export function bounded(promise, label, milliseconds = 15000) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} deadline`)), milliseconds);
  })]).finally(() => clearTimeout(timer));
}
function executable() {
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE) return process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
  if (existsSync(chromium.executablePath())) return chromium.executablePath();
  for (const name of readdirSync(join(homedir(), '.cache/ms-playwright')).filter(name => name.startsWith('chromium-')).sort().reverse()) {
    for (const suffix of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const candidate = join(homedir(), '.cache/ms-playwright', name, suffix);
      if (existsSync(candidate)) return candidate;
    }
  }
  throw new Error('pinned Chromium unavailable');
}
function quote(value) { return `'${value.replaceAll("'", "'\\''")}'`; }

export function createOwnedIrcFixtureTools({ root, fixture, report, nativeProcesses, servers }) {
async function listen(server) {
  servers.push(server);
  await new Promise((resolveListening, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolveListening);
  });
  return server.address().port;
}
function nativeFixture(config, name, restricted = false) {
  const configPath = join(fixture, `native-${name}.json`);
  writeFileSync(configPath, JSON.stringify(config));
  const child = spawn('rtk', ['proxy', join(root, restricted ? 'target/debug/browser_irc_restricted_native' : 'target/debug/browser_irc_datagram'), configPath, join(fixture, 'ca.pem'), ...(restricted ? [join(fixture, `node-${name}`)] : [])], { stdio: ['pipe', 'pipe', 'pipe'] });
  nativeProcesses.add(child);
  const entries = [];
  const pending = [];
  let failure;
  createInterface({ input: child.stdout }).on('line', line => {
    const value = JSON.parse(line);
    report.native.push({ name, ...value });
    entries.push(value);
    for (const waiter of [...pending]) {
      const index = entries.findIndex(waiter.matches);
      if (index >= 0) { pending.splice(pending.indexOf(waiter), 1); waiter.resolve(entries.splice(index, 1)[0]); }
    }
  });
  child.stderr.on('data', bytes => console.error(`native ${name}: ${bytes}`));
  child.on('error', error => { failure = error; for (const waiter of pending.splice(0)) waiter.reject(error); });
  child.once('exit', (code, signal) => {
    nativeProcesses.delete(child);
    failure = new Error(`native ${name} exited ${code}/${signal}`);
    for (const waiter of pending.splice(0)) waiter.reject(failure);
  });
  let id = 0;
  function wait(event, match = () => true) {
    const matches = value => value.event === event && match(value);
    const index = entries.findIndex(matches);
    if (index >= 0) return Promise.resolve(entries.splice(index, 1)[0]);
    if (failure) return Promise.reject(failure);
    let waiter;
    const promise = new Promise((resolveValue, reject) => { waiter = { matches, resolve: resolveValue, reject }; pending.push(waiter); });
    return bounded(promise, `native ${name} ${event}`).finally(() => {
      const index = pending.indexOf(waiter); if (index >= 0) pending.splice(index, 1);
    });
  }
  return { child, wait, command(command, event) {
    const request = ++id;
    const result = wait(event, value => restricted || value.id === request || event === 'status');
    child.stdin.write(`${restricted ? command.command : JSON.stringify({ ...command, id: request })}\n`);
    return result;
  }, async stop() {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = new Promise(resolveExit => child.once('exit', (code, signal) => resolveExit({ code, signal })));
    child.stdin.end(`${restricted ? 'stop' : JSON.stringify({ command: 'stop' })}\n`);
    assert.equal((await bounded(exited, `native ${name} cleanup`, 3000)).code, 0);
  } };
}


 return { listen, nativeFixture };
}
export function wasmServer(root) {
 return httpServer((request,response)=>{
  if(request.url==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><meta charset="utf-8"><title>Owned IRC qualification</title>');return;}
  const path=`browser-probe${request.url}`;
  if(!path.startsWith('browser-probe/pkg/')||path.includes('..')){response.writeHead(404).end();return;}
  if(path.endsWith('.wasm'))response.setHeader('Content-Type','application/wasm');if(path.endsWith('.js'))response.setHeader('Content-Type','text/javascript');
  createReadStream(join(root,path)).on('error',()=>response.writeHead(404).end()).pipe(response);
 });
}
export function privateBrowserLaunch(fixture) {
 const legacyDb=join(homedir(),'.pki/nssdb');const defaultDb=join(process.env.XDG_DATA_HOME??join(homedir(),'.local/share'),'pki/nssdb');const nssTarget=existsSync(legacyDb)?legacyDb:defaultDb;assert(existsSync(nssTarget));
 const wrapper=join(fixture,'chromium-private-trust.sh');writeFileSync(wrapper,`#!/bin/sh\nexec rtk proxy bwrap --ro-bind / / --tmpfs /tmp --bind ${quote(fixture)} ${quote(fixture)} --bind ${quote(join(fixture,'nssdb'))} ${quote(nssTarget)} --dev-bind /dev /dev --proc /proc -- ${quote(executable())} "$@"\n`,{mode:0o700});
 return {executablePath:wrapper,headless:true,args:['--no-sandbox'],ignoreHTTPSErrors:false,privateNssMount:true,certificateBypassFlags:[]};
}
