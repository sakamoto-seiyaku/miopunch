// Owned-page instrumentation of the real SDK IndexedDB transaction. It does
// not create admission material, alter identity, or implement delivery.
export function installAdmissionCommitFixture({ abortSecretOnce = true } = {}) {
  const address = new URL(location.href);
  const actor = address.searchParams.get('actor');
  if (address.protocol !== 'http:' || address.hostname !== '127.0.0.1' || !address.port || !actor)
    throw new Error('an exact owned local SDK page is required');
  if (globalThis.__mio83AdmissionFixture) throw new Error('owned admission fixture already installed');
  const originalPut = IDBObjectStore.prototype.put;
  const transactions = [];
  let authorizationOnlyCommitted = false;
  let abortArmed = abortSecretOnce;
  let secretAbortObserved = null;
  let restored = false;
  let reportFailures = 0;
  const snapshot = () => ({ scope: 'owned actual SDK IndexedDB boundary only',
    authorizationOnlyCommitted, secretAbortObserved, faultArmed: abortArmed,
    restored, reportFailures, transactions: transactions.map(value => ({ ...value })) });
  const report = () => {
    // Only these fixed facts cross local control; never the parsed snapshot.
    fetch(`/result?actor=${encodeURIComponent(actor)}`, { method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: 'mio83-admission-fixture', ok: true, result: snapshot() }),
    }).then(response => { if (!response.ok) reportFailures++; }).catch(() => { reportFailures++; });
  };
  function wrappedPut(value, key) {
    if (this.name !== 'node' || key !== 'state'
      || this.transaction?.db?.name !== 'miopunch-browser-network' || typeof value !== 'string')
      return originalPut.call(this, value, key);
    let next;
    try { next = JSON.parse(value); } catch { return originalPut.call(this, value, key); }
    const authorizationSaved = next.authorization != null;
    const secretSaved = next.secret != null;
    if (!authorizationSaved) return originalPut.call(this, value, key);
    const transaction = this.transaction;
    const observed = { phase: secretSaved ? 'secret' : 'authorization-only',
      mode: transaction.mode, durability: transaction.durability ?? null,
      abortRequested: false, committed: null, aborted: null, errorEvent: null };
    transactions.push(observed);
    transaction.addEventListener('complete', () => {
      observed.committed = true; observed.aborted = false;
      if (!secretSaved) authorizationOnlyCommitted = true;
      report();
    }, { once: true });
    transaction.addEventListener('abort', () => {
      observed.committed = false; observed.aborted = true;
      if (observed.abortRequested) secretAbortObserved = true;
      report();
    }, { once: true });
    transaction.addEventListener('error', () => { observed.errorEvent = true; report(); }, { once: true });
    if (secretSaved && authorizationOnlyCommitted && abortArmed) {
      abortArmed = false;
      observed.abortRequested = true;
      transaction.abort();
      report();
      return undefined;
    }
    return originalPut.call(this, value, key);
  }
  IDBObjectStore.prototype.put = wrappedPut;
  const fixture = {
    snapshot,
    restore() {
      if (restored) return snapshot();
      if (IDBObjectStore.prototype.put !== wrappedPut)
        throw new Error('owned admission fixture no longer owns the put wrapper');
      IDBObjectStore.prototype.put = originalPut;
      abortArmed = false; restored = true;
      return snapshot();
    },
  };
  globalThis.__mio83AdmissionFixture = fixture;
  addEventListener('pagehide', () => { fixture.restore(); }, { once: true });
  report();
}

export function admissionCommitPrelude(options = {}) {
  return `(${installAdmissionCommitFixture.toString()})(${JSON.stringify(options)});`;
}
