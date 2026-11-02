import initialize, { BrowserNetwork } from './pkg/compiled.js';

let node;
let lifecycle = Promise.resolve();
const status = document.querySelector('#status');

function serialize(operation) {
  const result = lifecycle.then(operation);
  lifecycle = result.catch(() => {});
  return result;
}

function snapshot() {
  return node ? { active: true, id: node.id(), status: JSON.parse(node.status()) } : { active: false };
}

function display() {
  const state = snapshot();
  status.textContent = JSON.stringify(state, null, 2);
  return state;
}

const ready = (async () => {
  if (!isSecureContext || !navigator.locks || !globalThis.indexedDB) {
    throw new Error('Use HTTPS or localhost with IndexedDB and Web Locks enabled.');
  }
  await initialize();
  return display();
})();

function start(mode, configuration = {}, invitation) {
  return serialize(async () => {
    await ready;
    if (node) throw new Error('This page already owns a node.');
    const config = JSON.stringify(configuration);
    switch (mode) {
      case 'create': node = await BrowserNetwork.create(config); break;
      case 'open': node = await BrowserNetwork.start(config); break;
      case 'join': node = await BrowserNetwork.join(config, invitation); break;
      case 'guest': node = await BrowserNetwork.join_guest(config, invitation); break;
      default: throw new Error('Unknown node start mode.');
    }
    return display();
  });
}

function close() {
  return serialize(async () => {
    if (node) {
      await node.close();
      node.free();
      node = undefined;
    }
    return display();
  });
}

// The SDK itself owns the actual Web Lock and transactional IndexedDB state.
// The server receives static-asset requests only; there is no native proxy.
Object.defineProperty(globalThis, 'miopunch', { value: Object.freeze({
  ready, create: configuration => start('create', configuration),
  open: configuration => start('open', configuration),
  join: (configuration, invitation) => start('join', configuration, invitation),
  joinGuest: (configuration, invitation) => start('guest', configuration, invitation),
  close, status: snapshot, get node() { return node; },
}) });

const failure = error => { status.textContent = String(error); };
document.querySelector('#open').onclick = () => start('open').catch(failure);
document.querySelector('#close').onclick = () => close().catch(failure);
ready.catch(failure);
