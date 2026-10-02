// One origin owns one node identity. A page holds the Web Lock for its lifetime;
// browser reclamation releases it. Other pages fail promptly rather than opening
// competing endpoints under the same key. No storage or keys go to the server.
const leases = new Map();
let nextLease = 1;

export async function acquireNode() {
  if (!navigator.locks || !globalThis.indexedDB) throw new Error('durable browser storage and Web Locks required');
  return new Promise((resolve, reject) => {
    navigator.locks.request('miopunch-browser-network-v1', { ifAvailable: true }, async lock => {
      if (!lock) { reject(new Error('identity already active in another page')); return; }
      const token = nextLease++;
      await new Promise(release => { leases.set(token, release); resolve(token); });
      leases.delete(token);
    }).catch(reject);
  });
}

export function releaseNode(token) { leases.get(token)?.(); }

function database() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('miopunch-browser-network', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('node');
    request.onerror = () => reject(new Error('browser storage unavailable'));
    request.onsuccess = () => resolve(request.result);
    request.onblocked = () => reject(new Error('browser storage blocked by another page'));
  });
}

export async function readNode(token) {
  if (!leases.has(token)) throw new Error('identity lease ended');
  const db = await database();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('node', 'readonly');
    const request = transaction.objectStore('node').get('state');
    transaction.oncomplete = () => { db.close(); resolve(request.result ?? null); };
    transaction.onabort = transaction.onerror = () => { db.close(); reject(new Error('browser storage read failed')); };
  });
}

// The comparison and replacement are in one real IndexedDB transaction. Resolve
// only after oncomplete, never after the put request alone. Stale pages, cleared
// stores and failed commits cannot acknowledge saved admission material.
export async function saveNode(token, previous, next) {
  if (!leases.has(token)) throw new Error('identity lease ended');
  const db = await database();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('node', 'readwrite', { durability: 'strict' });
    const store = transaction.objectStore('node');
    const request = store.get('state');
    request.onsuccess = () => {
      if ((request.result ?? null) !== previous) { transaction.abort(); return; }
      store.put(next, 'state');
    };
    transaction.oncomplete = () => { db.close(); resolve(); };
    transaction.onabort = transaction.onerror = () => { db.close(); reject(new Error('browser material commit failed or storage changed')); };
  });
}
