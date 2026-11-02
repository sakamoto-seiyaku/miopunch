<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { encode, decode, type RPC } from './node';
const props = defineProps<{ rpc: RPC; name: string }>();
const emit = defineEmits<{ close: []; end: [] }>();
const root = ref<HTMLElement>();
const error = ref('');
const ready = ref(false);
let terminal: Terminal;
let fit: FitAddon;
let stopped = false;
let timer: ReturnType<typeof setTimeout>;
let resize: ResizeObserver;
let offset = 0;
let writes: Promise<unknown> = Promise.resolve();
async function readOutput() {
    const result = await props.rpc({ op: 'terminal', action: 'poll', name: props.name, offset });
    if (stopped) return;
    if (result.truncated) terminal.write('\r\n[较早输出已超出缓冲区]\r\n');
    offset = result.offset;
    const bytes = decode(result.data);
    if (bytes.length) await new Promise<void>(resolve => terminal.write(bytes, resolve));
}
async function poll() {
  try {
    await readOutput();
  } catch (reason) { error.value = String(reason); }
  if (!stopped) timer = setTimeout(poll, 100);
}
onMounted(async () => {
  terminal = new Terminal({ disableStdin: true, cursorBlink: true, fontSize: 14, scrollback: 2000, theme: { background: '#111827' } });
  fit = new FitAddon(); terminal.loadAddon(fit); terminal.open(root.value!); fit.fit();
  try {
    const attachment = await props.rpc({ op: 'terminal', action: 'attach', name: props.name });
    if (stopped) { await props.rpc({ op: 'terminal', action: 'detach', name: props.name }); return; }
    terminal.onData(data => {
      // Rendering old PTY bytes can generate device-query replies. Those old
      // requests have already expired at tmux; forwarding them would type the
      // reply into the shell. Only live output may generate terminal input.
      if (!ready.value) return;
      writes = writes.then(() => props.rpc({ op: 'terminal', action: 'write', name: props.name, data: encode(new TextEncoder().encode(data)) }))
        .catch(reason => { error.value = String(reason); });
    });
    do { await readOutput(); } while (!stopped && offset < (attachment.historyEnd || 0));
    if (stopped) return;
    const update = () => {
      fit.fit();
      props.rpc({ op: 'terminal', action: 'resize', name: props.name, cols: terminal.cols, rows: terminal.rows }).catch(() => {});
    };
    resize = new ResizeObserver(update); resize.observe(root.value!); update();
    terminal.options.disableStdin = false; ready.value = true;
    void poll(); terminal.focus();
  } catch (reason) { error.value = String(reason); }
});
onBeforeUnmount(() => {
  stopped = true; clearTimeout(timer); resize?.disconnect(); terminal?.dispose();
  void props.rpc({ op: 'terminal', action: 'detach', name: props.name }).catch(() => {});
});
</script>
<template>
  <div class="terminal-bar"><strong>{{ name }}</strong><span class="muted">{{ ready ? '已附着' : '连接中…' }}</span><span class="spacer"></span><button @click="emit('close')">关闭视图，保留 tmux</button><button class="danger" @click="emit('end')">结束工作区</button></div>
  <p v-if="error" class="error">{{ error }}</p><div ref="root" class="terminal"></div>
</template>
