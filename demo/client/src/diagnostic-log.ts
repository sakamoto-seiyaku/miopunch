// Connection metadata only: never record RPC bodies, console output or config.
const events: { at: string; event: string; detail?: string }[] = [];
export function logConnection(event: string, detail?: unknown) {
  events.push({ at: new Date().toISOString(), event, ...(detail === undefined ? {} : { detail: String(detail).slice(0, 2000) }) });
  if (events.length > 200) events.shift();
}
export function diagnosticEvents() { return events.slice(); }

export function redactDiagnostics(value: unknown, secrets: string[]): unknown {
  if (Array.isArray(value)) return value.map(item => redactDiagnostics(item, secrets));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key,
    /^(authorization|access[-_]?token|token|code|webChannel|password|credential|private[-_]?key|secret|sdp|localDescription|remoteDescription|usernameFragment|ufrag|ice[-_]?pwd)$/i.test(key)
      ? '[redacted]' : redactDiagnostics(item, secrets),
  ]));
  if (typeof value !== 'string') return value;
  let text = value;
  for (const secret of secrets.filter(Boolean)) text = text.split(secret).join('[redacted]');
  return text.replace(/\b(?:https?|wss?):\/\/[^\s"<>]+/gi, address => {
    try { const url = new URL(address); return url.origin + url.pathname; } catch { return '[redacted-url]'; }
  }).replace(/\bBearer\s+[^\s"<>]+/gi, 'Bearer [redacted]')
    .replace(/a=ice-(?:ufrag|pwd):[^\r\n]+/gi, '[redacted-ice-credential]');
}
