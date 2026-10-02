import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync, createReadStream, existsSync, readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const browserRoot = join(root, 'browser-probe');
const temporary = mkdtempSync(join(tmpdir(), 'mio50-browser-'));
const children = [];
const events = [];
const caseArgument = process.argv.indexOf('--case');
const selected = process.argv.includes('--suite') ? 'suite' : caseArgument < 0 ? 'first' : process.argv[caseArgument + 1];
const includes = name => selected === 'suite' || selected === name;
assert(['first', 'lifecycle', 'persistent-restart', 'multi-page', 'delivery-interruption', 'storage-clear', 'commit-loss', 'no-relay', 'disabled-relay', 'unavailable-relay', 'tls', 'replica', 'precode', 'management', 'new-code', 'browser-admission', 'replica-faults', 'revocation', 'storage-corrupt', 'browser-precode', 'suite'].includes(selected), 'unknown browser case');
const controlCases=['replica','precode','management','new-code','browser-admission','replica-faults','revocation','storage-corrupt','browser-precode'];
const requests = [];
let browser;

function bounded(promise, label) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), 30000);
  })]).finally(() => clearTimeout(timer));
}
function child(binary, args) {
  const process = spawn(join(root, 'target/debug', binary), args, { stdio: ['pipe', 'pipe', 'pipe'] });
  children.push(process);
  const lines = [];
  const waiters = [];
  const stream = createInterface({ input: process.stdout });
  stream.on('line', line => {
    console.log(`${binary}: ${line.includes('"code":') ? '[private invitation response]' : line}`);
    lines.push(line);
    for (const waiter of [...waiters]) {
      if (waiter.pattern.test(line)) {
        waiters.splice(waiters.indexOf(waiter), 1);
        waiter.resolve(line);
      }
    }
  });
  process.stderr.on('data', data => console.error(`${binary}: ${data}`));
  return { process, wait(pattern, fresh = false) {
    const existing = !fresh && lines.find(line => pattern.test(line));
    return existing ? Promise.resolve(existing) : bounded(new Promise(resolve => waiters.push({ pattern, resolve })), `${binary} ${pattern}`);
  }, command(value, event) {
    const result = this.wait(new RegExp(`"event":"${event}"`), true);
    process.stdin.write(`${value}\n`);
    return result.then(JSON.parse);
  }};
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
  throw new Error('install the pinned Playwright Chromium');
}
async function browserContext() {
  const context = await browser.newContext();
  monitor(context);
  return context;
}
function monitor(context) {
  context.on('request', request => requests.push(request.url()));
  context.on('page', page => page.on('websocket', socket => requests.push(socket.url())));
}
const web = createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const allowed = pathname === '/network.html' || pathname.startsWith('/pkg/');
  const file = resolve(browserRoot, `.${pathname}`);
  if (!allowed || (pathname.startsWith('/pkg/') && !file.startsWith(`${browserRoot}/pkg/`))) return response.writeHead(404).end();
  response.setHeader('Content-Type', pathname.endsWith('.wasm') ? 'application/wasm' : pathname.endsWith('.html') ? 'text/html' : 'text/javascript');
  createReadStream(file).on('error', () => response.writeHead(404).end()).pipe(response);
});

