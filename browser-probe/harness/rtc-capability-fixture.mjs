// An owned qualification page can inject a host API failure without changing
// device/browser settings. This never changes the production SDK protocol.
export function installRtcCapabilityFixture(mode) {
  const modes = ['observe', 'api-missing', 'peer-connection-denied', 'data-channel-denied'];
  if (!modes.includes(mode)) throw new Error('unknown owned RTC capability fixture');
  if (globalThis.__mio76RtcFixture) throw new Error('owned RTC fixture already installed');
  const originalDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'RTCPeerConnection');
  const OriginalPeerConnection = globalThis.RTCPeerConnection;
  const webSocketDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket');
  const OriginalWebSocket = globalThis.WebSocket;
  const peers = new Set();
  const channels = new Set();
  const sockets = [];
  const started = performance.now();
  const counts = { peerCreated: 0, peerCloseCalls: 0, channelCreated: 0, channelCloseCalls: 0 };
  const bytes = value => typeof value === 'string' ? new TextEncoder().encode(value).byteLength
    : value?.byteLength ?? value?.size ?? 0;
  function ObservedWebSocket(...args) {
    const socket = new OriginalWebSocket(...args);
    const receipt = { socket, origin: new URL(socket.url).origin,
      createdAtMs: performance.now() - started, sentBytes: 0, receivedBytes: 0,
      sentFrames: 0, receivedFrames: 0, errorEvents: 0 };
    sockets.push(receipt);
    const send = socket.send.bind(socket);
    socket.send = value => {
      const result = send(value);
      receipt.sentBytes += bytes(value); receipt.sentFrames++;
      return result;
    };
    socket.addEventListener('message', event => {
      receipt.receivedBytes += bytes(event.data); receipt.receivedFrames++;
    });
    socket.addEventListener('open', () => { receipt.openedAtMs = performance.now() - started; });
    socket.addEventListener('error', () => {
      receipt.errorEvents++; receipt.lastErrorAtMs = performance.now() - started;
    });
    socket.addEventListener('close', event => {
      receipt.closedAtMs = performance.now() - started;
      receipt.closeCode = event.code; receipt.closeWasClean = event.wasClean;
    });
    return socket;
  }
  if (typeof OriginalWebSocket === 'function') {
    ObservedWebSocket.prototype = OriginalWebSocket.prototype;
    Object.setPrototypeOf(ObservedWebSocket, OriginalWebSocket);
    Object.defineProperty(globalThis, 'WebSocket', {
      configurable: true, writable: true, value: ObservedWebSocket,
    });
  }
  function ObservedPeerConnection(...args) {
    if (mode === 'peer-connection-denied') {
      throw new DOMException('owned MIO76 constructor policy injection', 'NotAllowedError');
    }
    const peer = new OriginalPeerConnection(...args);
    peers.add(peer);
    counts.peerCreated++;
    const close = peer.close.bind(peer);
    peer.close = () => { counts.peerCloseCalls++; return close(); };
    const create = peer.createDataChannel.bind(peer);
    peer.createDataChannel = (...channelArgs) => {
      if (mode === 'data-channel-denied') {
        throw new DOMException('owned MIO76 channel policy injection', 'NotAllowedError');
      }
      const channel = create(...channelArgs);
      channels.add(channel);
      counts.channelCreated++;
      const closeChannel = channel.close.bind(channel);
      channel.close = () => { counts.channelCloseCalls++; return closeChannel(); };
      return channel;
    };
    return peer;
  }
  if (typeof OriginalPeerConnection === 'function') {
    ObservedPeerConnection.prototype = OriginalPeerConnection.prototype;
    Object.setPrototypeOf(ObservedPeerConnection, OriginalPeerConnection);
  }
  Object.defineProperty(globalThis, 'RTCPeerConnection', {
    configurable: true, writable: true,
    value: mode === 'api-missing' ? undefined : ObservedPeerConnection,
  });
  globalThis.__mio76RtcFixture = {
    snapshot() {
      return { mode, injected: mode !== 'observe', ...counts,
        openPeerConnections: [...peers].filter(peer => peer.connectionState !== 'closed').length,
        openDataChannels: [...channels].filter(channel => channel.readyState !== 'closed').length,
        sockets: sockets.map(({ socket, ...receipt }) => ({ ...receipt, readyState: socket.readyState })),
        openWebSockets: sockets.filter(({ socket }) => socket.readyState !== 3).length };
    },
    resetCounts() {
      if ([...peers].some(peer => peer.connectionState !== 'closed')
        || [...channels].some(channel => channel.readyState !== 'closed')) {
        throw new Error('owned RTC fixture cannot reset while a PC or DataChannel remains open');
      }
      peers.clear(); channels.clear();
      for (const name of Object.keys(counts)) counts[name] = 0;
    },
    restore() {
      for (const channel of channels) channel.close();
      for (const peer of peers) peer.close();
      for (const { socket } of sockets) socket.close();
      channels.clear(); peers.clear();
      if (originalDescriptor) Object.defineProperty(globalThis, 'RTCPeerConnection', originalDescriptor);
      else delete globalThis.RTCPeerConnection;
      if (webSocketDescriptor) Object.defineProperty(globalThis, 'WebSocket', webSocketDescriptor);
      else delete globalThis.WebSocket;
      delete globalThis.__mio76RtcFixture;
    },
  };
}

export function rtcCapabilityPrelude(mode) {
  return `(${installRtcCapabilityFixture.toString()})(${JSON.stringify(mode)});`;
}
