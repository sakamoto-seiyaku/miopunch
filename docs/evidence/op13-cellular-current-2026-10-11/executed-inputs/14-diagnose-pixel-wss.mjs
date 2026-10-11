import fs from 'node:fs';
const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const address = new URL(input.cdp);
if (address.protocol !== 'ws:' || address.hostname !== '127.0.0.1') throw new Error('owned local CDP required');
const socket = new WebSocket(input.cdp);
const result = await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error('owned diagnostic observer timeout')), 35000);
  socket.addEventListener('error', () => reject(new Error('owned diagnostic CDP failed')), { once: true });
  socket.addEventListener('open', () => {
    const bareExpression = '(async () => { const started = performance.now(); let opened = false; let errors = 0; '
      + 'const ws = new WebSocket(' + JSON.stringify(input.wss) + ', "mqtt"); '
      + 'return await new Promise(resolve => { let settled = false; '
      + 'const done = reason => { if (settled) return; settled = true; clearTimeout(timer); '
      + 'if (ws.readyState < 2) ws.close(1000, "owned diagnostic complete"); '
      + 'resolve({opened, errors, observation: reason, elapsedMs: performance.now()-started}); }; '
      + 'const timer = setTimeout(() => done("observer-deadline"), 15000); '
      + 'ws.addEventListener("open", () => {opened=true;done("open");}); '
      + 'ws.addEventListener("error", () => {errors++;done("error");}); '
      + 'ws.addEventListener("close", () => done("close")); }); })()';
    const carrierExpression = '(async () => { const sdk = await import("/pkg/miopunch_browser_probe.js"); '
      + 'const started = performance.now(); const carrier = sdk.BrowserMqttDatagram.connect('
      + JSON.stringify(JSON.stringify({facility: input.facility, disabled: []})) + '); '
      + 'const work = carrier.wait_ready().then(() => "ready", () => "carrier-error"); '
      + 'let timer; const outcome = await Promise.race([work, new Promise(resolve => {timer = setTimeout(() => resolve("observer-deadline"), 15000);})]); '
      + 'clearTimeout(timer); const diagnostics = JSON.parse(carrier.diagnostics()); delete diagnostics.route; '
      + 'carrier.close(); await work; const afterClose = JSON.parse(carrier.diagnostics()); delete afterClose.route; carrier.free(); '
      + 'return {outcome, elapsedMs:performance.now()-started, diagnostics, afterClose}; })()';
    const expression = input.mode === 'mqtt-carrier' ? carrierExpression : bareExpression;
    socket.send(JSON.stringify({id: 1, method: 'Runtime.evaluate',
      params: {expression, awaitPromise: true, returnByValue: true}}));
  });
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.id === 1) {
      clearTimeout(timer);
      if (message.error || message.result?.exceptionDetails) reject(new Error('owned diagnostic evaluation failed'));
      else resolve(message.result?.result?.value);
    }
  });
});
console.log(JSON.stringify(result));
await new Promise(resolve => {
  const timer = setTimeout(resolve, 2000);
  socket.addEventListener('close', () => {clearTimeout(timer);resolve();}, {once: true});
  socket.close();
});