try {
  const tls = includes('tls') || controlCases.includes(selected);
  const cert = join(temporary, 'relay-certificate.der');
  const relay = child('browser-probe-native', tls ? ['relay-tls', cert] : ['relay', '0']);
  const address = (await relay.wait(/^RELAY /)).slice(6);
  const relayUrl = tls ? address : `http://${address}/`;
  const native = child('browser_network_native', [join(temporary, 'admin'), relayUrl, ...(tls ? [cert] : [])]);
  const ready = JSON.parse(await native.wait(/"event":"ready"/));
  await new Promise(resolveListening => web.listen(0, '127.0.0.1', resolveListening));
  const launch = { executablePath: executable(), headless: true, args: ['--no-sandbox', ...(tls ? ['--ignore-certificate-errors'] : [])] };
  const profile = join(temporary, 'browser-profile');
  let context = await chromium.launchPersistentContext(profile, launch);
  browser = context.browser();
  monitor(context);
  let page = await context.newPage();
  page.on('console', message => console.log(`browser: ${message.text()}`));
  page.on('pageerror', error => console.error(`browser: ${error}`));
  const config = { relays: [relayUrl], disabled: [] };
  const url = `http://127.0.0.1:${web.address().port}/network.html?config=${encodeURIComponent(JSON.stringify(config))}&invitation=${ready.invitation}`;
  await page.goto(url);
  await page.waitForFunction(() => window.ready !== undefined, null, { timeout: 5000 });
  const first = await bounded(page.evaluate(() => window.ready), 'browser start');
  assert.notEqual(first, ready.id);
  assert.equal(await page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'AwaitingApproval');
  assert.equal((await native.command('approve', 'approved')).count, 2);
  assert.equal(await page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
  assert.equal(await page.evaluate(() => window.network.complete_admission()), 'Complete');
  assert.equal(await page.evaluate(() => window.network.exchange('browser-controlled-message')), 'browser-controlled-message');
  await page.reload();
  assert.equal(await bounded(page.evaluate(() => window.ready), 'reload'), first);
  assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
  assert.equal(await page.evaluate(() => window.network.exchange('after-refresh')), 'after-refresh');
  events.push({ case: 'first-real-check', browser: first, native: ready.id, stage: 'Complete', count: 2, reload: true, controlledBytes: 26 });

  if (selected === 'replica') {
    const thirdContext = await browserContext();
    const third = await thirdContext.newPage();
    await third.goto(url);
    await bounded(third.evaluate(() => window.ready), 'third browser start');
    assert.equal(await third.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'AwaitingApproval');
    await native.command('approve', 'approved');
    await third.evaluate(inv => window.network.request_admission(inv), ready.invitation);
    await third.evaluate(() => window.network.complete_admission());
    const authored = await native.command('record', 'record');
    const outcome = JSON.parse(await page.evaluate(id => window.network.sync_replica(id), ready.id));
    assert.equal(outcome.complete, true);
    assert(outcome.scope.includes(authored.record));
    await page.reload();
    await bounded(page.evaluate(() => window.ready), 'replica reopen');
    assert.equal(JSON.parse(await page.evaluate(() => window.network.control_state())).records, outcome.scope.length);
    native.process.stdin.write('stop\n');
    assert.equal(await bounded(new Promise(resolve => native.process.once('exit', resolve)), 'original author offline'),0);
    console.log('AUTHOR_OFFLINE exit=0');
    const serving = page.evaluate(() => window.network.serve_replica());
    const copied = JSON.parse(await third.evaluate(id => window.network.sync_replica(id), first));
    await serving;
    assert.equal(copied.complete, true);
    assert.equal(copied.source, first);
    assert(copied.scope.includes(authored.record));
    const original=JSON.parse(await third.evaluate(id=>window.network.control_record(id),authored.record));
    assert.equal(original.body,Buffer.from('original-author browser replica body').toString('hex'));
    await third.reload();
    await bounded(third.evaluate(() => window.ready), 'third durable replica');
    assert.equal(JSON.parse(await third.evaluate(() => window.network.control_state())).records, outcome.scope.length);
    events.push({ case: 'replica-author-offline-browser-reload-third-node', ...copied });
    await thirdContext.close();
  }

  if (selected === 'precode') {
    const { code } = await native.command('issue', 'issued');
    const joiningContext = await browserContext();
    const owner = await joiningContext.newPage();
    await owner.goto(url);
    const identity = await bounded(owner.evaluate(() => window.ready), 'precode owner');
    assert.equal(await owner.evaluate(code => window.network.request_precode(code), code), 'PendingMaterials');
    const competitor = await joiningContext.newPage();
    await competitor.goto(url);
    await assert.rejects(() => competitor.evaluate(() => window.ready), /identity already active/);
    await owner.close();
    await competitor.reload();
    assert.equal(await bounded(competitor.evaluate(() => window.ready), 'precode takeover'), identity);
    assert.equal(await competitor.evaluate(code => window.network.request_precode(code), code), 'PendingMaterials');
    assert.equal(await competitor.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal((await native.command('usage', 'usage')).uses, 1);
    assert.equal((await native.command('status', 'status')).count, 3);
    await competitor.reload();
    await bounded(competitor.evaluate(() => window.ready), 'precode completed reload');
    assert.equal(await competitor.evaluate(code => window.network.request_precode(code), code), 'PendingMaterials');
    await competitor.evaluate(() => window.network.complete_admission());
    const result=await native.command('usage', 'usage');
    assert.equal(result.uses,1); assert.equal(result.count,3);
    events.push({case:'precode-multiple-pages-same-attempt', exclusiveOwner:true, takeover:true, reload:true, uses:result.uses, count:result.count});
    await joiningContext.close();
  }

  if (selected === 'management') {
    const outsiderContext=await browserContext();
    const outsider=await outsiderContext.newPage(); await outsider.goto(url);
    const target=await bounded(outsider.evaluate(()=>window.ready),'management target identity');
    await outsiderContext.close();
    let session=await page.evaluate(()=>window.network.begin_management_session());
    await assert.rejects(()=>page.evaluate(({session,target})=>window.network.manage(session,'add-member',target),{session,target}));
    const credential=readFileSync(join(temporary,'admin','genesis.credential'));
    const rootMaterial={seed:credential.subarray(0,32).toString('hex'),genesis:credential.subarray(32).toString('hex')};
    const elevate=windowMs=>page.evaluate(({session,rootMaterial,windowMs})=>window.network.elevate_genesis(session,rootMaterial.seed,rootMaterial.genesis,windowMs),{session,rootMaterial,windowMs});
    await elevate(60000);
    await page.evaluate(({session,first})=>window.network.manage(session,'appoint-administrator',first),{session,first});
    await page.evaluate(session=>window.network.exit_genesis_elevation(session),session);
    await page.evaluate(({session,target})=>window.network.manage(session,'add-member',target),{session,target});
    await page.evaluate(({session,target})=>window.network.manage(session,'remove-member',target),{session,target});
    for (const action of ['appoint-administrator','demote-administrator','remove-member']) {
      await assert.rejects(()=>page.evaluate(({session,action,target})=>window.network.manage(session,action,target),{session,action,target:ready.id}),/genesis elevation required/);
    }
    await assert.rejects(()=>page.evaluate(session=>window.network.leave(session),session),/genesis elevation required/);
    await elevate(1);
    await new Promise(resolve=>setTimeout(resolve,20));
    await assert.rejects(()=>page.evaluate(({session,target})=>window.network.manage(session,'demote-administrator',target),{session,target:ready.id}),/genesis elevation required/);
    await elevate(60000);
    await page.evaluate(({session,target})=>window.network.manage(session,'demote-administrator',target),{session,target:ready.id});
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).members.find(member=>member.id===ready.id).administrator,false);
    await page.evaluate(({session,target})=>window.network.manage(session,'appoint-administrator',target),{session,target:ready.id});
    await page.evaluate(session=>window.network.end_management_session(session),session);
    await assert.rejects(()=>page.evaluate(({session,target})=>window.network.manage(session,'demote-administrator',target),{session,target:ready.id}),/session ended/);
    await page.reload(); await bounded(page.evaluate(()=>window.ready),'management reopen');
    await assert.rejects(()=>page.evaluate(({session,target})=>window.network.manage(session,'demote-administrator',target),{session,target:ready.id}),/session ended/);
    events.push({case:'browser-management-permission-matrix',ordinaryCannotAdd:true,dailyAddRemove:true,administratorProtected:true,administratorLeaveProtected:true,expiry:true,explicitExit:true,sessionEnd:true,reloadDropsElevation:true});
  }

  if (['new-code','browser-admission'].includes(selected)) {
    const credential=readFileSync(join(temporary,'admin','genesis.credential'));
    await page.evaluate(({seed,genesis,first})=>{
      const session=window.network.begin_management_session();
      window.network.elevate_genesis(session,seed,genesis,60000);
      return window.network.manage(session,'appoint-administrator',first);
    },{seed:credential.subarray(0,32).toString('hex'),genesis:credential.subarray(32).toString('hex'),first});
    const joiningContext=await browserContext(); const joining=await joiningContext.newPage();
    await joining.goto(url); const joiningId=await bounded(joining.evaluate(()=>window.ready),'browser-admin applicant');
    if (selected==='new-code') {
      const original=await native.command('issue','issued');
      assert.equal(await joining.evaluate(code=>window.network.request_precode(code),original.code),'PendingMaterials');
      await joining.reload(); await bounded(joining.evaluate(()=>window.ready),'interrupted original delivery');
      await page.evaluate(id=>window.network.sync_replica(id),ready.id);
      const before=JSON.parse(await page.evaluate(()=>window.network.control_state())).records;
      assert.equal((await native.command('usage','usage')).uses,1);
      native.process.stdin.write('stop\n');
      assert.equal(await bounded(new Promise(resolve=>native.process.once('exit',resolve)),'original delivery unavailable'),0);
      console.log('AUTHOR_OFFLINE exit=0');
      const replacement=await page.evaluate(()=>window.network.issue_precode(BigInt(Math.floor(Date.now()/1000)+3600),1n));
      const serving=page.evaluate(()=>window.network.serve_admission(true));
      assert.equal(await joining.evaluate(code=>window.network.request_precode(code),replacement),'PendingMaterials');
      assert.equal(await joining.evaluate(()=>window.network.complete_admission()),'Complete');
      assert.equal(await serving,'Complete');
      assert.equal(await page.evaluate(code=>window.network.precode_usage(code),replacement),1n);
      assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).records,before);
      const servingReplica=page.evaluate(()=>window.network.serve_replica());
      assert.equal(JSON.parse(await joining.evaluate(id=>window.network.sync_replica(id),first)).complete,true);
      await servingReplica;
      events.push({case:'new-code-other-browser-original-offline',originalUses:1,newUses:1,noDuplicateAdd:true,qualifiedReplica:true});
    } else {
      const invitation=await page.evaluate(()=>window.network.manual_invitation());
      let serving=page.evaluate(()=>window.network.serve_admission(false));
      assert.equal(await joining.evaluate(inv=>window.network.request_admission(inv),invitation),'AwaitingApproval');
      assert.equal(await serving,'AwaitingApproval');
      const request=JSON.parse(await page.evaluate(()=>window.network.admission_requests())).find(request=>request.applicant===joiningId);
      await page.evaluate(request=>window.network.approve_admission(request.applicant,request.request),request);
      serving=page.evaluate(()=>window.network.serve_admission(false));
      await joining.evaluate(inv=>window.network.request_admission(inv),invitation);
      await joining.evaluate(()=>window.network.complete_admission());
      assert.equal(await serving,'Complete');
      events.push({case:'browser-independent-manual-approval',administrator:first,applicant:joiningId,complete:true});
    }
    await joiningContext.close();
  }

  if (selected==='replica-faults') {
    await native.command('prepare-faults','prepared');
    await native.command('fixture-missing-body','fixture');
    await assert.rejects(()=>page.evaluate(id=>window.network.sync_replica(id),ready.id));
    let state=JSON.parse(await page.evaluate(()=>window.network.control_state()));
    assert.equal(state.records,2); assert.equal(state.pending,0);
    await native.command('fixture-child-only','fixture');
    const partial=JSON.parse(await page.evaluate(id=>window.network.sync_replica(id),ready.id));
    assert.equal(partial.transport_done,true); assert.equal(partial.complete,false);
    state=JSON.parse(await page.evaluate(()=>window.network.control_state()));
    assert.equal(state.records,2); assert.equal(state.pending,1);
    await page.reload(); await bounded(page.evaluate(()=>window.ready),'pending dependency reopen');
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).pending,1);
    await native.command('fixture-reversed-duplicates','fixture');
    const completed=JSON.parse(await page.evaluate(id=>window.network.sync_replica(id),ready.id));
    assert.equal(completed.complete,true);
    state=JSON.parse(await page.evaluate(()=>window.network.control_state()));
    assert.equal(state.records,4); assert.equal(state.pending,0);
    await native.command('fixture-normal','fixture');
    await page.evaluate(id=>window.network.sync_replica(id),ready.id);
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).records,4);
    await native.command('record','record');
    await page.evaluate(()=>{
      const put=IDBObjectStore.prototype.put;
      IDBObjectStore.prototype.put=function(...args) { this.transaction.abort(); return put.apply(this,args); };
    });
    await assert.rejects(()=>page.evaluate(id=>window.network.sync_replica(id),ready.id),/commit failed|storage changed/);
    await page.reload(); await bounded(page.evaluate(()=>window.ready),'failed replica commit reopen');
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).records,4);
    assert.equal(JSON.parse(await page.evaluate(id=>window.network.sync_replica(id),ready.id)).complete,true);
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.control_state())).records,5);
    events.push({case:'browser-replica-failure-boundaries',missingBodyRejected:true,missingDependencyIncomplete:true,pendingSurvivesReload:true,reversedDuplicatesAppliedOnce:true,realIdbAbortNoCompletion:true,retryComplete:true});
  }

  if (selected==='revocation') {
    await page.evaluate(async id=>{window.member=await window.network.open_member_connection(id);},ready.id);
    assert.equal(await page.evaluate(()=>window.member.exchange('before-revocation')),'before-revocation');
    assert.equal(await page.evaluate(()=>window.member.is_closed()),false);
    const removal=await native.command(`remove-record ${first}`,'removal-record');
    await page.evaluate(header=>window.network.ingest_management_record(header,undefined),removal.header);
    assert.equal(await page.evaluate(()=>window.member.is_closed()),true);
    await assert.rejects(()=>page.evaluate(()=>window.member.exchange('after-revocation')));
    await assert.rejects(()=>page.evaluate(id=>window.network.open_member_connection(id),ready.id));
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.status())).stage,'Removed');
    await page.reload();await bounded(page.evaluate(()=>window.ready),'revoked reopen');
    assert.equal(JSON.parse(await page.evaluate(()=>window.network.status())).stage,'Removed');
    events.push({case:'browser-local-revocation-closes-real-connection',existingClosed:true,newRejected:true,reloadRemainsRemoved:true});
  }
  if (selected==='storage-corrupt') {
    await page.evaluate(()=>new Promise((resolve,reject)=>{
      const request=indexedDB.open('miopunch-browser-network',1);
      request.onsuccess=()=>{const db=request.result;const tx=db.transaction('node','readwrite',{durability:'strict'});tx.objectStore('node').put('invalid persisted snapshot','state');tx.oncomplete=()=>{db.close();resolve();};tx.onabort=()=>reject(new Error('fixture corruption failed'));};
    }));
    await page.reload(); await assert.rejects(()=>page.evaluate(()=>window.ready),/materials corrupt/);
    await page.reload(); await assert.rejects(()=>page.evaluate(()=>window.ready),/materials corrupt/);
    events.push({case:'browser-corrupt-restore-fails-closed',explicitFailure:true,noSilentNewIdentity:true});
  }

  if (selected==='browser-precode') {
    const credential=readFileSync(join(temporary,'admin','genesis.credential'));
    let session=await page.evaluate(()=>window.network.begin_management_session());
    await page.evaluate(({session,seed,genesis})=>window.network.elevate_genesis(session,seed,genesis,60000),{session,seed:credential.subarray(0,32).toString('hex'),genesis:credential.subarray(32).toString('hex')});
    await page.evaluate(({session,first})=>window.network.manage(session,'appoint-administrator',first),{session,first});
    const code=await page.evaluate(()=>window.network.issue_precode(BigInt(Math.floor(Date.now()/1000)+3600),1n));
    const joiningContext=await browserContext();const joining=await joiningContext.newPage();
    await joining.goto(url);await bounded(joining.evaluate(()=>window.ready),'browser-code joining');
    let serving=page.evaluate(()=>window.network.serve_admission(true));
    await joining.evaluate(code=>window.network.request_precode(code),code);
    await joining.evaluate(()=>window.network.complete_admission());await serving;
    await page.reload();await bounded(page.evaluate(()=>window.ready),'issuer ledger reopen');
    assert.equal(await page.evaluate(code=>window.network.precode_usage(code),code),1n);
    serving=page.evaluate(()=>window.network.serve_admission(true));
    await joining.evaluate(code=>window.network.request_precode(code),code);
    await joining.evaluate(()=>window.network.complete_admission());await serving;
    assert.equal(await page.evaluate(code=>window.network.precode_usage(code),code),1n);
    const oldCode=await page.evaluate(()=>window.network.issue_precode(BigInt(Math.floor(Date.now()/1000)+3600),1n));
    session=await page.evaluate(()=>window.network.begin_management_session());
    await page.evaluate(({session,seed,genesis})=>window.network.elevate_genesis(session,seed,genesis,60000),{session,seed:credential.subarray(0,32).toString('hex'),genesis:credential.subarray(32).toString('hex')});
    await page.evaluate(({session,first})=>window.network.manage(session,'demote-administrator',first),{session,first});
    await page.evaluate(({session,first})=>window.network.manage(session,'appoint-administrator',first),{session,first});
    const otherContext=await browserContext();const other=await otherContext.newPage();
    await other.goto(url);await bounded(other.evaluate(()=>window.ready),'old-code applicant');
    serving=page.evaluate(()=>window.network.serve_admission(true)).then(()=>false,()=>true);
    await assert.rejects(()=>other.evaluate(code=>window.network.request_precode(code),oldCode));
    assert.equal(await serving,true);
    assert.equal(await page.evaluate(code=>window.network.precode_usage(code),oldCode),0n);
    const fresh=await page.evaluate(()=>window.network.issue_precode(BigInt(Math.floor(Date.now()/1000)+3600),1n));
    serving=page.evaluate(()=>window.network.serve_admission(true));
    await other.evaluate(code=>window.network.request_precode(code),fresh);await other.evaluate(()=>window.network.complete_admission());await serving;
    assert.equal(await page.evaluate(code=>window.network.precode_usage(code),fresh),1n);
    events.push({case:'browser-precode-durable-ledger-and-authority-episode',reopenUses:1,retryUses:1,oldCodeAfterDemoteReappointUses:0,newCodeUses:1});
    await otherContext.close();await joiningContext.close();
  }

  if (includes('lifecycle')) {
    const cdp = await context.newCDPSession(page);
    await cdp.send('Page.setWebLifecycleState', { state: 'frozen' });
    await new Promise(resolve => setTimeout(resolve, 1000));
    await cdp.send('Page.setWebLifecycleState', { state: 'active' });
    assert.equal(await page.evaluate(() => window.network.exchange('after-pause')), 'after-pause');
    await page.close();
    page = await context.newPage();
    await page.goto(url);
    assert.equal(await bounded(page.evaluate(() => window.ready), 'normal reopen'), first);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
    assert.equal(await page.evaluate(() => window.network.exchange('after-reopen')), 'after-reopen');
    const status = await native.command('status', 'status');
    assert.equal(status.count, 2, 'pause and close must neither Remove nor Add');
    assert.equal(status.requests, 1);
    assert(status.members.includes(first));
    events.push({ case: 'priority-pause-reopen', frozen: true, sameIdentity: true, count: status.count, requestCount: status.requests, controlledBytes: 12 });
  }

  if (includes('persistent-restart')) {
    await context.close();
    context = await chromium.launchPersistentContext(profile, launch);
    browser = context.browser();
    monitor(context);
    page = await context.newPage();
    const restoredUrl = new URL(url);
    restoredUrl.searchParams.set('config', '{}');
    await page.goto(restoredUrl.toString());
    assert.equal(await bounded(page.evaluate(() => window.ready), 'actual browser process restart'), first);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
    assert.equal(await page.evaluate(() => window.network.exchange('after-browser-process-restart')), 'after-browser-process-restart');
    const status = await native.command('status', 'status');
    assert.equal(status.count, 2);
    assert.equal(status.requests, 1);
    events.push({ case: 'persistent-browser-process-restart', sameIdentity: true, bothMaterialsSaved: true, savedRelayChoiceRestored: true, count: 2, requestCount: 1 });
  }

  if (includes('multi-page')) {
    const competing = await context.newPage();
    await competing.goto(url);
    await assert.rejects(() => competing.evaluate(() => window.ready), /identity already active/);
    assert.equal((await native.command('status', 'status')).count, 2);
    await page.close();
    page = competing;
    await page.reload();
    assert.equal(await bounded(page.evaluate(() => window.ready), 'lease takeover after close'), first);
    assert.equal(await page.evaluate(() => window.network.exchange('after-page-takeover')), 'after-page-takeover');
    const status = await native.command('status', 'status');
    assert.equal(status.count, 2);
    assert.equal(status.requests, 1);
    events.push({ case: 'same-identity-pages', competingPageRejected: true, takeoverIdentity: first, count: 2, requests: 1 });
  }

  if (includes('delivery-interruption')) {
    const interruptedContext = await browserContext();
    let interrupted = await interruptedContext.newPage();
    await interrupted.goto(url);
    const applicant = await bounded(interrupted.evaluate(() => window.ready), 'interrupted applicant start');
    await assert.rejects(() => interrupted.evaluate(() => window.network.exchange('early-before-approval')), /materials incomplete/);
    assert.equal(await interrupted.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'AwaitingApproval');
    const approved = await native.command('approve', 'approved');
    assert.equal(approved.count, 3);
    assert.equal(await interrupted.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
    const before = JSON.parse(await interrupted.evaluate(() => window.network.status()));
    assert.equal(before.authorization_saved, true);
    assert.equal(before.secret_saved, false);
    await assert.rejects(() => interrupted.evaluate(() => window.network.exchange('early-after-authorization')), /materials incomplete/);
    await interrupted.close();
    interrupted = await interruptedContext.newPage();
    await interrupted.goto(url);
    assert.equal(await bounded(interrupted.evaluate(() => window.ready), 'interrupted applicant reopen'), applicant);
    assert.equal(JSON.parse(await interrupted.evaluate(() => window.network.status())).stage, 'PendingMaterials');
    assert.equal(await interrupted.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
    assert.equal(await interrupted.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal(await interrupted.evaluate(() => window.network.exchange('after-delivery-resume')), 'after-delivery-resume');
    assert.equal((await native.command('status', 'status')).count, 3, 'delivery retries reuse the original Add');
    await interruptedContext.close();
    events.push({ case: 'delivery-interruption', authorizationSaved: true, secretInitiallySaved: false, sameIdentity: true, resumed: true, earlyDataRejected: true, count: 3 });
  }

  if (includes('storage-clear')) {
    const countBefore = (await native.command('status', 'status')).count;
    const cdp = await context.newCDPSession(page);
    await cdp.send('Storage.clearDataForOrigin', { origin: new URL(url).origin, storageTypes: 'indexeddb' });
    await assert.rejects(() => page.evaluate(() => window.network.exchange('after-material-clear')), /materials cleared or changed/);
    await page.close();
    page = await context.newPage();
    await page.goto(url);
    const replacement = await bounded(page.evaluate(() => window.ready), 'cleared browser restart');
    assert.notEqual(replacement, first, 'cleared identity must not impersonate its predecessor');
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'AwaitingApproval');
    await assert.rejects(() => page.evaluate(() => window.network.exchange('cleared-early-data')), /materials incomplete/);
    assert.equal((await native.command('status', 'status')).count, countBefore, 'storage clearing does not author Leave or Add');
    assert.equal(await page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'AwaitingApproval');
    assert.equal((await native.command('approve', 'approved')).count, countBefore + 1);
    assert.equal(await page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
    assert.equal(await page.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal(await page.evaluate(() => window.network.exchange('after-explicit-readmission')), 'after-explicit-readmission');
    events.push({ case: 'material-clear', oldIdentity: first, newIdentity: replacement, staleMemoryRejected: true, explicitNewApproval: true, count: countBefore + 1 });
  }

  if (includes('commit-loss')) {
    const failingContext = await browserContext();
    let failingPage = await failingContext.newPage();
    await failingPage.goto(url);
    const pendingIdentity = await bounded(failingPage.evaluate(() => window.ready), 'commit-loss browser start');
    assert.equal(await failingPage.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'AwaitingApproval');
    const count = (await native.command('approve', 'approved')).count;
    assert.equal(await failingPage.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
    const cdp = await failingContext.newCDPSession(failingPage);
    await cdp.send('Storage.clearDataForOrigin', { origin: new URL(url).origin, storageTypes: 'indexeddb' });
    await assert.rejects(() => failingPage.evaluate(() => window.network.complete_admission()), /commit failed or storage changed/);
    assert.equal(JSON.parse(await failingPage.evaluate(() => window.network.status())).stage, 'PendingMaterials', 'failed secret save cannot report Complete');
    await assert.rejects(() => failingPage.evaluate(() => window.network.exchange('after-failed-save')), /materials incomplete/);
    assert.equal((await native.command('status', 'status')).count, count);
    await failingPage.close();
    failingPage = await failingContext.newPage();
    await failingPage.goto(url);
    assert.notEqual(await bounded(failingPage.evaluate(() => window.ready), 'commit-loss browser reopen'), pendingIdentity);
    assert.equal(JSON.parse(await failingPage.evaluate(() => window.network.status())).stage, 'AwaitingApproval');
    await failingContext.close();
    events.push({ case: 'secret-commit-loss', failedSaveRejected: true, neverComplete: true, noDuplicateAdd: true, count });
  }

  for (const resourceCase of ['no-relay', 'disabled-relay'].filter(includes)) {
    const before = await native.command('status', 'status');
    const identity = await page.evaluate(() => window.network.id());
    await page.close();
    page = await context.newPage();
    const resourceRequests = [];
    page.on('request', request => resourceRequests.push(request.url()));
    page.on('websocket', socket => resourceRequests.push(socket.url()));
    const limited = resourceCase === 'disabled-relay' ? { relays: [relayUrl], disabled: [`${relayUrl}excluded-route?candidate=forbidden`] } : { relays: [], disabled: [] };
    const limitedUrl = new URL(url);
    limitedUrl.searchParams.set('config', JSON.stringify(limited));
    await page.goto(limitedUrl.toString());
    await assert.rejects(() => page.evaluate(() => window.ready), /temporarily unreachable: no allowed browser relay/);
    limitedUrl.searchParams.set('config', '{}');
    await page.goto(limitedUrl.toString());
    await assert.rejects(() => page.evaluate(() => window.ready), /temporarily unreachable: no allowed browser relay/);
    assert(resourceRequests.every(value => value.startsWith(new URL(url).origin)), 'disabled/empty resources must make no external facility access');
    await page.goto(url);
    assert.equal(await bounded(page.evaluate(() => window.ready), 'explicit resource import'), identity);
    assert.equal(await page.evaluate(() => window.network.exchange('after-resource-import')), 'after-resource-import');
    const after = await native.command('status', 'status');
    assert.equal(after.count, before.count);
    assert.equal(after.requests, before.requests);
    events.push({ case: resourceCase === 'disabled-relay' ? 'disabled-relay' : 'no-allowed-relay', forbiddenFacilityAccesses: 0, persistedChoice: true, explicitImportRestored: true, count: after.count });
  }

  if (includes('unavailable-relay')) {
    const failingContext = await browserContext();
    const failingPage = await failingContext.newPage();
    const failureRequests = [];
    failingPage.on('request', request => failureRequests.push(request.url()));
    failingPage.on('websocket', socket => failureRequests.push(socket.url()));
    const unavailable = new URL(url);
    unavailable.searchParams.set('config', JSON.stringify({ relays: [`${new URL(url).origin}/`], disabled: [] }));
    await failingPage.goto(unavailable.toString());
    await assert.rejects(() => bounded(failingPage.evaluate(() => window.ready), 'allowed unavailable relay'), /temporarily unreachable: allowed browser relays unavailable/);
    assert(failureRequests.every(value => value.startsWith(new URL(url).origin) || value.startsWith(new URL(url).origin.replace(/^http/, 'ws'))), 'unavailable relay must not enable a public fallback');
    assert(failureRequests.some(value => value.startsWith(new URL(url).origin.replace(/^http/, 'ws'))), 'must actually attempt the allowed unavailable relay');
    await failingContext.close();
    events.push({ case: 'unavailable-relay', actualAllowedAttempt: true, publicFallbackAccesses: 0, explicitTemporarilyUnreachable: true });
  }
  const websocket = relayUrl.replace(/^http/, 'ws');
  assert(requests.every(value => value.startsWith(`http://127.0.0.1:${web.address().port}/`) || value.startsWith(`ws://127.0.0.1:${web.address().port}/`) || value.startsWith(relayUrl) || value.startsWith(websocket)), `unexpected external resource ${requests}`);
  assert(requests.some(value => value.startsWith(websocket)));
  if (tls) events.push({ case: 'tls-websocket-relay', relayUrl, websocket, nativeTrustedTestCa: true, isolatedChromiumTestCertificateException: true });
  console.log(`RESULT ${JSON.stringify(events)}`);
  console.log(`PASS ${selected}: real browser admission and selected boundary checks`);
} catch (error) {
  console.error(`CHECK_FAILED ${error.stack}`);
  throw error;
} finally {
  await browser?.close();
  await new Promise(resolveClose => web.close(resolveClose));
  for (const process of children.reverse()) {
    if (process.exitCode !== null || process.signalCode !== null) continue;
    const exited = new Promise(resolveExit => process.on('exit', (code, signal) => resolveExit({ code, signal })));
    process.stdin.end();
    process.kill('SIGINT');
    const timer = setTimeout(() => process.kill('SIGKILL'), 20000);
    const result = await exited;
    clearTimeout(timer);
    console.log(`CLEANUP ${JSON.stringify(result)}`);
  }
  rmSync(temporary, { recursive: true, force: true });
}
