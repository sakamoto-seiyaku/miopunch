/* Technical host surface. Native state survives this document and its window. */
const invoke = window.__TAURI__.core.invoke;
const generation = `${Date.now()}-${Math.random()}`;
const status = document.querySelector('#status');
const autostart = document.querySelector('#autostart');
async function refresh() {
  const state = await invoke('host_state');
  status.textContent = JSON.stringify(state, null, 2);
  autostart.checked = state.autostart;
  return state;
}
function showError(error) { status.textContent = String(error); }
document.querySelector('#refresh').onclick = () => refresh().catch(showError);
document.querySelector('#close').onclick = () => invoke('close_window').catch(showError);
document.querySelector('#quit').onclick = () => invoke('quit').catch(showError);
autostart.onchange = () => invoke('set_autostart', {enabled: autostart.checked}).then(refresh).catch(showError);

// An explicit private verification directory enables an external controller to
// submit operations through this actual WebView and its normal Tauri IPC.
async function verificationLoop() {
  const request = await invoke('verification_next');
  if (request) {
    let result;
    try {
      switch (request.action) {
        case 'state': result = await refresh(); break;
        case 'start': result = await invoke('host_start', {mode: request.mode, resources: request.resources ?? null}); break;
        case 'request': result = await invoke('host_request', {input: JSON.stringify(request.request)}); break;
        case 'autostart': result = await invoke('set_autostart', {enabled: request.enabled}); break;
        case 'close': await invoke('close_window', {verificationId: request.id}); return;
        case 'quit': await invoke('quit', {verificationId: request.id}); return;
        default: throw new Error('unknown verification action');
      }
      result = {ok: true, generation, result};
    } catch (error) { result = {ok: false, generation, error: String(error)}; }
    await invoke('verification_result', {id: request.id, result});
  }
  setTimeout(() => verificationLoop().catch(showError), 100);
}
invoke('window_ready', {generation}).then(refresh).then(verificationLoop).catch(showError);
