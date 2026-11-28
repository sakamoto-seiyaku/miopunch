import { artifactPath } from './artifact-path.mjs';
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
const mid1Cases = ['mid1-repair', 'mid1-repair-backup', 'mid1-repair-drop'];
const mid1Repair = mid1Cases.includes(selected);
assert(['first', 'lifecycle', 'persistent-restart', 'multi-page', 'delivery-interruption', 'storage-clear', 'commit-loss', 'no-relay', 'disabled-relay', 'unavailable-relay', 'tls', 'replica', 'precode', 'management', 'new-code', 'browser-admission', 'replica-faults', 'revocation', 'storage-corrupt', 'browser-precode', 'maintenance-concurrency', 'paged-history', 'paged-migration', 'paged-recovery', 'admission-commit-cancel', 'migration-material-commit', ...mid1Cases, 'suite'].includes(selected), 'unknown browser case');
const controlCases=['paged-recovery','paged-history','replica','precode','management','new-code','browser-admission','replica-faults','revocation','storage-corrupt','browser-precode'];
const requests = [];
let browser;

function bounded(promise, label) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), 30000);
  })]).finally(() => clearTimeout(timer));
}
function child(binary, args) {
  const process = spawn(artifactPath(join(root, 'target/debug', binary)), args, { stdio: ['pipe', 'pipe', 'pipe'] });
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
// Delay only the notification of a real completed IndexedDB transaction.
function installAdmissionCommitDelay({ inactive = false, stage = 'invitation' } = {}) {
  const put = IDBObjectStore.prototype.put;
  const completion = Object.getOwnPropertyDescriptor(IDBTransaction.prototype, 'oncomplete');
  const delayed = new WeakMap();
  const fixture = window.admissionCommitDelay = { armed: stage, completed: [], delayMs: 5000 };
  Object.defineProperty(IDBTransaction.prototype, 'oncomplete', { ...completion,
    set(callback) {
      completion.set.call(this, typeof callback === 'function' ? function(event) {
        const stage = delayed.get(this);
        if (stage) setTimeout(() => {
          callback.call(this, event);
          fixture.completed.push(stage);
        }, fixture.delayMs);
        else callback.call(this, event);
      } : callback);
    },
  });
  IDBObjectStore.prototype.put = function(value, key) {
    if (this.name === 'node' && key === 'state' && typeof value === 'string' && fixture.armed) {
      const snapshot = JSON.parse(value);
      const saved = inactive ? snapshot.migration?.inactive : snapshot;
      if (!saved) return put.call(this, value, key);
      const stage = saved.secret != null ? 'secret' : saved.authorization != null ? 'authorization' : saved.invitation != null ? 'invitation' : null;
      if (stage === fixture.armed) {
        delayed.set(this.transaction, stage);
        fixture.armed = null;
      }
    }
    return put.call(this, value, key);
  };
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
  createReadStream(artifactPath(file)).on('error', () => response.writeHead(404).end()).pipe(response);
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
  if (['admission-commit-cancel', 'migration-material-commit'].includes(selected)) config.maintenance = { contact_timeout_ms: 3000 };
  if (['paged-history', 'paged-migration'].includes(selected)) config.maintenance = { exploration_interval_ms: 60000 };
  if (selected === 'maintenance-concurrency') {
    config.rtc_enabled = true;
    config.rtc_ice = { stun_servers: [], local_bind_addresses: [], candidate_addresses: [] };
  }
  if (mid1Repair) {
    config.rtc_enabled = false;
    config.member_data = { via: ready.id, forward_data: true };
  }
  const url = `http://127.0.0.1:${web.address().port}/network.html?config=${encodeURIComponent(JSON.stringify(config))}&invitation=${ready.invitation}`;
  await page.goto(url);
  await page.waitForFunction(() => window.ready !== undefined, null, { timeout: 5000 });
  const first = await bounded(page.evaluate(() => window.ready), 'browser start');
  assert.notEqual(first, ready.id);
  let beforeHistoryContext;
  let beforeHistoryPage;
  let largeHistory = null;
  const verifyLargeBodies = async tab => {
    if (!largeHistory) return;
    for (const id of largeHistory.ids) {
      const retained = await tab.evaluate(id => {
        const original = JSON.parse(window.network.control_record(id));
        return { bytes: original.body.length / 2, originalZeros: /^0+$/.test(original.body) };
      }, id);
      assert.deepEqual(retained, { bytes: 6 * 1024 * 1024, originalZeros: true });
    }
  };
  const resumeAdmission = async (tab, method, input, interrupt) => {
    const invoke = () => tab.evaluate(({ method, input }) => window.network[method](input), { method, input });
    const progress = () => tab.evaluate(() => JSON.parse(window.network.status()));
    let previous = 0;
    let requestId;
    if (interrupt) {
      const waiting = invoke().catch(error => error);
      // The pending WASM request exclusively borrows BrowserNetwork. Inspect
      // only committed checkpoint metadata, without borrowing the request owner.
      const saved = await tab.evaluate(async () => {
        const until = Date.now() + 30000;
        while (Date.now() < until) {
        const db = await new Promise((resolve, reject) => {
          const request = indexedDB.open('miopunch-browser-network', 1);
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        });
        const saved = await new Promise((resolve, reject) => {
          const transaction = db.transaction('node', 'readonly');
          const request = transaction.objectStore('node').get('state');
          transaction.oncomplete = () => { db.close(); resolve(request.result); };
          transaction.onerror = () => { db.close(); reject(transaction.error); };
        });
        const state = JSON.parse(saved);
        const history = state.admission_history;
        if (!history?.records.length || history.complete) {
          await new Promise(resolve => setTimeout(resolve, 25));
          continue;
        }
        return {
          records: history.records.length,
          request_id: history.binding.request.map(byte => byte.toString(16).padStart(2, '0')).join(''),
          authorization_saved: state.authorization !== null,
          secret_saved: state.secret !== null,
        };
        }
        throw new Error('no durable partial admission checkpoint before deadline');
      });
      assert.equal(saved.authorization_saved, false);
      assert.equal(saved.secret_saved, false);
      requestId = saved.request_id;
      previous = saved.records;
      await tab.reload();
      await waiting;
      await bounded(tab.evaluate(() => window.ready), 'paged admission interrupted reopen');
      const reopened = await progress();
      assert.equal(reopened.admission_history.request_id, requestId);
      assert.equal(reopened.admission_history.records, previous);
      events.push({ case: 'paged-admission-checkpoint-reopen', requestId, records: previous });
    }
    for (let attempt = 0; attempt < 5; attempt++) {
      try {
        const stage = await invoke();
        assert(['PendingMaterials', 'Complete'].includes(stage));
        const authorized = await progress();
        if (requestId) assert.equal(authorized.admission_attempt.request_id, requestId);
        assert.equal(authorized.authorization_saved, true);
        if (stage === 'PendingMaterials') assert.equal(authorized.secret_saved, false);
        return;
      } catch (error) {
        const saved = await progress();
        const checkpoint = saved.admission_history;
        assert(checkpoint && checkpoint.records > previous, 'failed contact must retain new verified records');
        if (requestId) assert.equal(checkpoint.request_id, requestId);
        requestId = checkpoint.request_id;
        previous = checkpoint.records;
        assert.equal(saved.authorization_saved, false);
        assert.equal(saved.secret_saved, false);
        await tab.reload();
        await bounded(tab.evaluate(() => window.ready), 'paged admission retry reopen');
        const reopened = await progress();
        assert.equal(reopened.admission_history.request_id, requestId);
        assert.equal(reopened.admission_history.records, previous);
        events.push({ case: 'paged-admission-contact-resumed', requestId, records: previous });
      }
    }
    assert.fail('bounded admission retries did not finish');
  };
  const finishAdmission = async (tab, method, input, interrupt) => {
    let requestId;
    for (let contact = 0; contact < 3; contact++) {
      await resumeAdmission(tab, method, input, interrupt && contact === 0);
      const authorized = await tab.evaluate(() => JSON.parse(window.network.status()));
      if (requestId) assert.equal(authorized.admission_attempt.request_id, requestId);
      requestId = authorized.admission_attempt.request_id;
      try {
        assert.equal(await tab.evaluate(() => window.network.complete_admission()), 'Complete');
        return;
      } catch (error) {
        const saved = await tab.evaluate(() => JSON.parse(window.network.status()));
        assert.equal(saved.authorization_saved, true);
        assert.equal(saved.admission_attempt.request_id, requestId);
        if (!saved.secret_saved) assert.equal(saved.admission_history.complete, true);
        await tab.reload();
        await bounded(tab.evaluate(() => window.ready), 'authorized admission completion reopen');
        const reopened = await tab.evaluate(() => JSON.parse(window.network.status()));
        assert.equal(reopened.authorization_saved, true);
        assert.equal(reopened.admission_attempt.request_id, requestId);
        events.push({ case: 'paged-authorized-admission-contact-resumed', requestId, secretSaved: reopened.secret_saved });
      }
    }
    assert.fail('saved authorization did not complete its original material delivery');
  };
  if (selected === 'admission-commit-cancel') {
    await page.evaluate(installAdmissionCommitDelay);
    const request = () => page.evaluate(invitation => window.network.request_admission(invitation), ready.invitation);
    await assert.rejects(request);
    await page.waitForFunction(() => window.admissionCommitDelay.completed.includes('invitation'), null, { timeout: 8000 });
    assert.equal(await request(), 'AwaitingApproval', 'the same page must read its completed invitation commit');
    assert.equal((await native.command('approve', 'approved')).count, 2);
    await page.evaluate(() => { window.admissionCommitDelay.armed = 'authorization'; });
    await request().catch(error => String(error));
    await page.waitForFunction(() => window.admissionCommitDelay.completed.includes('authorization'), null, { timeout: 8000 });
    const authorized = JSON.parse(await page.evaluate(() => window.network.admission_material_receipt()));
    assert.equal(authorized.authorization_saved, true);
    assert.equal(authorized.secret_saved, false);
    assert.equal(await request(), 'PendingMaterials');
    await page.evaluate(() => { window.admissionCommitDelay.armed = 'secret'; });
    await assert.rejects(() => page.evaluate(() => window.network.complete_admission()));
    await page.waitForFunction(() => window.admissionCommitDelay.completed.includes('secret'), null, { timeout: 8000 });
    const saved = JSON.parse(await page.evaluate(() => window.network.admission_material_receipt()));
    assert.equal(saved.authorization, authorized.authorization);
    assert.equal(saved.request, authorized.request);
    assert.equal(saved.secret_saved, true);
    assert.equal(await request(), 'Complete');
    assert.equal(await page.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal(await page.evaluate(() => window.network.exchange('same-page-commit-retry')), 'same-page-commit-retry');
    const delayed = await page.evaluate(() => window.admissionCommitDelay.completed);
    assert.deepEqual(delayed, ['invitation', 'authorization', 'secret']);
    await page.reload();
    assert.equal(await bounded(page.evaluate(() => window.ready), 'committed admission reopen'), first);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
    assert.equal(await page.evaluate(() => window.network.exchange('commit-reopen')), 'commit-reopen');
    assert.equal((await native.command('status', 'status')).count, 2);
    events.push({ case: 'same-page-admission-commit-after-deadline', phases: delayed,
      sameAuthorization: true, sameRequest: true, reopened: true, members: 2 });
  } else {
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
  }
  if (selected === 'paged-recovery') {
    largeHistory = await native.command('large-history', 'large-history');
    for (const header of largeHistory.headers) {
      await page.evaluate(h => window.network.ingest_management_record(h, '00'.repeat(6 * 1024 * 1024)), header);
    }
    const credential = readFileSync(join(temporary, 'admin', 'genesis.credential'));
    const recoveringContext = await browserContext();
    const recovering = await recoveringContext.newPage();
    await recovering.goto(url);
    await recovering.evaluate(async ({ config, card, contact }) => {
      await window.ready;
      await window.network.close();
      const { BrowserNetwork } = await import('/pkg/miopunch_browser_probe.js');
      window.network = await BrowserNetwork.prepare_management_recovery(JSON.stringify(config), card, undefined, contact);
    }, { config, card: `miopunch-recovery:1:${credential.toString('base64url')}`, contact: first });
    let result;
    let lastFailure;
    for (let contact = 0; contact < largeHistory.count + 2; contact++) {
      try {
        result = JSON.parse(await recovering.evaluate(id => window.network.restore_management_authority(id), first));
        break;
      } catch (cause) {
        lastFailure = cause;
        await recovering.reload();
        await bounded(recovering.evaluate(() => window.ready), 'reopen recovery checkpoint');
      }
    }
    assert(result?.saved_locally && result.stage === 'Complete', String(lastFailure));
    await recovering.reload();
    await bounded(recovering.evaluate(() => window.ready), 'reopen recovered authority');
    assert(JSON.parse(await recovering.evaluate(() => window.network.management_recovery_status())).effective_administrator_locally);
    await verifyLargeBodies(recovering);
    await recoveringContext.close();
    events.push({ case: 'paged-recovery', bytes: largeHistory.body_bytes, originals: largeHistory.ids.length });
  }
  if (selected === 'paged-migration') {
    const targetPath = join(temporary, 'migration-target');
    const targetFixture = child('browser_network_native', [targetPath, relayUrl]);
    const targetReady = JSON.parse(await targetFixture.wait(/"event":"ready"/));
    largeHistory = await targetFixture.command('large-history', 'large-history');
    assert.equal(largeHistory.body_bytes, 18 * 1024 * 1024);
    const fixtureExited = new Promise(resolveExit => targetFixture.process.once('exit', resolveExit));
    targetFixture.process.stdin.end();
    await bounded(fixtureExited, 'migration target fixture shutdown');
    assert.equal(targetFixture.process.exitCode, 0);

    // Open that exact destination with the public NativeClient host: the small
    // admission fixture does not serve the migration's MemberSync qualification.
    const target = child('miopunch-client', ['open', targetPath]);
    const targetStatus = JSON.parse(await target.wait(/"event":"ready"/));
    assert.equal(targetStatus.status.node, targetReady.id);
    let sequence = 0;
    const rpc = async (op, values = {}) => {
      const id = ++sequence;
      const response = target.wait(new RegExp(`"id":${id},`), true).then(JSON.parse);
      target.process.stdin.write(`${JSON.stringify({ id, op, ...values })}\n`);
      const result = await response;
      assert.equal(result.ok, true, `migration destination ${op}`);
      return result.result;
    };
    const invitationPath = join(temporary, 'migration-target-invitation.json');
    await rpc('export_recovery_card', { path: join(temporary, 'migration-target-card') });
    await rpc('acknowledge_recovery_saved');
    await rpc('invite', { path: invitationPath });
    const invitation = readFileSync(invitationPath, 'utf8');
    const saved = () => page.evaluate(async () => {
      const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open('miopunch-browser-network', 1);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      return new Promise((resolve, reject) => {
        const transaction = db.transaction('node', 'readonly');
        const request = transaction.objectStore('node').get('state');
        transaction.oncomplete = () => { db.close(); resolve(JSON.parse(request.result)); };
        transaction.onerror = () => { db.close(); reject(transaction.error); };
      });
    });
    const original = await saved();
    const holdAfterFirstPage = limit => {
      const put = IDBObjectStore.prototype.put;
      window.migrationPageLimit = limit;
      window.allowMigrationHistory = false;
      IDBObjectStore.prototype.put = function (...args) {
        if (!window.allowMigrationHistory && args[1] === 'state') {
          const history = JSON.parse(args[0]).migration?.inactive?.admission_history;
          if (history?.records.length) {
            window.migrationPageLimit ??= history.records.length;
            if (history.records.length > window.migrationPageLimit || history.complete) {
              this.transaction.abort();
              throw new DOMException('owned migration page interruption', 'QuotaExceededError');
            }
          }
        }
        return put.apply(this, args);
      };
    };
    await page.evaluate(holdAfterFirstPage, null);
    const started = JSON.parse(await page.evaluate(inv => window.network.begin_network_migration(inv), invitation));
    assert.equal(started.switched, false);
    const admission = started.admission.request_id;
    let pending;
    for (let attempt = 0; attempt < 40; attempt++) {
      pending = (await rpc('requests')).requests;
      if (pending.length) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(pending.length, 1);
    assert.equal(pending[0].applicant, first);
    await rpc('approve', pending[0]);
    await page.waitForFunction(() => window.migrationPageLimit > 0, null, { timeout: 90000 });
    // Wait for the first real strict IndexedDB transaction, not its put call.
    let partial;
    for (let attempt = 0; attempt < 100; attempt++) {
      partial = await saved();
      if (partial.migration?.inactive.admission_history?.records.length) break;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    const history = partial.migration.inactive.admission_history;
    assert(history.records.length > 0 && !history.complete);
    assert.equal(partial.migration.verified, null);
    assert.deepEqual(partial.records, original.records);
    assert.deepEqual(partial.authorization, original.authorization);
    assert.deepEqual(partial.secret, original.secret);
    assert.equal(partial.admission_history, undefined);
    assert.equal(partial.migration.inactive.authorization, null);
    assert.equal(partial.migration.inactive.secret, null);
    assert.equal(await page.evaluate(() => window.network.exchange('source-usable-with-inactive-page')), 'source-usable-with-inactive-page');
    events.push({ case: 'paged-migration-inactive-checkpoint', records: history.records.length, activeSourceUnchanged: true, sourceBusiness: true });
    await page.addInitScript(holdAfterFirstPage, history.records.length);
    await page.reload();
    assert.equal(await bounded(page.evaluate(() => window.ready), 'paged migration same-node reopen'), first);
    const reopened = await saved();
    assert.deepEqual(reopened.migration.inactive.admission_history, history);
    assert.deepEqual(reopened.records, original.records);
    const progress = JSON.parse(await page.evaluate(() => window.network.migration_progress()));
    assert.equal(progress.admission.request_id, admission);
    assert.equal(progress.switched, false);
    assert.equal(await page.evaluate(() => window.network.exchange('source-usable-after-page-reopen')), 'source-usable-after-page-reopen');
    await page.evaluate(() => { window.allowMigrationHistory = true; });
    await page.waitForFunction(() => JSON.parse(window.network.migration_progress())?.switched, null, { timeout: 180000 });
    const switched = JSON.parse(await page.evaluate(() => window.network.migration_progress()));
    assert.equal(switched.admission.request_id, admission);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
    assert.deepEqual((await saved()).records.length, largeHistory.count + 1);
    await verifyLargeBodies(page);
    events.push({ case: 'paged-migration-same-node-resumed-and-switched', node: first, requestId: admission, bodyBytes: largeHistory.body_bytes, originalBodies: true, switched: true });
  }
  if (selected === 'migration-material-commit') {
    const targetPath = join(temporary, 'migration-commit-target');
    const targetFixture = child('browser_network_native', [targetPath, relayUrl]);
    const targetReady = JSON.parse(await targetFixture.wait(/"event":"ready"/));
    const stopped = new Promise(resolve => targetFixture.process.once('exit', resolve));
    targetFixture.process.stdin.end();
    assert.equal(await bounded(stopped, 'small migration fixture shutdown'), 0);
    const target = child('miopunch-client', ['open', targetPath]);
    assert.equal(JSON.parse(await target.wait(/"event":"ready"/)).status.node, targetReady.id);
    let sequence = 0;
    const rpc = async (op, values = {}) => {
      const id = ++sequence;
      const response = target.wait(new RegExp(`"id":${id},`), true).then(JSON.parse);
      target.process.stdin.write(`${JSON.stringify({ id, op, ...values })}\n`);
      const result = await response;
      assert.equal(result.ok, true, `small migration target ${op}`);
      return result.result;
    };
    await rpc('export_recovery_card', { path: join(temporary, 'migration-commit-card') });
    await rpc('acknowledge_recovery_saved');
    const invitationPath = join(temporary, 'migration-commit-invitation.json');
    await rpc('invite', { path: invitationPath });
    await page.evaluate(installAdmissionCommitDelay, { inactive: true, stage: 'authorization' });
    const started = JSON.parse(await page.evaluate(invitation => window.network.begin_network_migration(invitation), readFileSync(invitationPath, 'utf8')));
    let pending;
    for (let attempt = 0; attempt < 40; attempt++) {
      pending = (await rpc('requests')).requests;
      if (pending.length) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(pending.length, 1);
    assert.equal(pending[0].applicant, first);
    await rpc('approve', pending[0]);
    await page.waitForFunction(() => window.admissionCommitDelay.completed.includes('authorization'), null, { timeout: 10000 });
    await page.evaluate(() => { window.admissionCommitDelay.armed = 'secret'; });
    assert.equal(await page.evaluate(() => window.network.exchange('source-during-local-material-commit')), 'source-during-local-material-commit');
    await page.waitForFunction(() => window.admissionCommitDelay.completed.includes('secret'), null, { timeout: 10000 });
    await page.waitForFunction(() => JSON.parse(window.network.migration_progress()).switched, null, { timeout: 30000 });
    const progress = JSON.parse(await page.evaluate(() => window.network.migration_progress()));
    assert.equal(progress.admission.request_id, started.admission.request_id);
    const members = JSON.parse(await page.evaluate(() => window.network.control_state())).members.map(member => member.id);
    assert(members.includes(first) && members.includes(targetReady.id) && !members.includes(ready.id));
    const phases = await page.evaluate(() => window.admissionCommitDelay.completed);
    assert.deepEqual(phases, ['authorization', 'secret']);
    await page.reload();
    assert.equal(await bounded(page.evaluate(() => window.ready), 'migration material reopen'), first);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.migration_progress())).switched, true);
    assert.equal(JSON.parse(await page.evaluate(() => window.network.status())).stage, 'Complete');
    events.push({ case: 'migration-material-commit-after-deadline', phases,
      sourceUsableBeforeSwitch: true, switched: true, sameIdentity: true, reopened: true });
  }
  if (selected === 'paged-history') {
    beforeHistoryContext = await browserContext();
    beforeHistoryPage = await beforeHistoryContext.newPage();
    await beforeHistoryPage.goto(url);
    await bounded(beforeHistoryPage.evaluate(() => window.ready), 'small-history replica receiver startup');
    assert.equal(await beforeHistoryPage.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'AwaitingApproval');
    await native.command('approve', 'approved');
    assert.equal(await beforeHistoryPage.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'PendingMaterials');
    assert.equal(await beforeHistoryPage.evaluate(() => window.network.complete_admission()), 'Complete');
    await beforeHistoryPage.evaluate(() => window.network.close());
    await page.evaluate(() => window.network.close());
    largeHistory = await native.command('large-history', 'large-history');
    assert(largeHistory.body_bytes > 16 * 1024 * 1024);
    const joiningContext = await browserContext();
    const joining = await joiningContext.newPage();
    await joining.goto(url);
    await bounded(joining.evaluate(() => window.ready), 'paged native admission startup');
    assert.equal(await joining.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'AwaitingApproval');
    await native.command('approve', 'approved');
    await finishAdmission(joining, 'request_admission', ready.invitation, true);
    await joining.reload();
    await bounded(joining.evaluate(() => window.ready), 'paged native admission final reopen');
    await verifyLargeBodies(joining);
    await joiningContext.close();
    await page.reload();
    await bounded(page.evaluate(() => window.ready), 'small-history native replica receiver reopen');
  }


  if (selected === 'maintenance-concurrency') {
    const repairPhases = [];
    page.on('console', message => {
      try {
        const event = JSON.parse(message.text());
        if (event.event === 'connection-timeline' && event.source === 'foreground_repair') repairPhases.push(event);
      } catch { /* non-event console message */ }
    });
    const peerContext = await browserContext();
    const peer = await peerContext.newPage();
    await peer.goto(url);
    const peerId = await peer.evaluate(() => window.ready);
    assert.equal(await peer.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'AwaitingApproval');
    await native.command('approve', 'approved');
    assert.equal(await peer.evaluate(invitation => window.network.request_admission(invitation), ready.invitation), 'PendingMaterials');
    assert.equal(await peer.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal(JSON.parse(await page.evaluate(id => window.network.sync_replica(id), ready.id)).complete, true);
    await Promise.all([
      peer.evaluate(async () => { window.member = await window.network.member_accept_attempt(5000).accept(); }),
      page.evaluate(async id => { window.member = await window.network.open_member_connection(id); }, peerId),
    ]);
    const sessions = () => Promise.all([page, peer].map(tab => tab.evaluate(() => JSON.parse(window.member.session_diagnostics()).coordinator.stableId)));
    const ids = await sessions();
    assert(ids.every(Number.isSafeInteger));
    await page.evaluate(async () => { window.stream = await window.member.open_data_stream(); });
    const readPeer = peer.evaluate(async () => { window.stream = await window.member.accept_data_stream(); return [...await window.stream.read_chunk()]; });
    await page.evaluate(() => window.stream.write_chunk(Uint8Array.of(7)));
    assert.deepEqual(await readPeer, [7]);

    // Hold only the owned relay until the actual dial has started under its M reservation.
    const holdRelay = tab => tab.evaluate(relay => {
      const send = WebSocket.prototype.send;
      const pending = [];
      WebSocket.prototype.send = function(data) {
        if (this.url.startsWith(relay)) pending.push([this, structuredClone(data)]);
        else send.call(this, data);
      };
      window.releaseMaintenanceRelay = () => {
        WebSocket.prototype.send = send;
        for (const [socket, data] of pending) send.call(socket, data);
      };
    }, relayUrl.replace(/^http/, 'ws'));
    const releaseRelay = tab => tab.evaluate(() => window.releaseMaintenanceRelay());
    const phaseStarted = (tab, source, phase) => bounded(new Promise(resolve => {
      function observe(message) {
        let event;
        try { event = JSON.parse(message.text()); } catch { return; }
        if (event.event === 'connection-timeline' && event.source === source && event.phase === phase && event.result === 'started') {
          tab.off('console', observe);
          resolve();
        }
      }
      tab.on('console', observe);
    }), `${source} ${phase}`);
    async function startImprovement(tab) {
      const started = phaseStarted(tab, 'explicit_improvement', 'route-dial');
      await tab.evaluate(() => { window.improvement = window.member.improve_path(2500).then(JSON.parse, String); });
      await started;
    }
    async function startRepair(budget) {
      const started = phaseStarted(page, 'foreground_repair', 'repair');
      await page.evaluate(ms => { window.repair = window.member.recover_path(ms).then(JSON.parse, String); }, budget);
      await started;
    }

    let expired, cancelled;
    await holdRelay(page);
    try {
      await startImprovement(page);
      expired = JSON.parse(await page.evaluate(() => window.member.recover_path(1)));
      await startRepair(1000);
      await page.evaluate(() => window.member.cancel_path_recovery());
      cancelled = await page.evaluate(() => window.repair);
      await startRepair(2500);
    } finally {
      await releaseRelay(page);
    }
    assert.equal(expired.outcome, 'unreachable');
    assert.match(expired.failure, /repair reservation: DeadlineExceeded/);
    assert.equal(cancelled.outcome, 'unreachable');
    assert.match(cancelled.failure, /repair reservation: Cancelled/);
    const [owner, reused] = await page.evaluate(() => Promise.all([window.improvement, window.repair]));
    assert.equal(owner.outcome, 'committed');
    assert.equal(reused.outcome, 'recovered');
    assert(repairPhases.some(event => event.phase === 'repair' && event.result === 'reused' && event.stableId === ids[0]));

    const tabs = [page, peer];
    await Promise.all(tabs.map(holdRelay));
    try {
      await Promise.all(tabs.map(startImprovement));
    } finally {
      await Promise.all(tabs.map(releaseRelay));
    }
    const outcomes = await Promise.all(tabs.map(tab => tab.evaluate(() => window.improvement)));
    assert(outcomes.some(outcome => outcome === 'peer deferred the active path attempt'), JSON.stringify(outcomes));
    assert.deepEqual(await sessions(), ids);
    await peer.evaluate(() => window.stream.write_chunk(Uint8Array.of(11)));
    assert.deepEqual(await page.evaluate(async () => [...await window.stream.read_chunk()]), [11]);
    await Promise.all([page, peer].map(tab => tab.evaluate(() => {
      window.stream.free();
      window.member.close(); window.member.free();
    })));
    await peerContext.close();
    events.push({ case: 'maintenance-concurrency', waiterResults: [expired.failure, cancelled.failure, reused.outcome], outcomes, originalStreamContinued: true });
  }

  if (mid1Repair) {
    const peerContext = await browserContext();
    const peer = await peerContext.newPage();
    peer.on('console', message => console.log(`peer: ${message.text()}`));
    await peer.goto(url);
    const peerId = await bounded(peer.evaluate(() => window.ready), 'second MID1 member startup');
    assert.notEqual(peerId, first);
    assert.notEqual(peerId, ready.id);
    assert.equal(await peer.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'AwaitingApproval');
    await native.command('approve', 'approved');
    assert.equal(await peer.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'PendingMaterials');
    assert.equal(await peer.evaluate(() => window.network.complete_admission()), 'Complete');
    assert.equal(JSON.parse(await page.evaluate(id => window.network.sync_replica(id), ready.id)).complete, true);
    const tabs = [page, peer];
    await native.command('member-data-enable', 'member-data-enable');
    const routes = await bounded(Promise.all(tabs.map(tab => tab.evaluate(async id => {
      window.outer = await window.network.open_member_connection(id);
      return JSON.parse(await window.network.attach_member_data(window.outer, 45000, 8000)).dynamicRoute;
    }, ready.id))), 'two authorized MID1 outer links');
    assert(routes.every(route => /^[a-f0-9]{96}$/.test(route)));
    await bounded(Promise.all([
      peer.evaluate(async () => { window.member = await window.network.member_accept_attempt(8000).accept(); }),
      page.evaluate(async ([id, route]) => {
        window.member = await window.network.member_data_connection_attempt(id, route, 8000).connect();
      }, [peerId, routes[1]]),
    ]), 'inner MID1 session');
    const sessions = () => Promise.all(tabs.map(tab => tab.evaluate(() => JSON.parse(window.member.session_diagnostics()))));
    const original = await sessions();
    const ids = original.map(session => session.coordinator.stableId);
    assert(ids.every(Number.isSafeInteger));
    for (const session of original) {
      assert(session.ordinaryHandle && session.memberAuthorized && session.coordinator.freshAck);
      assert.equal(session.paths.length, 1);
      assert(session.paths[0].isMemberData && session.paths[0].selected);
    }
    await page.evaluate(async () => { window.stream = await window.member.open_data_stream(); });
    const readPeer = peer.evaluate(async () => { window.stream = await window.member.accept_data_stream(); return [...await window.stream.read_chunk()]; });
    await page.evaluate(() => window.stream.write_chunk(Uint8Array.of(7)));
    assert.deepEqual(await bounded(readPeer, 'first original stream bytes'), [7]);
    const exchange = (from, to, byte) => bounded(Promise.all([
      from.evaluate(value => window.stream.write_chunk(Uint8Array.of(value)), byte),
      to.evaluate(async () => [...await window.stream.read_chunk()]),
    ]).then(([, data]) => assert.deepEqual(data, [byte])), 'same MID1 stream sample');
    await exchange(peer, page, 8);
    const repair = JSON.parse(await bounded(page.evaluate(() => window.member.recover_path(2000)), 'public MID1 repair'));
    assert.equal(repair.outcome, 'unreachable', JSON.stringify(repair));
    assert.equal(repair.stableId, ids[0]);
    assert(repair.elapsedMs < 2250);
    assert(repair.failure.includes('DataPathUnavailable'), JSON.stringify(repair));
    await exchange(page, peer, 11);
    await exchange(peer, page, 12);

    const alternate = selected === 'mid1-repair-backup';
    const dropNetwork = selected === 'mid1-repair-drop';
    if (alternate) {
      await bounded(page.evaluate(relay => window.member.mid1_relay_backup_fixture(relay), relayUrl), 'open relay on the original member');
      await Promise.all(tabs.map(tab => tab.waitForFunction(() => {
        const paths = JSON.parse(window.member.session_diagnostics()).paths;
        return paths.some(path => path.isMemberData) && paths.some(path => path.isRelay);
      }, null, { timeout: 5000 })));
    }
    if (dropNetwork) {
      // Keep the original member and stream handles after releasing the owner.
      await page.evaluate(() => window.network.free());
      await page.waitForFunction(() => window.member.is_closed(), null, { timeout: 2000 });
    }
    assert.equal((await native.command('member-data-disable', 'member-data-disable')).activeLinks, 0);
    const liveNetworks = dropNetwork ? [peer] : tabs;
    await Promise.all(liveNetworks.map(tab => tab.waitForFunction(() => JSON.parse(window.network.member_data_diagnostics()).stopped, null, { timeout: 2000 })));
    assert.deepEqual((await sessions()).map(session => session.coordinator.stableId), ids);
    for (const tab of liveNetworks) assert(await tab.evaluate(() => !window.outer.is_closed()));
    if (alternate) {
      await exchange(page, peer, 21);
      await exchange(peer, page, 22);
      assert((await sessions()).every(session => session.ordinaryHandle && session.paths.some(path => path.isRelay && path.selected && path.streamSent > 0 && path.streamReceived > 0)));
    } else {
      await Promise.all(tabs.map(tab => tab.waitForFunction(() => window.member.is_closed(), null, { timeout: 2000 })));
      assert((await sessions()).every(session => !session.ordinaryHandle));
      for (const tab of tabs) {
        await assert.rejects(tab.evaluate(() => window.stream.write_chunk(Uint8Array.of(31))));
      }
      const terminal = JSON.parse(await page.evaluate(() => window.member.recover_path(2000)));
      assert.equal(terminal.outcome, 'terminal');
      assert.equal(terminal.stableId, ids[0]);
    }
    await Promise.all(tabs.map(tab => tab.evaluate(async closeNetwork => {
      window.stream.free(); window.member.free(); window.outer.free();
      if (closeNetwork) await window.network.close();
    }, liveNetworks.includes(tab))));
    await peerContext.close();
    events.push({ case: selected, repair, ids, originalBytesAfterRepair: true, originalClosedAfterRevocation: !alternate, allowedRelayContinued: alternate });
  }

  if (selected === 'replica' || selected === 'paged-history') {
    const thirdContext = beforeHistoryContext ?? await browserContext();
    const third = beforeHistoryPage ?? await thirdContext.newPage();
    if (!beforeHistoryPage) {
      await third.goto(url);
      await bounded(third.evaluate(() => window.ready), 'third browser start');
      assert.equal(await third.evaluate(inv => window.network.request_admission(inv), ready.invitation), 'AwaitingApproval');
      await native.command('approve', 'approved');
      await third.evaluate(inv => window.network.request_admission(inv), ready.invitation);
      await third.evaluate(() => window.network.complete_admission());
    }
    const authored = await native.command('record', 'record');
    let outcome;
    let nativeProgress = JSON.parse(await page.evaluate(() => window.network.control_state())).records;
    for (let attempt = 0; attempt < (largeHistory ? 5 : 1); attempt++) {
      try {
        outcome = JSON.parse(await page.evaluate(id => window.network.sync_replica(id), ready.id));
        break;
      } catch (error) {
        const transfer = JSON.parse(await page.evaluate(() => window.network.replica_diagnostics()))?.transfer;
        const saved = JSON.parse(await page.evaluate(() => window.network.control_state()));
        assert(largeHistory && saved.records > nativeProgress, `native replica contact must retain new original records: ${error}`);
        nativeProgress = saved.records;
        await page.reload();
        await bounded(page.evaluate(() => window.ready), 'native replica partial reopen');
        const reopened = JSON.parse(await page.evaluate(() => window.network.control_state())).records;
        assert(reopened >= nativeProgress, 'native replica reopen must retain committed originals');
        nativeProgress = reopened;
        events.push({ case: 'paged-native-replica-checkpoint-reopen', records: nativeProgress, transfer });
      }
    }
    assert(outcome, 'native replica did not finish within bounded contacts');
    if (largeHistory) {
      assert(outcome.reused >= nativeProgress);
      assert(outcome.received < outcome.scope.length);
    }
    assert.equal(outcome.complete, true);
    assert(outcome.scope.includes(authored.record));
    if (largeHistory) for (const id of largeHistory.ids) assert(outcome.scope.includes(id));
    await page.reload();
    await bounded(page.evaluate(() => window.ready), 'replica reopen');
    assert.equal(JSON.parse(await page.evaluate(() => window.network.control_state())).records, outcome.scope.length);
    native.process.stdin.write('stop\n');
    assert.equal(await bounded(new Promise(resolve => native.process.once('exit', resolve)), 'original author offline'),0);
    console.log('AUTHOR_OFFLINE exit=0');
    if (largeHistory) {
      const idleProviderUrl = new URL(url);
      idleProviderUrl.searchParams.set('config', JSON.stringify({ ...config,
        maintenance: { ...config.maintenance, contact_timeout_ms: 5000 } }));
      await page.goto(idleProviderUrl.toString());
      assert.equal(await bounded(page.evaluate(() => window.ready), 'short provider startup'), first);
      const providerStarted = Date.now();
      await assert.rejects(() => page.evaluate(() => window.network.serve_replica()));
      const providerMs = Date.now() - providerStarted;
      assert(providerMs >= 4900 && providerMs < 6500, `provider exceeded its explicit deadline: ${providerMs}`);
      await page.goto(url);
      assert.equal(await bounded(page.evaluate(() => window.ready), 'provider default-budget reopen'), first);
      events.push({ case: 'paged-replica-provider-explicit-deadline', budgetMs: 5000, operationMs: providerMs });
      const shortUrl = new URL(url);
      const shortBudgetMs = 18000;
      shortUrl.searchParams.set('config', JSON.stringify({ ...config,
        maintenance: { ...config.maintenance, contact_timeout_ms: shortBudgetMs } }));
      await third.goto(url);
      await bounded(third.evaluate(() => window.ready), 'small-history browser replica receiver reopen');
      const shortIdentity = await third.evaluate(() => window.network.id());
      const beforeShort = JSON.parse(await third.evaluate(() => window.network.control_state())).records;
      await third.evaluate(before => {
        const put = IDBObjectStore.prototype.put;
        let held = false;
        let committedRecords = 0;
        const pending = [];
        IDBObjectStore.prototype.put = function(value, ...args) {
          if (!held && this.name === 'node') {
            const records = JSON.parse(value).records;
            if (records.length > before && records.some(([, body]) => typeof body === 'string' && body.length === 8 * 1024 * 1024)) this.transaction.addEventListener('complete', () => {
              if (held) return;
              held = true;
              committedRecords = records.length;
              // Stop at an actual durable original, before its page receipt.
              WebSocket.prototype.send = function(data) { pending.push([this, structuredClone(data)]); };
            }, { once: true });
          }
          return put.call(this, value, ...args);
        };
        window.shortReplicaCheckpoint = () => ({ held, committedRecords });
      }, beforeShort);
      const checkpointServing = page.evaluate(() => window.network.serve_replica()).catch(error => error);
      const checkpointReceiving = third.evaluate(id => window.network.sync_replica(id), first).catch(error => error);
      await third.waitForFunction(() => window.shortReplicaCheckpoint().held, null, { timeout: 60000 });
      const checkpoint = await third.evaluate(() => window.shortReplicaCheckpoint());
      await third.goto(shortUrl.toString());
      await checkpointReceiving;
      await page.reload();
      await checkpointServing;
      assert.equal(await bounded(page.evaluate(() => window.ready), 'checkpoint provider reopen'), first);
      assert.equal(await bounded(third.evaluate(() => window.ready), 'checkpoint receiver reopen'), shortIdentity);
      assert(JSON.parse(await third.evaluate(() => window.network.control_state())).records >= checkpoint.committedRecords);
      events.push({ case: 'paged-replica-forced-checkpoint-reopen', records: checkpoint.committedRecords });
      await third.evaluate(() => {
        const pending = [];
        const send = WebSocket.prototype.send;
        const original = Object.fromEntries(['log', 'info', 'debug'].map(level => [level, console[level]]));
        let held = false;
        for (const level of Object.keys(original)) console[level] = function(...args) {
          original[level].apply(this, args);
          if (held) return;
          let event;
          try { event = JSON.parse(args[0]); } catch { return; }
          if (event.event === 'connection-timeline' && event.phase === 'dial-delivered' && event.detail?.purpose === 'replica') {
            held = true;
            WebSocket.prototype.send = function(data) { pending.push([this, structuredClone(data)]); };
          }
        };
        window.releaseShortReplica = () => {
          for (const [level, log] of Object.entries(original)) console[level] = log;
          WebSocket.prototype.send = send;
          for (const [socket, data] of pending) send.call(socket, data);
          return { held };
        };
      });
      const shortServing = page.evaluate(() => window.network.serve_replica()).catch(error => error);
      let shortFailure;
      try {
        await third.evaluate(id => window.network.sync_replica(id), first);
      } catch (error) { shortFailure = String(error); }
      const held = await third.evaluate(() => window.releaseShortReplica());
      await shortServing;
      assert(shortFailure, 'held transport must not report Replica complete');
      assert.equal(held.held, true);
      assert(checkpoint.committedRecords > beforeShort);
      const shortDiagnostics = JSON.parse(await third.evaluate(() => window.network.replica_diagnostics()));
      assert.equal(shortDiagnostics.completeReturned, false);
      assert.equal(shortDiagnostics.callerBudgetMs, shortBudgetMs);
      assert.equal(shortDiagnostics.remainingCallerMs, 0);
      assert(shortDiagnostics.operationMs >= shortBudgetMs - 100);
      assert(shortDiagnostics.operationMs < shortBudgetMs + 1500);
      let originalsRetained = 0;
      for (const id of largeHistory.ids) {
        const original = await third.evaluate(id => {
          try {
            const record = JSON.parse(window.network.control_record(id));
            return { bytes: record.body.length / 2, zeros: /^0+$/.test(record.body) };
          } catch { return null; }
        }, id);
        if (original) {
          assert.deepEqual(original, { bytes: 6 * 1024 * 1024, zeros: true });
          originalsRetained++;
        }
      }
      assert(originalsRetained > 0);
      await third.goto(url);
      assert.equal(await bounded(third.evaluate(() => window.ready), 'short Replica durable reopen'), shortIdentity);
      assert(JSON.parse(await third.evaluate(() => window.network.control_state())).records >= checkpoint.committedRecords);
      events.push({ case: 'paged-replica-explicit-deadline-reopen', budgetMs: shortBudgetMs,
        operationMs: shortDiagnostics.operationMs, originalsRetained, records: checkpoint.committedRecords });
    }
    let copied;
    let previousRecords = JSON.parse(await third.evaluate(() => window.network.control_state())).records;
    for (let attempt = 0; attempt < (largeHistory ? 5 : 1); attempt++) {
      const serving = page.evaluate(() => window.network.serve_replica()).catch(error => error);
      try {
        copied = JSON.parse(await third.evaluate(id => window.network.sync_replica(id), first));
        await serving;
        break;
      } catch (error) {
        const providerError = await serving;
        const diagnosis = JSON.parse(await third.evaluate(() => window.network.replica_diagnostics()));
        const transfer = diagnosis?.transfer;
        const progress = JSON.parse(await third.evaluate(() => window.network.control_state()));
        assert(largeHistory, `browser replica failed: ${error}; provider: ${providerError}`);
        if (diagnosis === null) {
          events.push({ case: 'paged-replica-contact-unavailable', error: String(error), providerError: String(providerError) });
          continue;
        }
        const originalsSaved = progress.records >= outcome.scope.length;
        assert(progress.records > previousRecords || originalsSaved,
          `replica retry must retain new original records: ${error}; provider: ${providerError}; diagnosis: ${JSON.stringify(diagnosis)}`);
        previousRecords = progress.records;
        await third.reload();
        await bounded(third.evaluate(() => window.ready), 'partial replica durable reopen');
        const reopened = JSON.parse(await third.evaluate(() => window.network.control_state())).records;
        assert(reopened >= previousRecords, 'browser replica reopen must retain committed originals');
        previousRecords = reopened;
        events.push({ case: 'paged-replica-checkpoint-reopen', records: previousRecords, transfer });
      }
    }
    assert(copied, 'replica did not finish within bounded contacts');
    if (largeHistory) {
      assert(copied.reused >= previousRecords);
      assert(copied.received < copied.scope.length);
    }
    assert.equal(copied.complete, true);
    assert.equal(copied.source, first);
    assert(copied.scope.includes(authored.record));
    if (largeHistory) for (const id of largeHistory.ids) assert(copied.scope.includes(id));
    const original=JSON.parse(await third.evaluate(id=>window.network.control_record(id),authored.record));
    assert.equal(original.body,Buffer.from('original-author browser replica body').toString('hex'));
    await third.reload();
    await bounded(third.evaluate(() => window.ready), 'third durable replica');
    assert.equal(JSON.parse(await third.evaluate(() => window.network.control_state())).records, outcome.scope.length);
    await verifyLargeBodies(third);
    if (largeHistory) {
      const memberSync = JSON.parse(await third.evaluate(id => window.network.sync_services(id), first));
      assert.equal(memberSync.saved_locally, true);
      assert.equal(JSON.parse(await third.evaluate(() => window.network.control_state())).records, outcome.scope.length);
      events.push({ case: 'paged-browser-member-sync', originalAuthorOffline: true, savedLocally: true });
    }
    if (largeHistory) {
      const credential = readFileSync(join(temporary, 'admin', 'genesis.credential'));
      const confirmation = await page.evaluate(async ({ card, first }) => {
        const proposal = JSON.parse(await window.network.propose_device_operation('appoint-administrator', first));
        return JSON.parse(await window.network.confirm_device_operation_offline(proposal.operation.id, card));
      }, { card: `miopunch-recovery:1:${credential.toString('base64url')}`, first });
      assert.equal(confirmation.effective_locally, true);
      const applicantContext = await browserContext();
      const applicant = await applicantContext.newPage();
      await applicant.goto(url);
      await bounded(applicant.evaluate(() => window.ready), 'large-history browser applicant');
      const code = await page.evaluate(() => window.network.issue_precode(BigInt(Math.floor(Date.now() / 1000) + 3600), 1n));
      await finishAdmission(applicant, 'request_precode', code, false);
      assert.equal(await page.evaluate(code => window.network.precode_usage(code), code), 1n);
      const applicantId = await applicant.evaluate(() => window.network.id());
      const receipts = JSON.parse(await page.evaluate(() => window.network.admission_history()));
      assert.equal(receipts.find(receipt => receipt.applicant === applicantId).decision, 'ReceiptReceived');
      await applicant.reload();
      await bounded(applicant.evaluate(() => window.ready), 'large-history browser admission reopen');
      const state = JSON.parse(await applicant.evaluate(() => window.network.control_state()));
      assert(state.records > outcome.scope.length);
      await verifyLargeBodies(applicant);
      await applicantContext.close();
      events.push({ case: 'paged-browser-provider-admission', originalAuthorOffline: true, records: state.records });
    }
    events.push({ case: largeHistory ? 'paged-history-author-offline-browser-reload-third-node' : 'replica-author-offline-browser-reload-third-node', ...copied, ...(largeHistory ? {bodyBytes:largeHistory.body_bytes, signedLargeRecords:largeHistory.ids.length, pagedAdmission:true} : {}) });
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
