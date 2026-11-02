import { sha256 } from '@noble/hashes/sha2.js';
import { hostCall } from './window-node';
import { logConnection } from './diagnostic-log';
export type Configuration = { peer: string; code: string; relay: string; contentOrigin: string; webChannel: string; exampleWeb: string; echoTcp: number; echoUdp: number; rtcIce: { stun_servers: string[] } };
export type RPC = (request: Record<string, unknown>) => Promise<any>;

export function encode(bytes: Uint8Array): string {
  let value = '';
  for (const byte of bytes) value += String.fromCharCode(byte);
  return btoa(value);
}
export function decode(value: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(atob(value), x => x.charCodeAt(0));
}
export async function digest(bytes: ArrayBuffer): Promise<string> {
  return Array.from(sha256(new Uint8Array(bytes)), x => x.toString(16).padStart(2, '0')).join('');
}

export class DemoNode {
  private worker?: SharedWorker;
  private workerId = '';
  private workerStatus: unknown = null;
  private nextRequest = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  configuration!: Configuration;
  requests = 0;
  get id(): string { return this.workerId; }
  get status(): unknown { return this.workerStatus; }
  private workerCall(message: Record<string, unknown>): Promise<any> {
    if (this.pending.size >= 64) return Promise.reject(new Error('浏览器节点请求过多'));
    const id = ++this.nextRequest;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id); logConnection(`${message.action}-timeout`); reject(new Error('浏览器节点请求超时'));
      }, 70000);
      this.pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => {
        clearTimeout(timer); logConnection(`${message.action}-failed`, error); reject(error);
      } });
      this.worker!.port.postMessage({ id, ...message });
    });
  }
  async connect(token: string, report: (message: string) => void): Promise<void> {
    logConnection('connect-start');
    report('读取演示邀请…');
    const response = await fetch('/__config', { headers: { Authorization: `Bearer ${token}` } });
    if (!response.ok) throw new Error('访问凭据无效，请使用提供的完整演示链接');
    this.configuration = await response.json();
    report('启动独立浏览器节点…');
    if (!this.worker) {
      if (typeof SharedWorker === 'undefined') throw new Error('此浏览器不支持 HTTP 演示所需的 SharedWorker，请使用桌面 Chrome、Firefox 或 Safari');
      this.worker = new SharedWorker('/node-worker.js', { name: 'miopunch-demo-node-v1', type: 'module' });
      this.worker.port.onmessage = event => {
        if (event.data.event === 'hostCall') {
          void hostCall(event.data.action, event.data, status => this.worker!.port.postMessage({ action: 'hostStatus', status })).then(
            result => this.worker!.port.postMessage({ action: 'hostReply', call: event.data.call, result }),
            error => this.worker!.port.postMessage({ action: 'hostReply', call: event.data.call, error: String(error) }),
          );
          return;
        }
        if (event.data.event === 'hostLost') {
          for (const item of this.pending.values()) item.reject(new Error('浏览器节点宿主已关闭，请关闭所有演示页面后重新打开'));
          this.pending.clear(); return;
        }
        if (event.data.event === 'status') { report(event.data.status); return; }
        const pending = this.pending.get(event.data.id); if (!pending) return;
        this.pending.delete(event.data.id);
        if (event.data.error) pending.reject(new Error(event.data.error)); else pending.resolve(event.data.result);
      };
      this.worker.onerror = () => { for (const item of this.pending.values()) item.reject(new Error('HTTP 浏览器节点启动失败')); this.pending.clear(); };
      this.worker.port.start();
      addEventListener('pagehide', () => this.worker?.port.postMessage({ action: 'host-pagehide' }));
    }
    let version;
    try { version = await this.workerCall({ action: 'version' }); }
    catch { throw new Error('请关闭所有旧版演示页面，再打开链接启用 WebRTC'); }
    if (version.runtime !== 'window-rtc-v2' || version.lost) throw new Error('请关闭所有旧版演示页面，再打开链接启用 WebRTC');
    report('连接 Linux 节点并协商 WebRTC…');
    const result = await this.workerCall({ action: 'connect', configuration: this.configuration });
    this.workerId = result.id; this.workerStatus = result.status;
    await this.rpc({ op: 'info' });
    logConnection('connect-complete', this.id);
  }
  rpc: RPC = async request => {
    if (!this.worker) throw new Error('请先连接节点');
    const result = await this.workerCall({ action: 'rpc', request });
    this.requests += 1;
    if (result.error) throw new Error(result.error);
    return result;
  };
  async diagnostics(): Promise<any> {
    const result = await this.workerCall({ action: 'diagnostics' });
    this.workerStatus = result.saved;
    return result;
  }
  async improve(): Promise<any> {
    logConnection('improve-start');
    const result = await this.workerCall({ action: 'improve' });
    logConnection('improve-complete');
    return result;
  }
  async close(): Promise<void> {
    // A detached UI can still host the one runtime for another attached page.
    // Keep its broker port until the Window itself closes.
    if (this.worker) await this.workerCall({ action: 'close' });
  }
}
