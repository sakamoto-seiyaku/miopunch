// One startup runtime check. No SDP, gathering or network request is started.
export function probeRtcCapability(permitted, label, protocol) {
  const result = {
    supported: null, permitted, effective: false, reason: 'disabled-by-policy',
    probe: { pcCreated: false, channelCreated: false, pcClosed: true, channelCloseRequested: false },
  };
  // A disabled resource is not even inspected, so forbidden constructors and
  // missing APIs cannot turn an explicit policy decision into a startup error.
  if (!permitted) return JSON.stringify(result);
  result.supported = false;
  result.reason = 'api-missing';
  let PeerConnection;
  try {
    PeerConnection = globalThis.RTCPeerConnection;
    const required = ['createDataChannel', 'createOffer', 'setLocalDescription',
      'setRemoteDescription', 'getStats', 'close'];
    if (typeof PeerConnection !== 'function'
      || required.some(name => typeof PeerConnection.prototype?.[name] !== 'function')) {
      return JSON.stringify(result);
    }
  } catch {
    return JSON.stringify(result);
  }
  let peer, channel;
  try {
    result.reason = 'peer-connection-rejected';
    peer = new PeerConnection({ iceServers: [] });
    result.probe.pcCreated = true;
    result.probe.pcClosed = false;
    result.reason = 'data-channel-rejected';
    channel = peer.createDataChannel(label, {
      negotiated: true, id: 0, ordered: false, maxRetransmits: 0, protocol,
    });
    result.probe.channelCreated = true;
    result.reason = 'channel-configuration-unsupported';
    channel.binaryType = 'arraybuffer';
    const matches = channel.id === 0 && channel.ordered === false && channel.negotiated === true
      && channel.maxRetransmits === 0 && channel.maxPacketLifeTime === null
      && channel.protocol === protocol && channel.label === label
      && channel.binaryType === 'arraybuffer'
      && typeof channel.send === 'function' && typeof channel.close === 'function';
    if (matches) {
      result.supported = true;
      result.reason = 'available';
    }
  } catch {
    // Only a fixed failure category is retained; arbitrary browser exception
    // messages never become part of public diagnostics or persisted identity.
  } finally {
    try {
      if (channel) {
        channel.close();
        result.probe.channelCloseRequested = true;
      }
    } catch {
      result.reason = 'cleanup-failed';
      result.supported = false;
    }
    try {
      if (peer) {
        peer.close();
        result.probe.pcClosed = peer.connectionState === 'closed';
        if (!result.probe.pcClosed) {
          result.reason = 'cleanup-failed';
          result.supported = false;
        }
      }
    } catch {
      result.reason = 'cleanup-failed';
      result.supported = false;
    }
  }
  result.effective = result.supported === true && permitted && result.probe.pcClosed;
  return JSON.stringify(result);
}
