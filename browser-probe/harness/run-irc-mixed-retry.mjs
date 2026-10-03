import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createServer as httpsServer } from 'node:https';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { startMixedMqttProxy } from './owned-mixed-mqtt-proxy.mjs';
import { bounded, createOwnedIrcFixtureTools, wasmServer, privateBrowserLaunch } from './owned-irc-fixture.mjs';
const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const argument = name => process.argv[process.argv.indexOf(name) + 1];
const evidence = resolve(argument('--evidence'));
const fixture = resolve(argument('--fixture'));
const nativePort = Number(argument('--native').split(':').at(-1));
const wssPort = Number(argument('--wss').split(':').at(-1));
const report = { startedUtc:new Date().toISOString(),passed:false,normal:[],native:[],websockets:[],faults:{},cleanup:{},budgets:{outerSeconds:60,diagnosticMilliseconds:Number(argument('--caller-budget-ms')),phaseSeconds:15} };
const nativeProcesses = new Set();
const ownedSockets = new Set();
const servers = [];
let browser, page, stopping;
function save(){writeFileSync(join(evidence,'irc-mixed-retry.json'),JSON.stringify(report,null,2)+'\n');}
const tools=createOwnedIrcFixtureTools({root,fixture,report,nativeProcesses,servers});
const listen=tools.listen;
const nativeFixture=(config,name)=>tools.nativeFixture(config,name,true);
async function cleanup() {
 if (stopping) return stopping;
 stopping=(async()=>{
  await page?.evaluate(()=>window.retire?.cancel()).catch(()=>{});
  await browser?.close();
  await Promise.all([...nativeProcesses].map(async child=>{
   const exited=new Promise(resolveExit=>child.once('exit',resolveExit));
   child.stdin.end('stop\n');
   try { await bounded(exited,'native cleanup',3000); } catch { child.kill('SIGKILL'); await exited; }
  }));
  for(const socket of ownedSockets)socket.destroy();
  await Promise.all(servers.map(server=>new Promise(resolveClose=>server.close(resolveClose))));
  report.cleanup.activeNativeProcesses=nativeProcesses.size;
  report.cleanup.activeOwnedSockets=ownedSockets.size;
  report.cleanup.serversClosed=servers.every(server=>!server.listening);
  report.completedUtc=new Date().toISOString();save();
 })();return stopping;
}
process.on('SIGTERM',()=>{report.failure='external deadline/cancellation';cleanup().finally(()=>process.exit(1));});
try {
 const http=wasmServer(root);
 report.origin=`http://localhost:${await listen(http)}`;
 report.launch=privateBrowserLaunch(fixture);
 browser=await chromium.launchPersistentContext(join(fixture,'profile'),report.launch);report.browserVersion=browser.browser().version();page=await browser.newPage();
 const cdp=await browser.newCDPSession(page);await cdp.send('Network.enable');const sockets=new Map();
 cdp.on('Network.webSocketCreated',({requestId,url})=>{const s={url};sockets.set(requestId,s);report.websockets.push(s);});
 cdp.on('Network.webSocketHandshakeResponseReceived',({requestId,response})=>{Object.assign(sockets.get(requestId),{status:response.status,headers:response.headers});});
 cdp.on('Network.webSocketClosed',({requestId})=>{sockets.get(requestId).closedMs=Date.now();});

 cdp.on('Network.webSocketFrameSent',({requestId,response})=>{
  const entry=sockets.get(requestId); if(entry){entry.sentFrames=(entry.sentFrames??0)+1;entry.sentFrameBytes=(entry.sentFrameBytes??0)+Buffer.byteLength(response.payloadData);}
 });
 cdp.on('Network.webSocketFrameReceived',({requestId,response})=>{
  const entry=sockets.get(requestId); if(entry){entry.receivedFrames=(entry.receivedFrames??0)+1;entry.receivedFrameBytes=(entry.receivedFrameBytes??0)+Buffer.byteLength(response.payloadData);}
 });
 page.on('pageerror',error=>console.error(error.stack));await page.goto(report.origin);
 await page.evaluate(async()=>{
  const module=await import('/pkg/miopunch_browser_probe.js');await module.default();window.BrowserNetwork=module.BrowserNetwork;
  window.ownedIntervals=new Set();const start=window.setInterval.bind(window);const stop=window.clearInterval.bind(window);
  window.setInterval=(callback,delay,...args)=>{const id=start(callback,delay,...args);window.ownedIntervals.add(id);return id;};
  window.clearInterval=id=>{window.ownedIntervals.delete(id);stop(id);};
 });
 const mode=argument('--mode');let dropping=mode==='retry';report.mode=mode;
 const mqtt=await startMixedMqttProxy({fixture,broker:argument('--broker'),report,listen,ownedSockets,dropPublish:()=>dropping});
 const config={facility:{logical_id:'owned-irc-mixed-retry',native_url:`ircs://localhost:${nativePort}/`,wss_url:`wss://localhost:${wssPort}/webirc`,channel:'#mio-'+randomBytes(6).toString('hex')},profile:{pacing_ms:600,assembly_ttl_ms:8000,queue_ttl_ms:1500,attempt_ms:60000},mqtt,disabled:[],caller_budget_ms:12000};
 const native=nativeFixture(config,'mixed');const ready=await native.wait('ready');report.contactId=ready.id;report.facility=config.facility;report.mqttFacility=mqtt;
 const configuration={relays:[],mqtt:[mqtt],irc:[{facility:config.facility,profile:config.profile}],disabled:[],contact_routes:[{contact:ready.id,facility:config.facility.logical_id,address:ready.route},{contact:ready.id,facility:mqtt.logical_id,address:ready.mqtt_route}]};
 await bounded(page.evaluate(async configuration=>{window.network=await window.BrowserNetwork.start(JSON.stringify(configuration));window.retire=window.network.cancellation_handle();window.identity=window.network.id();},configuration),'mixed browser ready',12000);
 report.originalIdentity=await page.evaluate(()=>window.identity);
 async function request(label){
  const started=performance.now();
  const result=await bounded(page.evaluate(async invitation=>{try{return {result:await window.network.request_admission(invitation),identity:window.network.id(),status:JSON.parse(window.network.status()),diagnostics:JSON.parse(window.network.path_diagnostics())};}catch(error){return {failure:String(error),identity:window.network.id(),status:JSON.parse(window.network.status()),diagnostics:JSON.parse(window.network.path_diagnostics())};}},ready.invitation),label,13000);
  result.elapsedMs=performance.now()-started;return result;
 }
 if(mode==='retry'){
  report.original=await request('original restricted IRC attempt');assert(report.original.failure);assert.equal(report.original.result,undefined);
  report.beforeRetry=await native.command({command:'status'},'status');assert.equal(report.beforeRetry.requests.length,0);assert.deepEqual(report.beforeRetry.members,[ready.id]);
  assert(report.original.diagnostics.carriers.some(carrier=>carrier.closed===true&&carrier.protocol?.sent_fragments.length>=10));
  assert.equal(report.beforeRetry.mqtt_traffic.received_packets,0);assert(report.beforeRetry.traffic.received>0);
  dropping=false;
  report.retry=await request('same retained MQTT retry');save();
  assert.equal(report.retry.result,'AwaitingApproval','retired IRC must not consume a later MQTT caller budget');
  assert(report.retry.elapsedMs<12000);assert.equal(report.retry.identity,report.originalIdentity);
 }else{
  const started=performance.now();await page.waitForTimeout(13000);report.idleMs=performance.now()-started;
  report.beforeRetry=await native.command({command:'status'},'status');assert.equal(report.beforeRetry.requests.length,0);
  report.delayedFirst=await request('delayed first MQTT admission');save();
  assert.equal(report.delayedFirst.result,'AwaitingApproval','SDK object age must not consume the first public caller budget');
  assert.equal(report.delayedFirst.identity,report.originalIdentity);assert(report.delayedFirst.elapsedMs<12000);
 }
 report.afterRetry=await native.command({command:'status'},'status');
 const result=report.retry??report.delayedFirst;
 assert.equal(report.afterRetry.requests.length,1);assert.equal(report.afterRetry.requests[0].applicant,report.originalIdentity);assert.equal(report.afterRetry.requests[0].approved,false);assert.deepEqual(report.afterRetry.members,[ready.id]);
 assert.equal(result.status.secret_saved,false);assert.equal(result.status.authorization_saved,false);assert.equal(result.diagnostics.request.business_handle,false);assert.equal(result.diagnostics.request.remote_id,ready.id);
 const retiredIrc=result.diagnostics.carriers.find(carrier=>carrier.protocol?.sent_fragments);
 assert.equal(retiredIrc.closed,true);assert.equal(retiredIrc.qualification.ready,false);assert.equal(retiredIrc.queue_pending,0);assert.equal(retiredIrc.assembly.pending,0);
 assert(result.diagnostics.carriers.find(carrier=>carrier.protocol==='mqtt').traffic.received_packets>0);
 assert(report.afterRetry.mqtt_traffic.received_packets>0);
 await page.evaluate(()=>window.retire.cancel());
 report.cleanup.cancelled=await page.evaluate(()=>JSON.parse(window.network.path_diagnostics()));
 await native.stop();report.cleanup.native=report.native.findLast(entry=>entry.event==='closed');
 assert.equal(report.cleanup.native.traffic.driver_active,0);assert.equal(report.cleanup.native.traffic.pending,0);assert.equal(report.cleanup.native.mqtt_traffic.protocol.disconnect_sent,true);assert.equal(report.cleanup.native.mqtt_traffic.queued_packets,0);
 assert.equal(await page.evaluate(()=>window.ownedIntervals.size),0);report.cleanup.activeBrowserIntervals=0;
 assert(report.mqtt.proxy.some(proxy=>proxy.publishCount>0));
 report.checked=true;report.sameBrowserIdentity=true;report.onlyPendingManualRequest=true;report.ircAvailable=false;
 report.passed=true;
}catch(error){report.failure=String(error.stack??error);console.error(report.failure);}finally{await cleanup();}
assert.equal(report.cleanup.activeNativeProcesses,0);assert.equal(report.cleanup.serversClosed,true);save();process.exitCode=report.passed?0:1;
