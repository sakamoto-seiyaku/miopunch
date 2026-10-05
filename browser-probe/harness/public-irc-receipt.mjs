// The public IRC runner's concrete evidence boundary. No network work here.
import { createHash } from 'node:crypto';

const hash = data => createHash('sha256').update(data).digest('hex');

export function redactPublicIrcReceipt(value, key = '') {
  if (['invitation', 'id', 'local_id', 'remote_id', 'contact', 'contactId', 'applicant', 'request_id', 'route', 'mqtt_route', 'address', 'stable_id'].includes(key)) return undefined;
  if (key === 'members' && Array.isArray(value)) return value.length;
  if (key === 'fragment') return { sha256: hash(value) };
  if (key === 'paths') return value.map(path => ({ kind: path.startsWith('Custom') ? 'Custom' : 'Other', sha256: hash(path) }));
  if (Array.isArray(value)) return value.map(v => redactPublicIrcReceipt(v));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
    .map(([k, v]) => [k, redactPublicIrcReceipt(v, k)])
    .filter(([, v]) => v !== undefined));
  return value;
}

export function publicIrcCleanupComplete(cleanup) {
  const native = cleanup.nativeReceipt;
  return cleanup.nativeExit?.code === 0
    && cleanup.nativeExit?.signal === null
    && cleanup.forcedKill !== true
    && native?.event === 'closed'
    && native.graceful_close === true
    && native.traffic?.driver_active === 0
    && native.traffic?.pending === 0
    && native.traffic?.protocol?.queue_pending === 0
    && cleanup.activeBrowserIntervals === 0
    && cleanup.immediate?.nativeExited === true
    && cleanup.immediate?.allWebSocketsClosed === true
    && cleanup.late?.nativeExited === true
    && cleanup.late?.allWebSocketsClosed === true;
}
