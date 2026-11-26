// One origin owns one node identity. A page holds the Web Lock for its lifetime;
// browser reclamation releases it. Other pages fail promptly rather than opening
// competing endpoints under the same key. No storage or keys go to the server.
const leases = new Map();
let nextLease = 1;

export async function acquireNode() {
  if (!navigator.locks || !globalThis.indexedDB) throw new Error('durable browser storage and Web Locks required');
  return new Promise((resolve, reject) => {
    let released;
    const closed = new Promise(done => { released = done; });
    navigator.locks.request('miopunch-browser-network-v1', { ifAvailable: true }, async lock => {
      if (!lock) { reject(new Error('identity already active in another page')); return; }
      const token = nextLease++;
      await new Promise(release => {
        leases.set(token, { release, closed, saves: new Set(), releasing: false });
        resolve(token);
      });
      leases.delete(token);
    }).then(released, reject);
  });
}

function releaseLease(token) {
  const lease = leases.get(token);
  if (!lease) return;
  if (!lease.releasing) {
    lease.releasing = true;
    // A dropped Rust future cannot cancel an IndexedDB transaction. The next
    // owner must open only after any save already started by this lease settles.
    Promise.allSettled(lease.saves).then(lease.release);
  }
  return lease.closed;
}

export function releaseNode(token) { void releaseLease(token); }

// Explicit close/rebind waits for the Web Locks request promise, which resolves
// after the browser has released the lock, rather than merely resolving the
// callback's release promise. Drop still has the non-blocking release above.
export async function releaseNodeAndWait(token) {
  await releaseLease(token);
}

function database() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('miopunch-browser-network', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('node');
    request.onerror = () => reject(new Error('browser storage unavailable'));
    request.onsuccess = () => resolve(request.result);
    request.onblocked = () => reject(new Error('browser storage blocked by another page'));
  });
}

export async function readNode(token, key = 'state') {
  if (!leases.has(token)) throw new Error('identity lease ended');
  const db = await database();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('node', 'readonly');
    const request = transaction.objectStore('node').get(key);
    transaction.oncomplete = () => { db.close(); resolve(request.result ?? null); };
    transaction.onabort = transaction.onerror = () => { db.close(); reject(new Error('browser storage read failed')); };
  });
}

// The comparison and replacement are in one real IndexedDB transaction. Resolve
// only after oncomplete, never after the put request alone. Stale pages, cleared
// stores and failed commits cannot acknowledge saved admission material.
async function trackSave(token, write) {
  const lease = leases.get(token);
  if (!lease || lease.releasing) throw new Error('identity lease ended');
  const save = write();
  lease.saves.add(save);
  try { await save; }
  finally { lease.saves.delete(save); }
}

const receiptKey = 'admission-receipts';
export async function readAdmissionReceipts(token) {
  return JSON.stringify(await readNode(token, receiptKey) ?? {});
}

export function settleAdmissionReceipt(token, wire) {
  return trackSave(token, () => writeNode(store => {
    updateReceipts(store, receipts => { delete receipts[wire]; });
  }));
}

function updateReceipts(store, update) {
  const request = store.get(receiptKey);
  request.onsuccess = () => {
    const receipts = request.result ?? {};
    update(receipts);
    store.put(receipts, receiptKey);
  };
}

export function saveNode(token, previous, next, receipt) {
  return trackSave(token, () => writeNode(store => {
    const request = store.get('state');
    request.onsuccess = () => {
      if ((request.result ?? null) !== previous) { store.transaction.abort(); return; }
      store.put(next, 'state');
      if (receipt) {
        const [wire, contact] = JSON.parse(receipt);
        updateReceipts(store, receipts => { receipts[wire] = contact; });
      }
    };
  }));
}

async function writeNode(write) {
  const db = await database();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('node', 'readwrite', { durability: 'strict' });
    write(transaction.objectStore('node'));
    transaction.oncomplete = () => { db.close(); resolve(); };
    transaction.onabort = transaction.onerror = () => { db.close(); reject(new Error('browser material commit failed or storage changed')); };
  });
}
