// Real public IRC qualification through the existing startup-inclusive SDK seam.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { bounded } from './owned-irc-fixture.mjs';

const root = resolve(fileURLToPath(new URL('../../', import.meta.url)));
const arg = name => process.argv[process.argv.indexOf(name) + 1];
const output = resolve(arg('--output'));
const fixture = resolve(arg('--fixture'));
const facility = JSON.parse(readFileSync(arg('--facility'), 'utf8'));
facility.channel = '#mio-qualification-' + randomBytes(6).toString('hex');
const profile = {pacing_ms:600, assembly_ttl_ms:8000, queue_ttl_ms:1500, attempt_ms:60000};
const report = {startedUtc:new Date().toISOString(), passed:false, available:false,
  scope:'Existing formal SDK only; does not qualify the future unified protocol or a business data path',
  facility, profile, callerBudgetMs:12000, stageBudgetMs:5000, native:[], websockets:[], attempts:[], cleanup:{},
  roles:{browser:'QUIC dialer / admission applicant',native:'QUIC acceptor / admission contact'},
  policy:{source:'https://testnet.ergo.chat/', normalCertificateValidation:true, originHeaderOverride:false, pageResponseReplacement:false, concurrentPublicClients:2}};
const hash = data => createHash('sha256').update(data).digest('hex');
// Public evidence retains sizes/timing/hashes, never invitation or endpoint identity material.
function redact(value, key='') {
  if (['invitation','id','local_id','remote_id','contact','contactId','applicant','request_id','route','mqtt_route','address','stable_id'].includes(key)) return undefined;
  if (key==='fragment') return {sha256:hash(value)};
  if (key==='paths') return value.map(path=>({kind:path.startsWith('Custom')?'Custom':'Other', sha256:hash(path)}));
  if (Array.isArray(value)) return value.map(v=>redact(v));
  if (value && typeof value==='object') return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,redact(v,k)]).filter(([,v])=>v!==undefined));
  return value;
}
function save() {writeFileSync(join(output,'qualification.json'), JSON.stringify(redact(report),null,2)+'\n');}
let browser, native, processExit, cleanupPromise;
const entries=[], pending=[];
function wait(event) {
  const index=entries.findIndex(e=>e.event===event);
  if(index>=0)return Promise.resolve(entries.splice(index,1)[0]);
  return bounded(new Promise((resolveValue,reject)=>pending.push({event,resolve:resolveValue,reject})),`native ${event}`,12000);
}
async function cleanup() {
  if(cleanupPromise)return cleanupPromise;
  cleanupPromise=(async()=>{const start=performance.now();
  await browser?.close();
  if(native && native.exitCode===null && native.signalCode===null) {
    native.stdin.end('stop\n');
    try {report.cleanup.nativeExit=await bounded(processExit,'native cleanup',4000);}
    catch(error) {report.cleanup.forcedKill=true;native.kill('SIGKILL');report.cleanup.nativeExit=await processExit;report.cleanup.error=String(error);}
  }
  report.cleanup.nativeReceipt=report.native.findLast(e=>e.event==='closed');
  report.cleanup.immediate={nativeExited:!native || native.exitCode!==null || native.signalCode!==null, allWebSocketsClosed:report.websockets.every(s=>s.closedAtMs!==undefined)};
  await new Promise(resolveTimer=>setTimeout(resolveTimer,500));
  report.cleanup.late={nativeExited:!native || native.exitCode!==null || native.signalCode!==null, allWebSocketsClosed:report.websockets.every(s=>s.closedAtMs!==undefined)};
  report.cleanup.elapsedMs=performance.now()-start;report.completedUtc=new Date().toISOString();save();
  })();return cleanupPromise;
}
process.on('SIGTERM',()=>{report.failure='outer cancellation';cleanup().finally(()=>process.exit(1));});
try {
  let executable=process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE??chromium.executablePath();
  if(!existsSync(executable))for(const name of readdirSync(join(homedir(),'.cache/ms-playwright')).filter(n=>n.startsWith('chromium-')).sort().reverse()) {
    const candidate=join(homedir(),'.cache/ms-playwright',name,'chrome-linux64/chrome');if(existsSync(candidate)){executable=candidate;break;}
  }
  browser=await chromium.launchPersistentContext(join(fixture,'browser-profile'),{executablePath:executable,headless:true,args:['--no-sandbox'],ignoreHTTPSErrors:false});
  const page=await browser.newPage();const cdp=await browser.newCDPSession(page);await cdp.send('Network.enable');const sockets=new Map();
  cdp.on('Network.webSocketCreated',({requestId,url})=>{const s={url,createdAtMs:performance.now()};sockets.set(requestId,s);report.websockets.push(s);});
  cdp.on('Network.webSocketWillSendHandshakeRequest',({requestId,request})=>{const s=sockets.get(requestId);if(s)s.origin=request.headers.Origin??request.headers.origin;});
  cdp.on('Network.webSocketHandshakeResponseReceived',({requestId,response})=>{const s=sockets.get(requestId);if(s){s.status=response.status;s.protocol=response.headers['Sec-WebSocket-Protocol'];}});
  cdp.on('Network.webSocketClosed',({requestId})=>{const s=sockets.get(requestId);if(s)s.closedAtMs=performance.now();});
  for(const [event,direction] of [['Network.webSocketFrameSent','sent'],['Network.webSocketFrameReceived','received']])cdp.on(event,({requestId,response})=>{const s=sockets.get(requestId);if(s){s[direction+'Frames']=(s[direction+'Frames']??0)+1;s[direction+'FrameBytes']=(s[direction+'FrameBytes']??0)+Buffer.byteLength(response.payloadData);}});
  const landing=await page.goto('https://testnet.ergo.chat/',{timeout:15000,waitUntil:'domcontentloaded'});
  report.policy.pageStatus=landing.status();report.policy.actualPageOrigin=await page.evaluate(()=>location.origin);report.browserVersion=browser.browser().version();
  assert.equal(report.policy.actualPageOrigin,'https://testnet.ergo.chat');
  const storagePath=(readFileSync(join(root,'browser-probe/pkg/miopunch_browser_probe.js'),'utf8').match(/from '(\.\/snippets\/[^']+)'/))[1];
  const storage=readFileSync(join(root,'browser-probe/pkg',storagePath),'utf8');
  const js=readFileSync(join(root,'browser-probe/pkg/miopunch_browser_probe.js'),'utf8').replace(storagePath,'data:text/javascript;base64,'+Buffer.from(storage).toString('base64'));
  await page.evaluate(async({js,wasm})=>{
    const url=URL.createObjectURL(new Blob([js],{type:'text/javascript'}));
    const module=await import(url);await module.default({module_or_path:Uint8Array.from(atob(wasm),c=>c.charCodeAt(0))});URL.revokeObjectURL(url);window.BrowserNetwork=module.BrowserNetwork;
    window.ownedIntervals=new Set();const start=window.setInterval.bind(window),stop=window.clearInterval.bind(window);
    window.setInterval=(callback,delay,...args)=>{const id=start(callback,delay,...args);window.ownedIntervals.add(id);return id;};window.clearInterval=id=>{window.ownedIntervals.delete(id);stop(id);};
  },{js,wasm:readFileSync(join(root,'browser-probe/pkg/miopunch_browser_probe_bg.wasm')).toString('base64')});
  const configPath=join(fixture,'config.json');writeFileSync(configPath,JSON.stringify({facility,profile,caller_budget_ms:12000,disabled:[]}));
  native=spawn('rtk',['proxy',join(root,'target/debug/browser_irc_restricted_native'),configPath,'-',join(fixture,'native-store')],{stdio:['pipe','pipe','pipe']});
  processExit=new Promise(resolveExit=>native.once('exit',(code,signal)=>{for(const p of pending.splice(0))p.reject(new Error(`native exit ${code}/${signal}`));resolveExit({code,signal});}));
  native.on('error',error=>{for(const p of pending.splice(0))p.reject(error);});
  native.stderr.on('data',data=>{report.nativeStderr=(report.nativeStderr??'')+String(data);});
  createInterface({input:native.stdout}).on('line',line=>{try{const value=JSON.parse(line);report.native.push(value);const index=pending.findIndex(p=>p.event===value.event);if(index>=0)pending.splice(index,1)[0].resolve(value);else entries.push(value);save();}catch(error){report.parseFailure=String(error);}});
  const ready=await wait('ready');report.nativeReady=true;
  const configuration={relays:[],mqtt:[],irc:[{facility,profile}],disabled:[],contact_routes:[{contact:ready.id,facility:facility.logical_id,address:ready.route}]};
  for(const scenario of ['expected-contact','wrong-contact']) {
    // The second call deliberately has no matching route. It must fail closed before QUIC or WSS sends.
    const current=scenario==='wrong-contact'?{...configuration,contact_routes:[{...configuration.contact_routes[0],contact:'00'.repeat(32)}]}:configuration;
    const start=performance.now();const beforeSockets=report.websockets.length;
    const result=await bounded(page.evaluate(async({configuration,invitation})=>JSON.parse(await window.BrowserNetwork.irc_restricted_probe(JSON.stringify(configuration),invitation,12000)),{configuration:current,invitation:ready.invitation}),'public SDK caller and retirement',15000);
    const gate=result.diagnostics.gate??{},request=result.diagnostics.request??{};
    const carrier=result.diagnostics.carriers?.[0];
    const initials=(carrier?.protocol.accepted_datagrams??[]).filter(item=>{const header=Buffer.from(item.first_five_bytes,'hex');return header.length===5 && (header[0]&0xf0)===0xc0 && header.readUInt32BE(1)===1;});
    const attempt={scenario,elapsedIncludingRetirementMs:performance.now()-start,...result,identity:{expectedContactVerified:gate.expected_contact_verified??null,remoteMatchesContact:request.remote_id?request.remote_id===ready.id:null,nativePeerMatchesBrowser:null},normalDataHandleReturned:request.business_handle===true,
      quic:{initialDatagrams:initials.length,initialBytes:initials.reduce((sum,item)=>sum+item.bytes,0),initialAtMs:initials.map(item=>item.at_ms),allInitialsAtLeast1200:initials.every(item=>item.bytes>=1200),exactRetransmissionCounter:null}};
    report.attempts.push(attempt);assert.equal(attempt.normalDataHandleReturned,false);
    if(scenario==='expected-contact')report.available=result.result==='AwaitingApproval';
    if(scenario==='wrong-contact'){assert(result.failure);assert.equal(result.result,null);assert.equal(carrier.protocol.accepted_bytes,0);attempt.noQuicDatagramSent=true;}
    save();
    // Failed IRC branch is stopped; the independent MQTT route is not configured or disabled here.
    if(scenario==='expected-contact'){
      report.stopBranch=report.available?'restricted-control-qualified':'IRC unavailable; do not advance IRC SDK branch; MQTT remains independent';
      if(result.failure)report.firstFailure={scenario,failure:result.failure,gateStage:gate.stage??null,requestStage:request.stage??null};
    }
  }
  const status=wait('status');native.stdin.write('status\n');report.nativeStatus=await status;
  const original=report.attempts[0];const nativeAdmission=report.native.find(e=>e.event==='admission');
  if(nativeAdmission)original.identity.nativePeerMatchesBrowser=nativeAdmission.remote_id===original.diagnostics.request?.local_id;
  if(report.available){assert.equal(report.nativeStatus.requests.length,1);assert.equal(original.identity.remoteMatchesContact,true);assert.equal(original.identity.expectedContactVerified,true);assert.equal(original.identity.nativePeerMatchesBrowser,true);}
  else assert.equal(report.nativeStatus.requests.length,0);
  report.cleanup.activeBrowserIntervals=await page.evaluate(()=>window.ownedIntervals.size);assert.equal(report.cleanup.activeBrowserIntervals,0);
  // Socket close delivery is asynchronous even after the SDK reports readyState=CLOSED.
  const socketObservationStarted=performance.now();
  await bounded((async()=>{while(report.websockets.some(s=>s.closedAtMs===undefined))await new Promise(resolveTimer=>setTimeout(resolveTimer,10));})(),'WSS close observation',1500);
  report.cleanup.socketCloseObservationMs=performance.now()-socketObservationStarted;
  report.outcome=report.available?'restricted-control-qualified':'profile-unavailable';report.passed=true;
} catch(error) {report.failure=String(error.stack??error);report.firstFailure=report.failure;}
finally {await cleanup();}
if(report.cleanup.forcedKill || !report.cleanup.late.nativeExited || !report.cleanup.late.allWebSocketsClosed)report.passed=false;
save();console.log(JSON.stringify({passed:report.passed,available:report.available,outcome:report.outcome,failure:report.failure}));process.exitCode=report.passed?0:1;
