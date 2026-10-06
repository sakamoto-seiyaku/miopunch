// Read metadata from the existing owned page's CDP connection. Never publish
// WebSocket frames, HTTP headers, URLs with paths, SDP or private values.
import { pathToFileURL } from 'node:url';

export function networkObservation(message, sockets) {
  const { method, params = {} } = message;
  if (method === 'Network.webSocketCreated') {
    const url = new URL(params.url);
    const socket = { number: sockets.size + 1, origin: url.origin };
    sockets.set(params.requestId, socket);
    return { event: 'socket-created', ...socket };
  }
  const socket = sockets.get(params.requestId);
  if (!socket) return undefined;
  const base = { socket: socket.number, origin: socket.origin };
  if (method === 'Network.webSocketFrameReceived' || method === 'Network.webSocketFrameSent') {
    const frame = params.response ?? {};
    const opcode = Number.isInteger(frame.opcode) && frame.opcode >= 0 && frame.opcode <= 15
      ? frame.opcode : null;
    const text = typeof frame.payloadData === 'string' ? frame.payloadData : '';
    const bytes = opcode === 2 ? Buffer.from(text, 'base64').length : Buffer.byteLength(text);
    return { event: method.endsWith('Received') ? 'frame-received' : 'frame-sent',
      ...base, timestamp: params.timestamp, opcode, bytes };
  }
  if (method === 'Network.webSocketWillSendHandshakeRequest') {
    return { event: 'handshake-sent', ...base, timestamp: params.timestamp };
  }
  if (method === 'Network.webSocketHandshakeResponseReceived') {
    return { event: 'handshake-response', ...base, timestamp: params.timestamp,
      status: params.response?.status };
  }
  if (method === 'Network.webSocketClosed') {
    return { event: 'socket-closed', ...base, timestamp: params.timestamp };
  }
  if (method === 'Network.webSocketFrameError' || method === 'Network.loadingFailed') {
    const text = params.errorMessage ?? params.errorText ?? '';
    const networkCode = text.match(/\bnet::[A-Z0-9_]+\b/)?.[0];
    const httpStatus = text.match(/Unexpected response code:\s*(\d{3})/)?.[1];
    return { event: 'socket-error', ...base, timestamp: params.timestamp,
      networkCode: networkCode ?? null, handshakeHttpStatus: httpStatus ? Number(httpStatus) : null,
      cancelled: params.canceled === true };
  }
  return undefined;
}

function main() {
  const address = new URL(process.argv[2]);
  if (address.protocol !== 'ws:' || address.hostname !== '127.0.0.1') {
    throw new Error('CDP observer requires the existing local owned forward');
  }
  const output = value => process.stdout.write(JSON.stringify(value) + '\n');
  const sockets = new Map();
  const connection = new WebSocket(address);
  let stopping = false;
  let closeTimer;
  function stop() {
    if (stopping) return;
    stopping = true;
    connection.close();
    closeTimer = setTimeout(() => {
      output({ event: 'observer-close-timeout' });
      process.exit(1);
    }, 2000);
  }
  process.stdin.on('data', stop);
  process.stdin.on('end', stop);
  connection.addEventListener('open', () => {
    connection.send(JSON.stringify({ id: 1, method: 'Network.enable' }));
  });
  connection.addEventListener('message', event => {
    try {
      const message = JSON.parse(event.data);
      if (message.id === 1) {
        output({ event: 'observer-ready', networkEnabled: !message.error });
        return;
      }
      const observed = networkObservation(message, sockets);
      if (observed) output(observed);
    } catch {
      output({ event: 'observer-invalid-message' });
    }
  });
  connection.addEventListener('error', () => { output({ event: 'observer-control-error' }); });
  connection.addEventListener('close', event => {
    clearTimeout(closeTimer);
    output({ event: 'observer-closed', code: event.code, wasClean: event.wasClean });
    process.stdin.pause();
    if (!stopping) process.exitCode = 1;
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
