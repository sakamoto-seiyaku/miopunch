// Bounded protocol observation only; no MIO/QUIC/RTC and no human channels.
import { chromium } from '../../../browser-probe/harness/node_modules/playwright-core/index.mjs';
import tls from 'node:tls';
import { randomBytes } from 'node:crypto';
import { writeFileSync, existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
const report = { startedUtc: new Date().toISOString(), service: 'testnet.ergo.chat', normalCertificateValidation: true, scope: 'Two owned nicks; two small PRIVMSG; one synthetic 1200-byte MIR1-shaped fragment series; no channels, secrets, QUIC or RTC', native: {}, browser: {}, observations: [] };
const suffix = randomBytes(5).toString('hex');
const nativeNick = 'mion' + suffix, browserNick = 'miob' + suffix;
let browser, socket, deadline;
const lines = [];
const wait = async (predicate, label, ms = 15000) => {
  const start = Date.now();
  while (!predicate()) { if (Date.now() - start > ms) throw new Error(label + ' timeout'); await new Promise(r => setTimeout(r, 50)); }
};
try {
  deadline = setTimeout(() => { socket?.destroy(); browser?.close(); }, 55000);
  let executablePath = chromium.executablePath();
  if (!existsSync(executablePath)) {
    const cache = join(homedir(), '.cache/ms-playwright');
    executablePath = readdirSync(cache).filter(n => n.startsWith('chromium-')).sort().reverse().flatMap(n => ['chrome-linux64/chrome','chrome-linux/chrome'].map(s => join(cache,n,s))).find(existsSync);
  }
  browser = await chromium.launch({ executablePath, headless: true });
  report.browser.version = browser.version();
  const page = await browser.newPage();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Network.enable');
  cdp.on('Network.webSocketHandshakeResponseReceived', e => { report.browser.handshakeStatus = e.response.status; });
  await page.evaluate(nick => {
    window.probe = { lines: [], opened: false, errors: [] };
    const ws = window.probe.ws = new WebSocket('wss://testnet.ergo.chat/webirc', 'text.ircv3.net');
    ws.onopen = () => { window.probe.opened = true; window.probe.protocol = ws.protocol; ws.send('NICK ' + nick); ws.send('USER mio 0 * :bounded interoperability probe'); };
    ws.onmessage = e => { const line = String(e.data); window.probe.lines.push({ line, at: Date.now() }); if (line.startsWith('PING ')) ws.send('PONG ' + line.slice(5)); };
    ws.onerror = () => window.probe.errors.push('websocket error');
  }, browserNick);
  socket = tls.connect({ host: 'testnet.ergo.chat', port: 6697, servername: 'testnet.ergo.chat', rejectUnauthorized: true });
  socket.setTimeout(15000, () => socket.destroy(new Error('native TLS socket timeout')));
  socket.on('error', e => { report.native.error = e.message; });
  await new Promise((resolve,reject) => { socket.once('secureConnect',resolve); socket.once('error',reject); socket.once('close', () => reject(new Error('native TLS closed before ready'))); });
  report.native.authorized = socket.authorized;
  report.native.protocol = socket.getProtocol();
  report.native.peerCertificate = { subject: socket.getPeerCertificate().subject, validTo: socket.getPeerCertificate().valid_to };
  let buffer = '';
  socket.on('data', data => { buffer += data.toString(); let at; while ((at = buffer.indexOf('\n')) >= 0) { const line = buffer.slice(0,at).trimEnd(); buffer = buffer.slice(at+1); lines.push({line,at:Date.now()}); if (line.startsWith('PING ')) socket.write('PONG ' + line.slice(5) + '\r\n'); } });
  socket.write('NICK '+nativeNick+'\r\nUSER mio 0 * :bounded interoperability probe\r\n');
  await wait(() => lines.some(x => x.line.includes(' 001 ')), 'native registration');
  await page.waitForFunction(() => window.probe.lines.some(x => x.line.includes(' 001 ')), undefined, { timeout: 15000 });
  socket.write('PRIVMSG '+browserNick+' :mio-public-probe-native-'+suffix+'\r\n');
  await page.waitForFunction(s => window.probe.lines.some(x => x.line.includes('mio-public-probe-native-'+s)), suffix, {timeout:10000});
  await page.evaluate(({nick,suffix}) => window.probe.ws.send('PRIVMSG '+nick+' :mio-public-probe-browser-'+suffix), {nick:nativeNick,suffix});
  await wait(() => lines.some(x => x.line.includes('mio-public-probe-browser-'+suffix)), 'browser to native');
  report.observations.push({ stage:'WSS-to-native-TLS-small-PRIVMSG-bidirectional',ok:true });
  const payload = Buffer.alloc(1200, 0x5a), target = randomBytes(16).toString('hex'), source = randomBytes(16).toString('hex'), id = randomBytes(8).toString('hex');
  const count = Math.ceil(payload.length/128);
  for(let i=0;i<count;i++) { socket.write(`PRIVMSG ${browserNick} :MIR1 ${target} ${source} ${id} ${i.toString(16).padStart(2,'0')} ${count.toString(16).padStart(2,'0')} ${payload.subarray(i*128,(i+1)*128).toString('hex')}\r\n`); await new Promise(r=>setTimeout(r,600)); }
  await page.waitForFunction(id => window.probe.lines.filter(x=>x.line.includes(' '+id+' ')).length===10,id,{timeout:10000});
  const received = await page.evaluate(id => window.probe.lines.filter(x=>x.line.includes(' '+id+' ')),id);
  report.observations.push({stage:'synthetic-MIR1-shaped-series',ok:true,datagramBytes:1200,fragments:count,sendSpacingMs:600,firstToLastArrivalMs:received.at(-1).at-received[0].at,maxReceivedLineBytes:Math.max(...received.map(x=>Buffer.byteLength(x.line))),matches:Buffer.concat(received.map(x=>Buffer.from(x.line.split(' ').at(-1),'hex'))).equals(payload),currentThreeSecondReassemblyWouldExpire:received.at(-1).at-received[0].at>=3000});
  report.native.numericCodes = [...new Set(lines.map(x=>x.line.match(/^:\S+ (\d{3}) /)?.[1]).filter(Boolean))];
  report.native.isupport = lines.filter(x=>x.line.includes(' 005 ')).map(x=>x.line.replace(nativeNick,'<owned-nick>'));
  report.browser.protocol = await page.evaluate(()=>window.probe.protocol);
  report.browser.errors = await page.evaluate(()=>window.probe.errors);
  report.ok = true;
  await page.evaluate(()=>{window.probe.ws.send('QUIT :probe complete');window.probe.ws.close();});
  socket.end('QUIT :probe complete\r\n');
} catch(error) { report.ok = false; report.error = String(error); }
finally { clearTimeout(deadline); socket?.destroy(); await browser?.close(); report.finishedUtc = new Date().toISOString(); report.cleanup = 'Owned TLS socket destroyed and browser closed; no accounts or channels created'; writeFileSync(new URL('./irc-entry-observation.json', import.meta.url),JSON.stringify(report,null,2)+'\n'); }
console.log(JSON.stringify(report,null,2));
if(!report.ok) process.exitCode=1;
