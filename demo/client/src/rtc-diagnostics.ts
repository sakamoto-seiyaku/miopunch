// Read-only, bounded RTC observations for this demo's owning Window. Never
// retain a PC, serialize SDP/ICE credentials, or change its configuration.
type Observation = {
  pc?: WeakRef<RTCPeerConnection>;
  started: number;
  states: Record<string, unknown>[];
  candidates: Record<string, unknown>[];
};
const observations: Observation[] = [];
let installed = false;
export function observeRtc() {
  if (installed || typeof RTCPeerConnection === 'undefined') return;
  installed = true;
  const Original = RTCPeerConnection;
  class Observed extends Original {
    constructor(configuration?: RTCConfiguration) {
      super(configuration);
      const observation: Observation = {
        pc: typeof WeakRef === 'undefined' ? undefined : new WeakRef(this),
        started: performance.now(), states: [], candidates: [],
      };
      observations.push(observation);
      if (observations.length > 16) observations.shift();
      const state = (event: string, details: Record<string, unknown> = {}) => {
        observation.states.push({ event, elapsedMs: Math.round(performance.now() - observation.started),
          ice: this.iceConnectionState, connection: this.connectionState,
          gathering: this.iceGatheringState, ...details });
        if (observation.states.length > 48) observation.states.shift();
      };
      for (const event of ['iceconnectionstatechange', 'connectionstatechange', 'icegatheringstatechange']) {
        this.addEventListener(event, () => state(event));
      }
      this.addEventListener('icecandidate', event => {
        const candidate = event.candidate;
        if (candidate && observation.candidates.length < 24) observation.candidates.push({
          type: candidate.type, protocol: candidate.protocol, address: candidate.address, port: candidate.port,
        });
      });
      this.addEventListener('icecandidateerror', event => state('icecandidateerror', {
        code: event.errorCode, text: String(event.errorText || '').slice(0, 256), url: event.url,
      }));
      state('created');
    }
  }
  try {
    Object.defineProperty(window, 'RTCPeerConnection', { value: Observed, configurable: true, writable: true });
  } catch { /* Optional diagnostics must not block an existing browser policy. */ }
}
export async function rtcDiagnostics() {
  return Promise.all(observations.map(async observation => {
    const stats: Record<string, unknown>[] = [];
    const pc = observation.pc?.deref();
    try {
      if (pc && pc.connectionState !== 'closed') (await pc.getStats()).forEach(value => {
        const fields: Record<string, string[]> = {
          'candidate-pair': ['id', 'localCandidateId', 'remoteCandidateId', 'state', 'nominated',
            'bytesSent', 'bytesReceived', 'requestsSent', 'responsesReceived', 'requestsReceived', 'responsesSent'],
          'local-candidate': ['id', 'candidateType', 'address', 'port', 'protocol'],
          'remote-candidate': ['id', 'candidateType', 'address', 'port', 'protocol'],
          'transport': ['id', 'selectedCandidatePairId', 'dtlsState', 'iceState'],
          'data-channel': ['id', 'state', 'messagesSent', 'messagesReceived', 'bytesSent', 'bytesReceived'],
        };
        if (fields[value.type] && stats.length < 48) stats.push(Object.fromEntries(
          ['type', ...fields[value.type]].filter(key => value[key] !== undefined).map(key => [key, value[key]]),
        ));
      });
    } catch { /* Closed/retired PCs retain their event history only. */ }
    return { states: observation.states.slice(), candidates: observation.candidates.slice(), stats };
  }));
}
