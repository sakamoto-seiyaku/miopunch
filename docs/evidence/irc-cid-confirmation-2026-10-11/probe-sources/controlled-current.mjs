import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import crypto from 'node:crypto';
import {createServer as httpsServer} from 'node:https';
import {privateBrowserLaunch} from '/home/js/Git/miopunch/browser-probe/harness/owned-irc-fixture.mjs';
import {startMixedMqttProxy} from '/home/js/Git/miopunch/browser-probe/harness/owned-mixed-mqtt-proxy.mjs';
const require=createRequire('/home/js/Git/miopunch/browser-probe/harness/package.json');
const {chromium}=require('playwright-core');
const here=path.dirname(new URL(import.meta.url).pathname);
const pkg='/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/pkg';
const caseName=process.env.MIO87_REGRESSION;const mode=caseName==='mqtt'?'mqtt':'irc';assertMode();function assertMode(){if(!['irc','mqtt'].includes(mode))throw new Error('invalid owned mode');}
const trustFixture=path.join(here,'private-fixture');const servers=[],ownedSockets=new Set();
const output=path.join(here,'runtime',caseName);fs.mkdirSync(output,{mode:0o700,recursive:true});
const fixture=fs.mkdtempSync(path.join(here,'owned-fixture-'));fs.chmodSync(fixture,0o700);
const report={case:caseName,sdkCallerBudgetMs:120000,profile:{pacing_ms:10,assembly_ttl_ms:8000,queue_ttl_ms:30000,attempt_ms:120000},scope:'owned slow IRC comparison; same TLS facility/profile/CID2; server fake lag2msg/sec; explicit120s caller; historical CID-only versus current combined SDK',passed:false,nativeEvents:[],cleanup:{}};
function save(){fs.writeFileSync(path.join(output,'private-runtime-receipt.json'),JSON.stringify(report,null,2),{mode:0o600});}
function stage(name,extra={}){console.log(JSON.stringify({phase:name,...extra}));save();}
async function bounded(p,label,ms){let timer;try{return await Promise.race([p,new Promise((_,r)=>timer=setTimeout(()=>r(new Error(label+' local observer timeout')),ms))]);}finally{clearTimeout(timer);}}
let browser,native,page,exited,nativeExit,mqtt,observer;const observation={sentFrames:0,receivedFrames:0};
async function listen(server){servers.push(server);await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});return server.address().port;}const entries=[],waiters=[];let nativeStderr='';
function wait(event,ms=10000){const i=entries.findIndex(v=>v.event===event);if(i>=0)return Promise.resolve(entries.splice(i,1)[0]);let w;return bounded(new Promise((resolve,reject)=>{w={event,resolve,reject};waiters.push(w);}),event,ms).finally(()=>{const i=waiters.indexOf(w);if(i>=0)waiters.splice(i,1);});}
const facility=JSON.parse(fs.readFileSync(path.join(here,'local-facility.json')));facility.channel='#mio-current-'+crypto.randomBytes(6).toString('hex');
report.facility=facility;
try{
 const originServer=httpsServer({key:fs.readFileSync(path.join(trustFixture,'leaf.key')),cert:fs.readFileSync(path.join(trustFixture,'leaf.pem'))},(req,res)=>{res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Owned SDK contact regression</title>');});
 const origin='https://localhost:'+await listen(originServer);
 if(mode==='mqtt')mqtt=await startMixedMqttProxy({fixture:trustFixture,broker:JSON.parse(fs.readFileSync(path.join(here,'local-broker.json'))).address,report,listen,ownedSockets,dropPublish:()=>false});
 browser=await chromium.launchPersistentContext(path.join(trustFixture,'browser-profile-'+path.basename(here)),privateBrowserLaunch(trustFixture));
 page=await browser.newPage();const cdp=await browser.newCDPSession(page);await cdp.send('Network.enable');const sockets=new Map();report.websockets=[];
 cdp.on('Network.webSocketCreated',v=>{const s={url:v.url};sockets.set(v.requestId,s);report.websockets.push(s);});
 cdp.on('Network.webSocketWillSendHandshakeRequest',v=>{const s=sockets.get(v.requestId);if(s)s.origin=v.request.headers.Origin??v.request.headers.origin;});
 cdp.on('Network.webSocketHandshakeResponseReceived',v=>{const s=sockets.get(v.requestId);if(s){s.status=v.response.status;s.protocol=v.response.headers['Sec-WebSocket-Protocol'];}});
 cdp.on('Network.webSocketFrameSent',()=>observation.sentFrames++);cdp.on('Network.webSocketFrameReceived',()=>observation.receivedFrames++);
 cdp.on('Network.webSocketClosed',v=>{const s=sockets.get(v.requestId);if(s)s.closed=true;});
 const landing=await page.goto(origin,{timeout:15000,waitUntil:'domcontentloaded'});report.pageStatus=landing.status();report.actualPageOrigin=await page.evaluate(()=>location.origin);if(report.pageStatus!==200||report.actualPageOrigin!==origin)throw new Error('genuine public origin missing');
 stage('genuine-HTTPS-origin-ready');
 let js=fs.readFileSync(path.join(pkg,'miopunch_browser_probe.js'),'utf8');
 js=js.replace(/from '(\.\/snippets\/[^']+)'/g,(full,name)=>"from 'data:text/javascript;base64,"+fs.readFileSync(path.join(pkg,name)).toString('base64')+"'");
 await page.evaluate(async({js,wasm})=>{const url=URL.createObjectURL(new Blob([js],{type:'text/javascript'}));try{const m=await import(url);await m.default({module_or_path:Uint8Array.from(atob(wasm),c=>c.charCodeAt(0))});window.currentSdk=m;}finally{URL.revokeObjectURL(url);}}, {js,wasm:fs.readFileSync(path.join(pkg,'miopunch_browser_probe_bg.wasm')).toString('base64')});
 const cfg=path.join(fixture,'config.json');fs.writeFileSync(cfg,JSON.stringify({facility,profile:report.profile,caller_budget_ms:120000,maintained:true,relay:'https://localhost:1/',disabled:[],mqtt}),{mode:0o600});
 native=spawn('rtk',['proxy','/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/native/pinned/irc_current_native',cfg,path.join(trustFixture,'ca.pem'),path.join(fixture,'native-store')],{stdio:['pipe','pipe','pipe']});
 exited=new Promise(resolve=>native.once('exit',(code,signal)=>{nativeExit={code,signal};report.nativeExit=nativeExit;save();for(const w of waiters.splice(0))w.reject(new Error('owned native exited'));resolve(nativeExit);}));
 native.on('error',e=>{for(const w of waiters.splice(0))w.reject(e);});
 native.stderr.on('data',data=>{nativeStderr+=String(data);fs.writeFileSync(path.join(output,'private-native-stderr.log'),nativeStderr,{mode:0o600});});
 createInterface({input:native.stdout}).on('line',line=>{fs.appendFileSync(path.join(output,'private-native-stdout.jsonl'),line+'\n',{mode:0o600});try{const v=JSON.parse(line);report.nativeEvents.push(v);if(v.event.startsWith('[DEBUG-m87]'))console.log(JSON.stringify({phase:v.event,elapsedMs:v.elapsed_ms,callerRemainingMs:v.caller_remaining_ms,closed:v.closed}));const i=waiters.findIndex(w=>w.event===v.event);if(i>=0)waiters.splice(i,1)[0].resolve(v);else entries.push(v);save();}catch(e){report.nativeParseFailure=String(e);}});
 const ready=await wait('ready');stage('native-public-IRC-ready');
 const configuration=mode==='irc'?{relays:[],disabled:[],mqtt:[],irc:[{facility,profile:report.profile}],rtc_enabled:false,maintenance:{contact_timeout_ms:300000},contact_routes:[{contact:ready.id,facility:facility.logical_id,address:ready.route}]}:{relays:[],disabled:[facility.logical_id],mqtt:[mqtt],irc:[{facility,profile:report.profile}],rtc_enabled:false,maintenance:{contact_timeout_ms:300000},contact_routes:[{contact:ready.id,facility:mqtt.logical_id,address:ready.mqtt_route}]};
 if(caseName==='short'||caseName==='cancel')configuration.contact_routes[0].address=ready.route.slice(0,-1)+(ready.route.endsWith('0')?'1':'0');
 configuration.maintenance={contact_timeout_ms:caseName==='invalid'?Number.MAX_SAFE_INTEGER:report.sdkCallerBudgetMs};if(caseName==='disabled')configuration.disabled.push(facility.logical_id);
 const startup=await bounded(page.evaluate(async configuration=>{const t=performance.now();window.net=await window.currentSdk.BrowserNetwork.start(JSON.stringify(configuration));return {elapsedMs:performance.now()-t,status:JSON.parse(window.net.status()),diagnostics:JSON.parse(window.net.path_diagnostics())};},configuration),'browser formal startup',305000);report.startup=startup;stage('browser-formal-SDK-IRC-started',{elapsedMs:startup.elapsedMs});
 if(caseName==='aged')await new Promise(r=>setTimeout(r,3100));
 stage('actual-regression-request-started',{case:caseName,listenerIdleMs:caseName==='aged'?3100:0});const observationStarted=performance.now();observer=setInterval(()=>console.log(JSON.stringify({phase:'bounded-progress',elapsedMs:Math.round(performance.now()-observationStarted),...observation,nativePhase:report.nativeEvents.at(-1)?.event??null})),10000);
 report.request=await bounded(page.evaluate(async ({invitation,caseName})=>{const t=performance.now();let result=null,failure=null;const cancellation=caseName==='cancel'?window.net.cancellation_handle():null;let timer;if(cancellation)timer=setTimeout(()=>cancellation.cancel(),250);try{result=await window.net.request_admission(invitation);}catch(e){failure=String(e);}finally{clearTimeout(timer);cancellation?.free();}return {result,failure,elapsedMs:performance.now()-t,status:JSON.parse(window.net.status()),diagnostics:JSON.parse(window.net.path_diagnostics())};},{invitation:ready.invitation,caseName}),'formal regression request',125000);
 const gate=report.request.diagnostics.gate??{},coord=gate.coordinator??{};
 report.currentStrictPurpose2Qualified=report.request.result==='AwaitingApproval'&&gate.tls_authenticated===true&&gate.expected_contact_verified===true&&gate.remote_id===ready.id&&gate.business_handle===false&&coord.version===1&&coord.purposeCode===2&&coord.restricted===true&&coord.ordinaryHandle===false;
 if(native.exitCode===null){native.stdin.write('status\n');try{report.nativeStatus=await wait('status');}catch(e){report.nativeStatusFailure=String(e);}} report.nativeRequestAccepted=(report.nativeStatus?.requests??[]).length===1;
 report.passed=caseName==='short'?report.request.failure!==null&&report.request.elapsedMs>=1800&&report.request.elapsedMs<=2500:caseName==='cancel'?report.request.failure!==null&&report.request.elapsedMs>=200&&report.request.elapsedMs<=1500:report.currentStrictPurpose2Qualified&&report.nativeRequestAccepted;
 stage('restricted-request-returned',{result:report.request.result,failed:report.request.failure!==null,elapsedMs:report.request.elapsedMs,strictPurpose2Qualified:report.currentStrictPurpose2Qualified,nativeRequestAccepted:report.nativeRequestAccepted});
}catch(e){report.firstFailure={class:e.constructor.name,message:String(e)};if(caseName==='disabled')report.passed=String(e).includes('no allowed browser')&&report.websockets.length===0;if(caseName==='invalid')report.passed=String(e).includes('outside bounded limits')&&report.websockets.length===0;stage('first-failure',{class:e.constructor.name});}
finally{
 clearInterval(observer);report.progress=observation;
 if(page&&!page.isClosed()){
  try{report.cleanup.browser=await bounded(page.evaluate(async()=>{if(!window.net)return {sdkStarted:false};const t=performance.now();await window.net.close();window.net.free();window.net=null;return {sdkStarted:true,closeReturned:true,freed:true,elapsedMs:performance.now()-t};}),'SDK close',120000);}catch(e){report.cleanup.browser={closeReturned:false,error:String(e)};}
  await new Promise(r=>setTimeout(r,500));report.cleanup.socketImmediateObservationPassed=report.websockets?.every(s=>s.closed===true)??false;
 }
 if(native&&native.exitCode===null&&native.signalCode===null){native.stdin.end('stop\n');try{await bounded(exited,'owned native natural close',130000);}catch(e){report.cleanup.nativeForced=true;native.kill('SIGTERM');await bounded(exited,'owned native forced close',2000).catch(()=>{});}}
 report.cleanup.nativeExit=nativeExit??null;report.cleanup.nativeClosedReceipt=report.nativeEvents.findLast(v=>v.event==='closed')??null;
 const socketWaitStarted=performance.now();while(!(report.websockets?.every(s=>s.closed===true)??false)&&performance.now()-socketWaitStarted<60000)await new Promise(r=>setTimeout(r,100));report.cleanup.lateSocketWaitMs=performance.now()-socketWaitStarted;report.cleanup.websocketsClosedBeforeContext=report.websockets?.every(s=>s.closed===true)??false;stage('late-socket-observation',{immediatePassed:report.cleanup.socketImmediateObservationPassed,latePassed:report.cleanup.websocketsClosedBeforeContext,waitMs:report.cleanup.lateSocketWaitMs});
 if(browser){await browser.close();report.cleanup.ownedBrowserContextClosed=true;}
 for(const socket of ownedSockets)socket.destroy();await Promise.all(servers.map(server=>new Promise(r=>server.close(r))));report.cleanup.ownedServersClosed=servers.every(s=>!s.listening);
 const absent=native?(()=>{try{process.kill(native.pid,0);return false;}catch(e){return e.code==='ESRCH';}})():true;report.cleanup.nativeProcessAbsentImmediate=absent;await new Promise(r=>setTimeout(r,500));report.cleanup.nativeProcessAbsentLate=native?(()=>{try{process.kill(native.pid,0);return false;}catch(e){return e.code==='ESRCH';}})():true;
 if(report.cleanup.nativeProcessAbsentLate){fs.rmSync(fixture,{recursive:true,force:true});report.cleanup.ownedFixtureRemoved=!fs.existsSync(fixture);}
 save();stage('case-cleaned',{functionalPassed:report.passed,nativeExit:nativeExit?.code,forced:report.cleanup.nativeForced===true});
}
report.cleanupQualified=report.cleanup.websocketsClosedBeforeContext===true&&report.cleanup.nativeExit?.code===0&&report.cleanup.nativeClosedReceipt?.graceful_close===true&&report.cleanup.nativeClosedReceipt?.traffic?.driver_active===0&&report.cleanup.nativeForced!==true&&report.cleanup.ownedBrowserContextClosed===true&&report.cleanup.ownedServersClosed===true&&report.cleanup.nativeProcessAbsentImmediate===true&&report.cleanup.nativeProcessAbsentLate===true&&report.cleanup.ownedFixtureRemoved===true&&(report.cleanup.browser?.sdkStarted===false||(report.cleanup.browser?.closeReturned===true&&report.cleanup.browser?.freed===true));report.qualified=report.passed&&report.cleanupQualified;save();process.exitCode=report.qualified?0:1;
