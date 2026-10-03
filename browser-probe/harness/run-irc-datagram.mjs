import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createServer as httpsServer } from 'node:https';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { bounded, createOwnedIrcFixtureTools, wasmServer, privateBrowserLaunch } from './owned-irc-fixture.mjs';
const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const argument = name => process.argv[process.argv.indexOf(name) + 1];
const evidence = resolve(argument('--evidence'));
const fixture = resolve(argument('--fixture'));
const nativePort = Number(argument('--native').split(':').at(-1));
const wssPort = Number(argument('--wss').split(':').at(-1));
const report = { startedUtc:new Date().toISOString(),passed:false,normal:[],native:[],websockets:[],faults:{},cleanup:{},budgets:{outerSeconds:60,profileSeconds:30,phaseSeconds:15} };
const nativeProcesses = new Set();
const ownedSockets = new Set();
const servers = [];
let browser, page, stopping;
function save(){writeFileSync(join(evidence,'irc-datagram.json'),JSON.stringify(report,null,2)+'\n');}
const tools=createOwnedIrcFixtureTools({root,fixture,report,nativeProcesses,servers});
const listen=tools.listen;
const nativeFixture=tools.nativeFixture;
async function browserStatus(){return JSON.parse(await page.evaluate(()=>window.carrier.report()));}
async function startBrowser(config){
  assert.equal(await page.evaluate(config=>{window.carrier?.free();window.carrier=window.BrowserIrcDatagram.connect(JSON.stringify(config));try{window.carrier.route();return false;}catch{return true;}},config),true);
  await bounded(page.evaluate(()=>window.carrier.wait_ready()),'browser IRC ready');
  return page.evaluate(()=>window.carrier.route());
}
async function cleanup(){
 if(stopping)return stopping;
 stopping=(async()=>{
  await page?.evaluate(()=>{window.carrier?.close();}).catch(()=>{});
  if(page)report.cleanup.browser=await browserStatus().catch(()=>null);
  await browser?.close();
  await Promise.all([...nativeProcesses].map(async child=>{const exited=new Promise(resolveExit=>child.once('exit',resolveExit));child.stdin.end(JSON.stringify({command:'stop'})+'\n');try{await bounded(exited,'native cleanup',4000);}catch{child.kill('SIGKILL');await exited;}}));
  for(const socket of ownedSockets)socket.destroy();
  await Promise.all(servers.map(server=>new Promise(resolveClose=>server.close(resolveClose))));
  report.cleanup.activeNativeProcesses=nativeProcesses.size;report.cleanup.serversClosed=servers.every(server=>!server.listening);report.cleanup.activeFaultSockets=ownedSockets.size;report.completedUtc=new Date().toISOString();save();
 })();return stopping;
}
process.on('SIGTERM',()=>{report.failure='external deadline/cancellation';cleanup().finally(()=>process.exit(1));});
function wsFrame(text,opcode=1){const bytes=Buffer.from(text);const header=bytes.length<126?Buffer.from([0x80|opcode,bytes.length]):Buffer.from([0x80|opcode,126,bytes.length>>8,bytes.length&255]);return Buffer.concat([header,bytes]);}
function wsRead(socket,consume){let pending=Buffer.alloc(0);return bytes=>{pending=Buffer.concat([pending,bytes]);if(pending.length>8192)return socket.destroy();while(pending.length>=2){const opcode=pending[0]&15;let size=pending[1]&127;let offset=2;if(!(pending[0]&128)||!(pending[1]&128)||size===127)return socket.destroy();if(size===126){if(pending.length<4)return;size=pending.readUInt16BE(2);offset=4;}if(size>4096)return socket.destroy();if(pending.length<offset+4+size)return;const mask=pending.subarray(offset,offset+4);const body=Buffer.from(pending.subarray(offset+4,offset+4+size));for(let i=0;i<body.length;i++)body[i]^=mask[i%4];pending=pending.subarray(offset+4+size);if(opcode===8){socket.end(wsFrame('',8));return;}if(opcode===1)consume(body.toString());else if(opcode===9)socket.write(wsFrame(body,10));else return socket.destroy();}};}
async function localFaults(config){
 report.faults.codec=JSON.parse(await page.evaluate(()=>window.ircFaultProbe()));
 assert.equal(report.faults.codec.missing.expired,1);assert.equal(report.faults.codec.missing.pending,0);assert.equal(report.faults.codec.conflict.rejected,1);assert.equal(report.faults.codec.conflict.delivered,0);assert.equal(report.faults.codec.cancelled.pending,0);
 let faultSocket;let pingAt;let pongAt;let fragments=0;let nick;let resolvePong;const pong=new Promise(resolveValue=>resolvePong=resolveValue);
 const fault=httpsServer({key:readFileSync(join(fixture,'leaf.key')),cert:readFileSync(join(fixture,'leaf.pem'))});
 fault.on('upgrade',(request,socket,head)=>{
  assert.equal(head.length,0);assert.equal(request.headers['sec-websocket-protocol'],'text.ircv3.net');
  ownedSockets.add(socket);socket.once('close',()=>ownedSockets.delete(socket));socket.on('error',()=>{});faultSocket=socket;
  const accept=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\nSec-WebSocket-Protocol: text.ircv3.net\r\n\r\n');
  socket.on('data',wsRead(socket,text=>{for(const line of text.trim().split('\r\n')){
   if(line.startsWith('NICK '))nick=line.slice(5);
   if(line.startsWith('USER ')){socket.write(wsFrame(':fault.test 001 '+nick+' :welcome'));socket.write(wsFrame(':fault.test 005 '+nick+' NICKLEN=32 :supported'));}
   if(line.startsWith('JOIN ')){const channel=line.slice(5);socket.write(wsFrame(':'+nick+'!~u@localhost JOIN :'+channel));socket.write(wsFrame(':fault.test 366 '+nick+' '+channel+' :names complete'));}
   if(line.startsWith('PRIVMSG ')){fragments++;if(fragments===1){pingAt=Date.now();socket.write(wsFrame('PING :owned-pacing-check'));}}
   if(line==='PONG :owned-pacing-check'){pongAt=Date.now();resolvePong();}
  }}));
 });
 const port=await listen(fault);const local=structuredClone(config);local.facility.wss_url=`wss://localhost:${port}/fault`;local.facility.logical_id='owned-irc-local-fault';
 const route=await startBrowser(local);const target=route.slice(0,32)+'22'.repeat(16);
 await page.evaluate(({target})=>window.carrier.send(target,new Uint8Array(1200).fill(90)),{target});
 await bounded(pong,'PONG during slow fragment queue',1000);assert(pongAt-pingAt<500);assert.equal(fragments,1);
 const blockedReceive=page.evaluate(async()=>{try{await window.carrier.receive();return false;}catch{return true;}});
 await page.evaluate(()=>window.carrier.close());assert.equal(await bounded(blockedReceive,'cancel wakes RX'),true);
 report.faults.lifecycle={pingAt,pongAt,pongLatencyMs:pongAt-pingAt,fragmentsAtCancel:fragments,browser:await browserStatus()};
 assert.equal(report.faults.lifecycle.browser.assembly.pending,0);assert.equal(report.faults.lifecycle.browser.queue_pending,0);assert.equal(report.faults.lifecycle.browser.rx_pending,0);assert.equal(report.faults.lifecycle.browser.protocol.sent_packets,0);
 await bounded(new Promise(resolveClose=>{if(faultSocket.destroyed)return resolveClose();faultSocket.once('close',resolveClose);}), 'fault socket close',3000);
 const short=structuredClone(local);short.profile.attempt_ms=250;await startBrowser(short);
 assert.equal(await bounded(page.evaluate(async()=>{try{await window.carrier.receive();return false;}catch{return true;}}),'whole-attempt deadline',1000),true);
 await page.waitForFunction(()=>JSON.parse(window.carrier.report()).socket_state===3);report.faults.deadline=await browserStatus();assert.match(report.faults.deadline.protocol.failure,/whole attempt deadline/);assert.equal(report.faults.deadline.queue_pending,0);assert.equal(report.faults.deadline.assembly.pending,0);assert.equal(report.faults.deadline.qualification.ready,false);report.faults.deadlineActiveTimers=await page.evaluate(()=>window.ownedIntervals.size);assert.equal(report.faults.deadlineActiveTimers,0,'terminal failure must stop its browser interval before explicit close');assert.equal(report.faults.deadline.timer_active,false);
}
try{
 const http=wasmServer(root);
 report.origin=`http://localhost:${await listen(http)}`;
 report.launch=privateBrowserLaunch(fixture);
 browser=await chromium.launchPersistentContext(join(fixture,'profile'),report.launch);report.browserVersion=browser.browser().version();page=await browser.newPage();
 const cdp=await browser.newCDPSession(page);await cdp.send('Network.enable');const sockets=new Map();
 cdp.on('Network.webSocketCreated',({requestId,url})=>{const s={url};sockets.set(requestId,s);report.websockets.push(s);});
 cdp.on('Network.webSocketHandshakeResponseReceived',({requestId,response})=>{Object.assign(sockets.get(requestId),{status:response.status,headers:response.headers});});
 cdp.on('Network.webSocketClosed',({requestId})=>{sockets.get(requestId).closedMs=Date.now();});
 page.on('pageerror',error=>console.error(error.stack));await page.goto(report.origin);
 await page.evaluate(async()=>{const module=await import('/pkg/miopunch_browser_probe.js');await module.default();window.BrowserIrcDatagram=module.BrowserIrcDatagram;window.ircFaultProbe=module.ircFaultProbe;window.ownedIntervals=new Set();const start=window.setInterval.bind(window);const stop=window.clearInterval.bind(window);window.setInterval=(callback,delay,...args)=>{const id=start(callback,delay,...args);window.ownedIntervals.add(id);return id;};window.clearInterval=id=>{window.ownedIntervals.delete(id);stop(id);};});
 const config={facility:{logical_id:'owned-irc-datagram-fixture',native_url:`ircs://localhost:${nativePort}/`,wss_url:`wss://localhost:${wssPort}/webirc`,channel:'#mio-'+randomBytes(6).toString('hex')},profile:{pacing_ms:600,assembly_ttl_ms:8000,queue_ttl_ms:1500,attempt_ms:30000},disabled:[]};report.facility=config.facility;report.profile=config.profile;
 const native=nativeFixture(config,'normal');const starting=await native.wait('starting');assert.equal(starting.address_count,0);assert(starting.route_rejected);const nativeReady=await native.wait('ready');assert.equal(nativeReady.address_count,1);assert.equal(nativeReady.traffic.protocol.tls.verified,true);assert.equal(nativeReady.traffic.protocol.tls.hostname,'localhost');
 const browserRoute=await startBrowser(config);assert.equal(browserRoute.slice(0,32),nativeReady.route.slice(0,32));assert.notEqual(browserRoute.slice(32),nativeReady.route.slice(32));
 const payload=Buffer.from(Array.from({length:1200},(_,i)=>(i*31+7)%256));
 await page.evaluate(({route,bytes})=>window.carrier.send(route,Uint8Array.from(bytes)),{route:nativeReady.route,bytes:[...payload]});
 const nativeReceived=await native.wait('received');assert.equal(nativeReceived.bytes,payload.toString('hex'));assert.equal(nativeReceived.source,browserRoute);report.normal.push({direction:'browser-native',bytes:1200,sha256:createHash('sha256').update(payload).digest('hex'),native:nativeReceived.traffic,browser:await browserStatus()});
 const received=page.evaluate(()=>window.carrier.receive());const sent=await native.command({command:'send',route:browserRoute,bytes:payload.toString('hex')},'sent');assert(sent.accepted);const browserReceived=JSON.parse(await bounded(received,'native-browser datagram'));assert.equal(browserReceived.bytes,payload.toString('hex'));assert.equal(browserReceived.source,nativeReady.route);const nativeStatus=await native.command({command:'status'},'status');report.normal.push({direction:'native-browser',bytes:1200,sha256:createHash('sha256').update(payload).digest('hex'),native:nativeStatus.traffic,browser:await browserStatus()});
 const browserFinal=await browserStatus();const nativeFinal=nativeStatus.traffic;for(const events of [browserFinal.protocol.received_fragments,nativeFinal.protocol.fragments]){assert.equal(events.length,10);assert.deepEqual(events.map(e=>e.fragment),Array.from({length:10},(_,i)=>i.toString(16).padStart(2,'0')));const span=events.at(-1).at_ms-events[0].at_ms;assert(span>=5000&&span<8000);assert(events.every(e=>e.line_bytes<=512));}
 assert.equal(browserFinal.assembly.pending,0);assert.equal(nativeFinal.pending,0);assert.equal(browserFinal.protocol.sent_packets,1);assert.equal(nativeFinal.sent,1);assert.equal(browserFinal.protocol.received_packets,1);assert.equal(nativeFinal.received,1);
 await page.evaluate(()=>window.carrier.close());await page.waitForFunction(()=>JSON.parse(window.carrier.report()).socket_state===3);report.cleanup.normalBrowser=await browserStatus();await native.stop();report.cleanup.normalNative=report.native.findLast(e=>e.event==='closed').traffic;assert.equal(report.cleanup.normalNative.driver_active,0);assert.equal(report.cleanup.normalNative.protocol.part_sent,true);assert.equal(report.cleanup.normalNative.protocol.quit_sent,true);assert.equal(report.cleanup.normalNative.protocol.queue_pending,0);assert.equal(report.cleanup.normalBrowser.buffered_bytes,0);assert.equal(report.cleanup.normalBrowser.waiters,0);assert.equal(report.cleanup.normalBrowser.timer_active,false);assert.equal(await page.evaluate(()=>window.ownedIntervals.size),0);
 const before=report.websockets.length;for(const disabled of [config.facility.logical_id,config.facility.native_url,config.facility.wss_url]){assert(await page.evaluate(({config,disabled})=>{try{window.BrowserIrcDatagram.connect(JSON.stringify({...config,disabled:[disabled]}));return false;}catch{return true;}},{config,disabled}));}assert.equal(report.websockets.length,before);report.faults.disabledAliasesNoWss=true;
 await localFaults(config);assert(report.websockets.find(s=>s.url===config.facility.wss_url)?.status===101);report.passed=true;
}catch(error){report.failure=String(error.stack??error);console.error(report.failure);}finally{await cleanup();}
assert.equal(report.cleanup.activeNativeProcesses,0);assert.equal(report.cleanup.activeFaultSockets,0);assert.equal(report.cleanup.serversClosed,true);save();process.exitCode=report.passed?0:1;
