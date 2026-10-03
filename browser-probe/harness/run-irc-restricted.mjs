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
const report = { startedUtc:new Date().toISOString(),passed:false,normal:[],native:[],websockets:[],faults:{},cleanup:{},budgets:{outerSeconds:60,diagnosticMilliseconds:Number(argument('--caller-budget-ms')),phaseSeconds:15} };
const nativeProcesses = new Set();
const ownedSockets = new Set();
const servers = [];
let browser, page, stopping;
function save(){writeFileSync(join(evidence,'irc-restricted-handshake.json'),JSON.stringify(report,null,2)+'\n');}
const tools=createOwnedIrcFixtureTools({root,fixture,report,nativeProcesses,servers});
const listen=tools.listen;
const nativeFixture=(config,name)=>tools.nativeFixture(config,name,true);
async function cleanup() {
 if (stopping) return stopping;
 stopping=(async()=>{
  await browser?.close();
  await Promise.all([...nativeProcesses].map(async child=>{
   const exited=new Promise(resolveExit=>child.once('exit',resolveExit));
   child.stdin.end('stop\n');
   try { await bounded(exited,'native cleanup',3000); } catch { child.kill('SIGKILL'); await exited; }
  }));
  await Promise.all(servers.map(server=>new Promise(resolveClose=>server.close(resolveClose))));
  report.cleanup.activeNativeProcesses=nativeProcesses.size;
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
 const callerBudgetMs=Number(argument('--caller-budget-ms'));
 assert(callerBudgetMs>0&&callerBudgetMs<=45000);
 const config={facility:{logical_id:'owned-irc-restricted-fixture',native_url:`ircs://localhost:${nativePort}/`,wss_url:`wss://localhost:${wssPort}/webirc`,channel:'#mio-'+randomBytes(6).toString('hex')},profile:{pacing_ms:600,assembly_ttl_ms:8000,queue_ttl_ms:1500,attempt_ms:60000},disabled:[],caller_budget_ms:callerBudgetMs};
 report.facility=config.facility;report.profile=config.profile;report.callerBudgetMs=callerBudgetMs;
 const native=nativeFixture(config,'restricted');const nativeReady=await native.wait('ready');
 report.contactId=nativeReady.id;
 const configuration={relays:[],mqtt:[],irc:[{facility:config.facility,profile:config.profile}],disabled:[],contact_routes:[{contact:nativeReady.id,facility:config.facility.logical_id,address:nativeReady.route}]};
 // Trusted native/WSS aliases must reject before any browser socket opens.
 const before=report.websockets.length;
 for(const disabled of [config.facility.logical_id,config.facility.native_url,config.facility.wss_url]) {
  const failure=await page.evaluate(async({configuration,disabled,invitation})=>{
   try { await window.BrowserNetwork.irc_restricted_probe(JSON.stringify({...configuration,disabled:[disabled]}),invitation,12000);return false; }catch{return true;}
  },{configuration,disabled,invitation:nativeReady.invitation});
  assert(failure);
 }
 assert.equal(report.websockets.length,before);report.faults.disabledAliasesNoWss=true;
 const started=performance.now();
 const result=await bounded(page.evaluate(async({configuration,invitation,callerBudgetMs})=>JSON.parse(await window.BrowserNetwork.irc_restricted_probe(JSON.stringify(configuration),invitation,callerBudgetMs)),{configuration,invitation:nativeReady.invitation,callerBudgetMs}),'actual restricted handshake caller',callerBudgetMs+3000);
 report.attempt={elapsedIncludingRetirementMs:performance.now()-started,...result};save();
 report.nativeStatus=await native.command({command:'status'},'status');
 await native.stop();report.cleanup.native=report.native.findLast(e=>e.event==='closed');
 await page.waitForFunction(()=>window.ownedIntervals.size===0,{},{timeout:1000});
 report.cleanup.activeBrowserIntervals=await page.evaluate(()=>window.ownedIntervals.size);
 await bounded((async()=>{while(report.websockets.some(s=>!s.closedMs))await new Promise(resolveTimer=>setTimeout(resolveTimer,10));})(),'actual WSS socket cleanup',1000);
 assert.equal(report.cleanup.native.traffic.driver_active,0);
 assert.equal(report.cleanup.native.traffic.protocol.queue_pending,0);
 assert.equal(report.cleanup.native.traffic.pending,0);
 assert.equal(result.cleanup.carriers[0].queue_pending,0);
 assert.equal(result.cleanup.carriers[0].assembly.pending,0);
 assert.equal(result.cleanup.carriers[0].rx_pending,0);
 assert.equal(result.status.authorization_saved,false);assert.equal(result.status.secret_saved,false);
 assert(report.websockets.find(s=>s.url===config.facility.wss_url)?.status===101);
 assert.equal(report.nativeStatus.traffic.protocol.tls.verified,true);
 const browserCarrier=result.diagnostics.carriers[0];
 const initialPackets=browserCarrier.protocol.accepted_datagrams.filter(item=>{const header=Buffer.from(item.first_five_bytes,'hex');return header.length===5&&(header[0]&0xf0)===0xc0&&header.readUInt32BE(1)===1;});
 report.quic={clientInitialDatagrams:initialPackets.length,clientInitialBytes:initialPackets.reduce((sum,item)=>sum+item.bytes,0),clientInitialAtMs:initialPackets.map(item=>item.at_ms),exactRetransmissionCounter:null,counterLimitation:'The hook records authenticated connection CRYPTO/loss stats when reached; no exact retransmission counter is exposed, so repeated Initial emissions are counted separately.'};
 report.handshakeAttemptMs=result.diagnostics.request.attempt_elapsed_ms;
 report.available=result.result==='AwaitingApproval';
 report.outcome=report.available?'restricted-control-qualified':'profile-unavailable';
 if(report.available){assert.equal(result.diagnostics.request.remote_id,nativeReady.id);assert.equal(result.diagnostics.request.business_handle,false);assert.equal(report.nativeStatus.requests.length,1);assert.equal(report.nativeStatus.requests[0].approved,false);}
 else {assert(result.failure);assert.equal(result.diagnostics.request.business_handle,false);assert.equal(report.nativeStatus.requests.length,0);}
 report.passed=true;
}catch(error){report.failure=String(error.stack??error);console.error(report.failure);}finally{await cleanup();}
assert.equal(report.cleanup.activeNativeProcesses,0);assert.equal(report.cleanup.serversClosed,true);save();process.exitCode=report.passed?0:1;
