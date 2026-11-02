import { DemoNode, encode, decode } from './node';

/** HTTP-compatible page carrier. Only HTTP-shaped events enter this adapter;
 * the selected site and RPC operations stay owned by the management origin. */
export class WebPane {
  private site?: { site: string; origin: string; path: string };
  private abort = new AbortController();
  private stopped = false;
  private active = new Set<Promise<void>>();
  constructor(private node: DemoNode, private access: string, private report: (message: string) => void) {}
  start(): void { void this.pump(); }
  stop(): void { this.stopped = true; this.abort.abort(); }
  async open(url: string): Promise<string> {
    this.site = await this.node.rpc({ op: 'web.open', url });
    this.report(`由 Linux 节点访问 ${this.site!.origin}，页面在下方打开`);
    return `${this.node.configuration.contentOrigin}/__open?channel=${encodeURIComponent(this.node.configuration.webChannel)}&path=${encodeURIComponent(this.site!.path)}`;
  }
  private async pump() {
    while (!this.stopped) {
      try {
        if (this.active.size >= 2) await Promise.race(this.active);
        const response = await fetch(`/__web/next?channel=${encodeURIComponent(this.node.configuration.webChannel)}`, {
          headers: { Authorization: `Bearer ${this.access}` }, signal: this.abort.signal,
        });
        if (!response.ok) throw new Error(await response.text());
        const message = await response.json();
        if (message.idle) continue;
        const task = this.serve(message).finally(() => this.active.delete(task));
        this.active.add(task);
      } catch (error) {
        if (this.stopped) return;
        this.report(`网页通道：${String(error)}`);
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  }
  private async serve(message: { id: string; request: Record<string, unknown> }) {
    let result;
    try {
      if (!this.site) throw new Error('请先打开目标地址');
      const request = message.request;
      // Explicit fields prevent content from choosing a management operation.
      const response = await this.node.rpc({ op: 'web.request', site: this.site.site,
        path: request.path, method: request.method, headers: request.headers, body: request.body });
      const chunks: Uint8Array<ArrayBuffer>[] = []; let offset = 0;
      while (offset < response.size) {
        const value = await this.node.rpc({ op: 'web.read', id: response.id, offset });
        if (value.offset <= offset) throw new Error('网页内容读取未前进');
        chunks.push(decode(value.data)); offset = value.offset;
      }
      result = { status: response.status, headers: response.headers, origin: response.origin,
        body: encode(new Uint8Array(await new Blob(chunks).arrayBuffer())) };
    } catch (error) { result = { error: String(error) }; }
    try {
      await fetch(`/__web/reply?channel=${encodeURIComponent(this.node.configuration.webChannel)}`, {
        method: 'POST', headers: { Authorization: `Bearer ${this.access}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: message.id, result }),
      });
    } catch (error) { if (!this.stopped) this.report(`网页响应：${String(error)}`); }
  }
}
