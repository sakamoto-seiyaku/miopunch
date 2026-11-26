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
export async function saveNode(token, previous, next, key = 'state') {
  const lease = leases.get(token);
  if (!lease || lease.releasing) throw new Error('identity lease ended');
  const save = replaceNode(previous, next, key);
  lease.saves.add(save);
  try { await save; }
  finally { lease.saves.delete(save); }
}

export function readAdmissionReceipt(token) {
  return readNode(token, 'admission-receipt');
}

export function saveAdmissionReceipt(token, previous, next) {
  return saveNode(token, previous, next, 'admission-receipt');
}

async function replaceNode(previous, next, key) {
  const db = await database();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('node', 'readwrite', { durability: 'strict' });
    const store = transaction.objectStore('node');
    const request = store.get(key);
    request.onsuccess = () => {
      if ((request.result ?? null) !== previous) { transaction.abort(); return; }
      store.put(next, key);
    };
    transaction.oncomplete = () => { db.close(); resolve(); };
    transaction.onabort = transaction.onerror = () => { db.close(); reject(new Error('browser material commit failed or storage changed')); };
  });
}
