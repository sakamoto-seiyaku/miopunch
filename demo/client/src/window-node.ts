// The SharedWorker grants this Window exclusive ownership before calling start.
// This bounded HTTP host adapter only supplies the one SDK lock operation;
// storage, durable CAS and the SDK itself remain the pinned implementation.
import { observeRtc, rtcDiagnostics } from './rtc-diagnostics';
let network: any, member: any, peer: string | undefined, sdk: any;
let rtcAttempt: any;
let pathAttempt: Promise<any> | undefined;
let lifecycle = Promise.resolve();
function transition(action: () => Promise<any>): Promise<any> {
  const result = lifecycle.then(action);
  lifecycle = result.catch(() => {});
  return result;
}
function httpIdentityLease() {
  if (navigator.locks) return;
  let held = false;
  Object.defineProperty(navigator, 'locks', { value: { async request(name: string, options: any, callback: (lock: any) => Promise<any>) {
    if (name !== 'miopunch-browser-network-v1' || options.ifAvailable !== true) throw new Error('unsupported demo identity lock');
    if (held) return callback(null);
    held = true;
    try { return await callback({ name, mode: 'exclusive' }); }
    finally { held = false; }
  } } });
}
async function connect(configuration: any, report: (message: string) => void) {
  if (peer && peer !== configuration.peer) throw new Error('demo node already belongs to another host');
  if (!network) {
    httpIdentityLease();
    observeRtc();
    const path = '/pkg/miopunch_browser_probe.js';
    report('正在加载浏览器节点…');
    sdk ||= await import(/* @vite-ignore */ path);
    await sdk.default();
    report('正在启动浏览器节点…');
    network = await sdk.BrowserNetwork.start(JSON.stringify({
      relays: [configuration.relay], disabled: [], rtc_enabled: true,
      rtc_ice: configuration.rtcIce,
      path_selection: { preference: 'PreferDirect', reevaluation_interval_ms: 1000 },
    }));
    peer = configuration.peer;
  }
  const saved = JSON.parse(network.status());
  if (!saved.authorization_saved || !saved.secret_saved) {
    report('正在兑换邀请…');
    await network.request_precode(configuration.code);
    report('正在保存成员资格…');
    await network.complete_admission();
  }
  if (!member || member.is_closed()) {
    report('正在建立成员连接并尝试 WebRTC…');
    member = await network.open_member_connection(configuration.peer);
  }
  const capability = JSON.parse(network.path_diagnostics()).rtcCapability;
  const paths = JSON.parse(member.session_diagnostics()).paths;
  if (capability.effective && !paths.some((path: any) => path.selected && path.isRtc)) {
    report('正在尝试 WebRTC 直连…');
    try { await improve(); } catch { /* Keep the usable member path on failure. */ }
  }
  return { id: network.id(), status: JSON.parse(network.status()) };
}
async function rpc(request: Record<string, unknown>) {
  if (!member || member.is_closed()) throw new Error('浏览器节点尚未连接');
  return JSON.parse(await member.exchange(JSON.stringify(request)));
}
async function diagnostics() {
  if (!network || !member) throw new Error('浏览器节点尚未连接');
  const local = {
    runtime: 'window-rtc-v2', saved: JSON.parse(network.status()), rtcAttempt,
    browserRtc: await rtcDiagnostics(), network: JSON.parse(network.path_diagnostics()),
    session: JSON.parse(member.session_diagnostics()),
  };
  // A failed member RPC must not discard the browser's ICE failure history.
  let native;
  try { native = await rpc({ op: 'connection.info' }); }
  catch (error) { native = { error: String(error) }; }
  return {
    ...local, native,
  };
}
async function improve(): Promise<any> {
  if (!member || member.is_closed()) throw new Error('浏览器节点尚未连接');
  if (pathAttempt) return pathAttempt;
  pathAttempt = (async () => {
    try {
      const paths = JSON.parse(member.session_diagnostics()).paths;
      // The SDK owns one RTC association per member session. A healthy selected
      // RTC path needs no second association; refresh the actual path instead.
      const result = paths.some((path: any) => path.selected && path.isRtc)
        ? { outcome: 'already-direct' }
        : JSON.parse(await member.improve_path(30000));
      rtcAttempt = { ok: true, result };
      return result;
    } catch (error) { rtcAttempt = { ok: false, error: String(error) }; throw error; }
  })();
  try { return await pathAttempt; } finally { pathAttempt = undefined; }
}
export async function hostCall(action: string, values: any, report: (message: string) => void): Promise<any> {
  if (action === 'connect') return transition(() => connect(values.configuration, report));
  if (action === 'close') return transition(async () => {
    member?.close();
    await network?.close();
    network = member = peer = rtcAttempt = undefined;
    return { closed: true };
  });
  if (action === 'rpc') return rpc(values.request);
  if (action === 'diagnostics') return diagnostics();
  if (action === 'improve') {
    const attempt = await improve();
    return { attempt, ...await diagnostics() };
  }
  throw new Error('unknown demo host operation');
}
