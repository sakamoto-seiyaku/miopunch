<script setup lang="ts">
import { onMounted, onBeforeUnmount, nextTick, ref, shallowRef } from 'vue';
import { DemoNode, encode, decode, digest } from './node';
import TerminalPane from './TerminalPane.vue';
import { WebPane } from './web';
import { diagnosticEvents, logConnection, redactDiagnostics } from './diagnostic-log';
const node = new DemoNode();
const token = ref(''); const status = ref('使用完整链接打开，然后连接节点。');
const connected = ref(false); const connecting = ref(false); const error = ref('');
const browserId = ref(''); const hostId = ref(''); const diagnostics = ref('');
const pathStatus = ref('尚未连接'); const improving = ref(false);
const exporting = ref(false); const exportStatus = ref(''); let latestPaths: any;
const fileProgress = ref(''); const files = shallowRef<any[]>([]); const uploading = ref(false);
const clipboard = ref(''); const clipboardStatus = ref('Linux 无桌面：远端提供文本缓冲，不代表写入了系统剪贴板。');
const clipboardInput = ref<HTMLTextAreaElement>(); const canClipboard = Boolean(navigator.clipboard && window.isSecureContext);
const autoClipboard = ref(false); let clipboardVersion = ''; let localClipboard = '';
const protocol = ref('tcp'); const forwardHost = ref('127.0.0.1'); const forwardPort = ref(0);
const forwards = shallowRef<any[]>([]); const probeText = ref('hello miopunch'); const probeResult = ref('');
const address = ref('');
const webSource = ref(''); const webVersion = ref(0); const webStatus = ref(''); const contentOrigin = ref('');
const terminals = ref<string[]>([]); const attached = ref<string[]>([]);
let web: WebPane | undefined;
let pollTimer: ReturnType<typeof setTimeout>; let closing = false;
async function run(action: () => Promise<void>) { error.value = ''; try { await action(); } catch (reason) { error.value = String(reason); logConnection('ui-error', reason); } }
async function info() {
  const paths = await node.diagnostics();
  latestPaths = paths;
  const selected = paths.session.paths.filter((path: any) => path.selected);
  pathStatus.value = selected.some((path: any) => path.isRtc) ? 'WebRTC 直连'
    : selected.some((path: any) => path.isIp) ? '原生 IP 直连'
    : selected.some((path: any) => path.isRelay) ? 'Relay 中继（WebRTC 尚未成为当前数据路径）'
    : '尚无已选数据路径';
  diagnostics.value = JSON.stringify({ browserId: node.id, hostId: hostId.value, requests: node.requests, saved: node.status, paths }, null, 2);
}
async function exportLog() {
  exporting.value = true; exportStatus.value = '';
  let paths = latestPaths; let snapshotError: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    if (connected.value) {
      try {
        paths = await Promise.race([node.diagnostics(), new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error('状态采集超时，使用最近记录')), 2500);
        })]);
        latestPaths = paths;
      } catch (reason) { snapshotError = String(reason); }
      finally { clearTimeout(timer); }
    }
    logConnection('diagnostics-export');
    const payload = redactDiagnostics({
      format: 'miopunch-browser-diagnostics-v1', exportedAt: new Date().toISOString(),
      environment: { origin: location.origin, userAgent: navigator.userAgent,
        secureContext: window.isSecureContext, online: navigator.onLine },
      connection: { connected: connected.value, status: status.value, path: pathStatus.value,
        error: error.value, browserId: node.id, hostId: hostId.value, requests: node.requests },
      events: diagnosticEvents(), paths: paths ?? null, snapshotError,
    }, [token.value.trim(), node.configuration?.code, node.configuration?.webChannel].filter(Boolean));
    const url = URL.createObjectURL(new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' }));
    const link = document.createElement('a'); link.href = url;
    link.download = `miopunch-diagnostics-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    exportStatus.value = '诊断日志已导出';
  } catch (reason) { exportStatus.value = `导出失败：${String(reason)}`; }
  finally { exporting.value = false; }
}
async function improve() {
  if (improving.value) return;
  improving.value = true; pathStatus.value = '正在重新探测直连…';
  let failure: unknown;
  try { await node.improve(); } catch (reason) { failure = reason; }
  try { await info(); }
  catch (reason) { pathStatus.value = '路径状态暂不可用'; failure ||= reason; }
  finally { improving.value = false; }
  if (failure) throw failure;
}
async function connect() {
  connecting.value = true;
  await run(async () => {
    await node.connect(token.value.trim(), message => { status.value = message; });
    connected.value = true; browserId.value = node.id; hostId.value = node.configuration.peer;
    sessionStorage.setItem('miopunch-demo-access', token.value.trim());
    contentOrigin.value = node.configuration.contentOrigin; address.value = node.configuration.exampleWeb;
    forwardPort.value = node.configuration.echoTcp;
    status.value = '已连接 · 成员资格已验证并保存';
    await refreshFiles(); await refreshTerminals(); await refreshForwards(); await info();
    void pollClipboard();
    web = new WebPane(node, token.value.trim(), message => { webStatus.value = message; }); web.start();
  });
  connecting.value = false;
}
async function refreshFiles() { files.value = (await node.rpc({ op: 'file.list' })).files; }
async function upload(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]; if (!file) return;
  uploading.value = true;
  await run(async () => {
    if (file.size > 128 * 1024 * 1024) throw new Error('演示文件上限 128 MiB');
    fileProgress.value = '计算 SHA-256…';
    const sha256 = await digest(await file.arrayBuffer());
    const transfer = await node.rpc({ op: 'file.begin', name: file.name, size: file.size, sha256 });
    let offset = 0;
    while (offset < file.size) {
      const data = encode(new Uint8Array(await file.slice(offset, offset + 24000).arrayBuffer()));
      let result;
      try { result = await node.rpc({ op: 'file.write', id: transfer.id, offset, data }); }
      catch { result = await node.rpc({ op: 'file.write', id: transfer.id, offset, data }); }
      offset = result.offset; fileProgress.value = `发送 ${Math.round(offset / file.size * 100)}% · ${offset.toLocaleString()} 字节`;
    }
    await node.rpc({ op: 'file.finish', id: transfer.id });
    fileProgress.value = `${file.name} 已保存，远端 SHA-256 一致`; await refreshFiles(); await info();
  });
  uploading.value = false; (event.target as HTMLInputElement).value = '';
}
async function download(file: any) {
  await run(async () => {
    const chunks: Uint8Array<ArrayBuffer>[] = []; let offset = 0;
    do { const result = await node.rpc({ op: 'file.read', id: file.id, offset }); chunks.push(decode(result.data)); offset = result.offset; if (result.done) break; } while (offset < file.size);
    const blob = new Blob(chunks); if (await digest(await blob.arrayBuffer()) !== file.sha256) throw new Error('下载校验失败');
    const url = URL.createObjectURL(blob); const link = document.createElement('a'); link.href = url; link.download = file.name; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    fileProgress.value = `${file.name} 下载校验通过`;
  });
}
async function sendClipboard() {
  const result = await node.rpc({ op: 'clipboard.set', text: clipboard.value }); clipboardVersion = result.version;
  clipboardStatus.value = '已写入 Linux 文本缓冲';
}
async function pollClipboard() {
  try {
    if (autoClipboard.value && document.hasFocus()) {
      const local = await navigator.clipboard.readText();
      if (local !== localClipboard) { clipboard.value = local; localClipboard = local; await sendClipboard(); }
    }
    const result = await node.rpc({ op: 'clipboard.get' });
    if (result.version !== clipboardVersion) {
      clipboard.value = result.text; clipboardVersion = result.version;
      if (autoClipboard.value && document.hasFocus()) { await navigator.clipboard.writeText(result.text); localClipboard = result.text; }
    }
  } catch (reason) { if (autoClipboard.value) { autoClipboard.value = false; clipboardStatus.value = `浏览器未允许自动剪贴板，请使用手动按钮：${String(reason)}`; } }
  if (!closing) pollTimer = setTimeout(pollClipboard, 1000);
}
async function pasteClipboard() { clipboard.value = await navigator.clipboard.readText(); await sendClipboard(); }
async function copyClipboard() {
  const result = await node.rpc({ op: 'clipboard.get' }); clipboard.value = result.text;
  if (canClipboard) await navigator.clipboard.writeText(result.text);
  else { await nextTick(); clipboardInput.value!.focus(); clipboardInput.value!.select(); if (!document.execCommand('copy')) throw new Error('请手动复制输入框中的文本'); }
  clipboardStatus.value = '已写入本机系统剪贴板';
}
async function refreshForwards() { forwards.value = (await node.rpc({ op: 'forward.list' })).forwards; }
async function createForward() { await node.rpc({ op: 'forward.create', protocol: protocol.value, host: forwardHost.value, port: Number(forwardPort.value) }); await refreshForwards(); }
async function stopForward(id: string) { await node.rpc({ op: 'forward.stop', id }); await refreshForwards(); }
async function probe(id: string) { const result = await node.rpc({ op: 'forward.probe', id, data: encode(new TextEncoder().encode(probeText.value)) }); probeResult.value = new TextDecoder().decode(decode(result.data)); }
function useEcho() { forwardHost.value = '127.0.0.1'; forwardPort.value = protocol.value === 'tcp' ? node.configuration.echoTcp : node.configuration.echoUdp; }
async function openWeb() {
  if (!web) throw new Error('请先连接节点');
  webSource.value = await web.open(address.value); webVersion.value += 1;
}
async function refreshTerminals() { terminals.value = (await node.rpc({ op: 'terminal', action: 'list' })).sessions; }
async function newTerminal() {
  const name = `term-${Date.now().toString(36)}`;
  await node.rpc({ op: 'terminal', action: 'attach', name }); attached.value.push(name); await refreshTerminals();
}
function reattach(name: string) { if (!attached.value.includes(name)) attached.value.push(name); }
async function endTerminal(name: string) { attached.value = attached.value.filter(x => x !== name); await node.rpc({ op: 'terminal', action: 'end', name }); await refreshTerminals(); }
onMounted(() => {
  const supplied = new URLSearchParams(location.hash.slice(1)).get('access');
  token.value = supplied || sessionStorage.getItem('miopunch-demo-access') || '';
  if (supplied) history.replaceState(null, '', location.pathname);
});
onBeforeUnmount(() => { closing = true; clearTimeout(pollTimer); web?.stop(); void node.close(); });
// Public demo boundary for remote acceptance checks, confined to this origin.
Object.assign(window, { demo: { node, rpc: node.rpc, openWeb, refreshFiles, refreshForwards, refreshTerminals } });
</script>
<template>
  <header><h1>miopunch <small>基础功能演示</small></h1><p>Vue 共享界面 · 独立浏览器节点 ↔ Linux 原生节点</p></header>
  <main>
    <section><h2>连接设备</h2><div class="row"><input v-model="token" type="password" placeholder="完整演示链接中的访问凭据" aria-label="访问凭据" :disabled="connected"><button id="connect" :disabled="connecting || connected || !token" @click="connect">{{ connected ? '已连接' : connecting ? '连接中…' : '连接 Linux 节点' }}</button></div><p id="status">{{ status }}</p><p v-if="connected" class="muted">浏览器 {{ browserId.slice(0, 16) }} · Linux {{ hostId.slice(0, 16) }}</p><div v-if="connected" class="row"><strong id="path-status">当前数据路径：{{ pathStatus }}</strong><button :disabled="improving" @click="run(info)">刷新路径</button><button :disabled="improving" @click="run(improve)">重新探测直连</button></div><p v-if="error" role="alert" class="error">{{ error }}</p><div class="row"><button id="export-log" :disabled="exporting" @click="exportLog">{{ exporting ? '正在导出…' : '导出诊断日志' }}</button><span class="muted">{{ exportStatus }}</span></div><details v-if="connected"><summary @click="run(info)">节点状态</summary><pre>{{ diagnostics }}</pre></details></section>
    <div v-if="connected" class="modules">
      <section><h2>文件传递</h2><p class="muted">发送给 Linux 并保存，可下载取回验证；单个文件最多 128 MiB。</p><input aria-label="选择发送文件" type="file" :disabled="uploading" @change="upload"><p>{{ fileProgress }}</p><button @click="run(refreshFiles)">刷新已接收文件</button><ul><li v-for="file in files" :key="file.id">{{ file.name }} · {{ file.size.toLocaleString() }} 字节 <button @click="download(file)">下载</button></li></ul></section>
      <section><h2>剪贴板</h2><p class="muted">{{ clipboardStatus }}</p><textarea ref="clipboardInput" v-model="clipboard" aria-label="剪贴板文本" rows="4" placeholder="也可以直接输入文本测试传输"></textarea><div class="row"><button @click="run(sendClipboard)">发送文本</button><button :disabled="!canClipboard" @click="run(pasteClipboard)">读取本机并发送</button><button @click="run(copyClipboard)">取回并复制到本机</button></div><label><input v-model="autoClipboard" type="checkbox" :disabled="!canClipboard"> 自动访问系统剪贴板（需要安全上下文与浏览器许可）</label><p v-if="!canClipboard" class="muted">HTTP 页面请手动粘贴文本后发送；可以取回并复制。</p></section>
      <section class="wide"><h2>TCP / UDP 转发</h2><p class="muted">监听建立在 Linux 的 127.0.0.1 上，供该主机应用访问。浏览器可发送一次真实 socket 探测；不会在你的本机创建监听端口。目标限节点可达的内网或回环地址。</p><div class="row"><select v-model="protocol" aria-label="转发协议" @change="useEcho"><option value="tcp">TCP</option><option value="udp">UDP</option></select><input v-model="forwardHost" aria-label="目标主机" placeholder="目标主机"><input v-model.number="forwardPort" aria-label="目标端口" type="number" min="1" max="65535"><button @click="run(createForward)">创建转发</button><button @click="useEcho">使用内置回显服务</button></div><input v-model="probeText" aria-label="探测内容"><ul><li v-for="rule in forwards" :key="rule.id">{{ rule.protocol.toUpperCase() }} · Linux {{ rule.listen }} → {{ rule.target }} <button @click="run(() => probe(rule.id))">发送探测</button><button class="danger" @click="run(() => stopForward(rule.id))">停止</button></li></ul><pre v-if="probeResult">返回：{{ probeResult }}</pre></section>
      <section class="wide"><h2>内网 Web</h2><p class="muted">输入 Linux 节点可达的 HTTP(S) 内网地址。演示支持页面、资源、表单与节点侧 Cookie；尚未实现 WebSocket，复杂站点可能不兼容。</p><div class="row"><input v-model="address" aria-label="内网网页地址"><button id="open-web" @click="run(openWeb)">打开网页</button><button @click="address = node.configuration.exampleWeb">使用演示站点</button></div><p class="muted">{{ webStatus }}</p><iframe :key="webVersion" :src="webSource || undefined" title="通过节点打开的网页" sandbox="allow-scripts allow-forms allow-same-origin"></iframe></section>
      <section class="wide"><h2>远程终端</h2><p class="muted">真实 PTY + tmux；运行在 Linux 上的独立演示容器内，以普通用户执行。关闭视图保留工作区，可重新附着；不会进入宿主的管理 shell。</p><div class="row"><button id="new-terminal" @click="run(newTerminal)">新建终端</button><button @click="run(refreshTerminals)">刷新工作区</button><button v-for="name in terminals" :key="name" :disabled="attached.includes(name)" @click="reattach(name)">{{ name }} · 附着</button></div><TerminalPane v-for="name in attached" :key="name" :rpc="node.rpc" :name="name" @close="attached = attached.filter(x => x !== name)" @end="run(() => endTerminal(name))" /></section>
    </div>
  </main>
</template>
