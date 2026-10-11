import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import crypto from 'node:crypto';
const require=createRequire('/home/js/Git/miopunch/browser-probe/harness/package.json');
const {chromium}=require('playwright-core');
const here=path.dirname(new URL(import.meta.url).pathname);
const pkg='/tmp/mio87-final-no-debug-sdk-v2-root-20261011/wasm/pkg';
const output=path.join(here,'runtime');fs.mkdirSync(output,{mode:0o700});
const fixture=fs.mkdtempSync(path.join(here,'owned-fixture-'));fs.chmodSync(fixture,0o700);
const relay='https://aps1-1.relay.n0.iroh.link./';
const report={sdkCallerBudgetMs:300000,profile:{pacing_ms:600,assembly_ttl_ms:8000,queue_ttl_ms:30000,attempt_ms:300000},scope:'current formal public Ergo explicit300s purpose2; maintained listeners,600ms pacing,queue30s,defaultQUIC',passed:false,nativeEvents:[],cleanup:{}};
function save(){fs.writeFileSync(path.join(output,'private-runtime-receipt.json'),JSON.stringify(report,null,2),{mode:0o600});}
function stage(name,extra={}){console.log(JSON.stringify({phase:name,...extra}));save();}
async function bounded(p,label,ms){let timer;try{return await Promise.race([p,new Promise((_,r)=>timer=setTimeout(()=>r(new Error(label+' local observer timeout')),ms))]);}finally{clearTimeout(timer);}}
let browser,browserProcess,native,page,exited,nativeExit,observer;const observation={sentFrames:0,receivedFrames:0};const entries=[],waiters=[];let nativeStderr='';
function wait(event,ms=10000){const i=entries.findIndex(v=>v.event===event);if(i>=0)return Promise.resolve(entries.splice(i,1)[0]);let w;return bounded(new Promise((resolve,reject)=>{w={event,resolve,reject};waiters.push(w);}),event,ms).finally(()=>{const i=waiters.indexOf(w);if(i>=0)waiters.splice(i,1);});}
const facility=JSON.parse(fs.readFileSync('/home/js/Git/miopunch/docs/evidence/public-irc-restricted-2026-10-05/facility.json'));facility.channel='#mio-current-'+crypto.randomBytes(6).toString('hex');
report.facility=facility;
try{
 browserProcess=await chromium.launch({executablePath:'/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome',headless:true,args:['--no-sandbox']});browser=await browserProcess.newContext({ignoreHTTPSErrors:false});
 page=await browser.newPage();const cdp=await browser.newCDPSession(page);await cdp.send('Network.enable');const sockets=new Map();report.websockets=[];
 cdp.on('Network.webSocketCreated',v=>{const s={url:v.url};sockets.set(v.requestId,s);report.websockets.push(s);});
 cdp.on('Network.webSocketWillSendHandshakeRequest',v=>{const s=sockets.get(v.requestId);if(s)s.origin=v.request.headers.Origin??v.request.headers.origin;});
 cdp.on('Network.webSocketHandshakeResponseReceived',v=>{const s=sockets.get(v.requestId);if(s){s.status=v.response.status;s.protocol=v.response.headers['Sec-WebSocket-Protocol'];}});
 cdp.on('Network.webSocketFrameSent',()=>observation.sentFrames++);cdp.on('Network.webSocketFrameReceived',()=>observation.receivedFrames++);
 cdp.on('Network.webSocketClosed',v=>{const s=sockets.get(v.requestId);if(s)s.closed=true;});
 const landing=await page.goto('https://testnet.ergo.chat/',{timeout:60000,waitUntil:'domcontentloaded'});report.pageStatus=landing.status();report.actualPageOrigin=await page.evaluate(()=>location.origin);if(report.pageStatus!==200||report.actualPageOrigin!=='https://testnet.ergo.chat')throw new Error('genuine public origin missing');
 stage('genuine-HTTPS-origin-ready');
 let js=fs.readFileSync(path.join(pkg,'miopunch_browser_probe.js'),'utf8');
 js=js.replace(/from '(\.\/snippets\/[^']+)'/g,(full,name)=>"from 'data:text/javascript;base64,"+fs.readFileSync(path.join(pkg,name)).toString('base64')+"'");
 await page.evaluate(async({js,wasm})=>{const url=URL.createObjectURL(new Blob([js],{type:'text/javascript'}));try{const m=await import(url);await m.default({module_or_path:Uint8Array.from(atob(wasm),c=>c.charCodeAt(0))});window.currentSdk=m;}finally{URL.revokeObjectURL(url);}}, {js,wasm:fs.readFileSync(path.join(pkg,'miopunch_browser_probe_bg.wasm')).toString('base64')});
 const cfg=path.join(fixture,'config.json');fs.writeFileSync(cfg,JSON.stringify({facility,profile:report.profile,caller_budget_ms:300000,maintained:true,relay,disabled:[]}),{mode:0o600});
 function launchNative(cfg){nativeExit=null;
 native=spawn('rtk',['proxy','/tmp/mio87-proper-sdk-close-order-build-root-20261011/native/pinned/irc_current_native',cfg,'-',path.join(fixture,'native-store')],{stdio:['pipe','pipe','pipe']});
 exited=new Promise(resolve=>native.once('exit',(code,signal)=>{nativeExit={code,signal};report.nativeExit=nativeExit;save();for(const w of waiters.splice(0))w.reject(new Error('owned native exited'));resolve(nativeExit);}));
 native.on('error',e=>{for(const w of waiters.splice(0))w.reject(e);});
 native.stderr.on('data',data=>{nativeStderr+=String(data);fs.writeFileSync(path.join(output,'private-native-stderr.log'),nativeStderr,{mode:0o600});});
 createInterface({input:native.stdout}).on('line',line=>{fs.appendFileSync(path.join(output,'private-native-stdout.jsonl'),line+'\n',{mode:0o600});try{const v=JSON.parse(line);report.nativeEvents.push(v);if(v.event.startsWith('[DEBUG-m87]'))console.log(JSON.stringify({phase:v.event,elapsedMs:v.elapsed_ms,callerRemainingMs:v.caller_remaining_ms,closed:v.closed}));const i=waiters.findIndex(w=>w.event===v.event);if(i>=0)waiters.splice(i,1)[0].resolve(v);else entries.push(v);save();}catch(e){report.nativeParseFailure=String(e);}});
 }
 launchNative(cfg);
 const ready=await wait('ready');stage('native-public-IRC-ready');
 const configuration={relays:[],disabled:[],mqtt:[],irc:[{facility,profile:report.profile}],rtc_enabled:false,maintenance:{contact_timeout_ms:300000},contact_routes:[{contact:ready.id,facility:facility.logical_id,address:ready.route}]};
 const startup=await bounded(page.evaluate(async configuration=>{const t=performance.now();window.net=await window.currentSdk.BrowserNetwork.start(JSON.stringify(configuration));return {elapsedMs:performance.now()-t,status:JSON.parse(window.net.status()),diagnostics:JSON.parse(window.net.path_diagnostics())};},configuration),'browser formal startup',305000);report.startup=startup;stage('browser-formal-SDK-IRC-started',{elapsedMs:startup.elapsedMs});
 stage('actual-300s-restricted-request-started');const observationStarted=performance.now();observer=setInterval(()=>console.log(JSON.stringify({phase:'bounded-progress',elapsedMs:Math.round(performance.now()-observationStarted),...observation,nativePhase:report.nativeEvents.at(-1)?.event??null})),10000);
 report.request=await bounded(page.evaluate(async invitation=>{const t=performance.now();let result=null,failure=null;try{result=await window.net.request_admission(invitation);}catch(e){failure=String(e);}return {result,failure,elapsedMs:performance.now()-t,status:JSON.parse(window.net.status()),diagnostics:JSON.parse(window.net.path_diagnostics())};},ready.invitation),'formal 300s request',305000);
 const gate=report.request.diagnostics.gate??{},coord=gate.coordinator??{};
 report.currentStrictPurpose2Qualified=report.request.result==='AwaitingApproval'&&gate.tls_authenticated===true&&gate.expected_contact_verified===true&&gate.remote_id===ready.id&&gate.business_handle===false&&coord.version===1&&coord.purposeCode===2&&coord.restricted===true&&coord.ordinaryHandle===false;
 if(native.exitCode===null){native.stdin.write('status\n');try{report.nativeStatus=await wait('status');}catch(e){report.nativeStatusFailure=String(e);}} report.nativeRequestAccepted=(report.nativeStatus?.requests??[]).length===1;
 report.passed=report.currentStrictPurpose2Qualified&&report.nativeRequestAccepted;
 stage('restricted-request-returned',{result:report.request.result,failed:report.request.failure!==null,elapsedMs:report.request.elapsedMs,strictPurpose2Qualified:report.currentStrictPurpose2Qualified,nativeRequestAccepted:report.nativeRequestAccepted});
 if(!report.passed)throw new Error('initial IRC restricted request did not qualify');
 clearInterval(observer);observer=null;
 const originalId=await page.evaluate(()=>window.net.id());
 native.stdin.write('approve\n');report.approval=await wait('approved');
 if(report.approval.new_records!==1)throw new Error('exactly one actual approval record required');
 async function closeBrowserStage(){return await bounded(page.evaluate(async()=>{const t=performance.now();await window.net.close();window.net.free();window.net=null;return {closeReturned:true,freed:true,elapsedMs:performance.now()-t};}),'intermediate browser close',120000);}
 report.afterIrcBrowserClose=await closeBrowserStage();
 const dataConfiguration={relays:[relay],disabled:[],mqtt:[],irc:[],rtc_enabled:false,maintenance:{contact_timeout_ms:300000},contact_routes:[]};
 const restart=async()=>bounded(page.evaluate(async configuration=>{window.net=await window.currentSdk.BrowserNetwork.start(JSON.stringify(configuration));return {id:window.net.id(),status:JSON.parse(window.net.status())};},dataConfiguration),'saved browser reopen',305000);
 report.pendingReopen=await restart();if(report.pendingReopen.id!==originalId)throw new Error('pending durable identity changed');
 report.approvedRequest=await bounded(page.evaluate(async invitation=>{const t=performance.now();const result=await window.net.request_admission(invitation);return {result,elapsedMs:performance.now()-t,status:JSON.parse(window.net.status()),diagnostics:JSON.parse(window.net.path_diagnostics())};},ready.invitation),'approved relay request',305000);
 if(report.approvedRequest.result!=='PendingMaterials')throw new Error('approved request did not save authorization');
 report.materialCompletion=await bounded(page.evaluate(async()=>{const result=await window.net.complete_admission();return {result,status:JSON.parse(window.net.status()),receipt:JSON.parse(await window.net.admission_material_receipt()),diagnostics:JSON.parse(window.net.path_diagnostics())};}),'synthetic durable admission material completion',305000);
 if(report.materialCompletion.result!=='Complete'||!report.materialCompletion.status.authorization_saved||!report.materialCompletion.status.secret_saved)throw new Error('durable material completion missing');
 stage('actual-approved-materials-complete',{pendingIdentityPreserved:true,authorizationSaved:true,secretSaved:true});
 report.beforeBusinessBrowserClose=await closeBrowserStage();
 native.stdin.end('stop\n');await bounded(exited,'first native natural close',130000);
 report.firstNativeClose={exit:nativeExit,closed:report.nativeEvents.findLast(v=>v.event==='closed')};
 if(nativeExit.code!==0||report.firstNativeClose.closed?.graceful_close!==true)throw new Error('first Native phase did not close naturally');
 fs.writeFileSync(cfg,JSON.stringify({facility,profile:report.profile,caller_budget_ms:300000,maintained:true,relay,reopen:true,disabled:[]}),{mode:0o600});
 launchNative(cfg);const reopenedNative=await wait('ready');
 if(reopenedNative.id!==ready.id)throw new Error('native durable identity changed');
 report.completeReopen=await restart();
 if(report.completeReopen.id!==originalId||report.completeReopen.status.stage!=='Complete')throw new Error('complete durable browser reopen failed');
 report.memberOpen=await bounded(page.evaluate(async target=>{const t=performance.now();window.member=await window.net.open_member_connection(target);return {elapsedMs:performance.now()-t,diagnostics:JSON.parse(window.member.rtc_path_diagnostics())};},ready.id),'formal approved member open',305000);
 report.nativeMemberReleased=await wait('member-released');
 report.business=await bounded(page.evaluate(async()=>{const result=await window.member.exchange('PING');const diagnostics=JSON.parse(window.member.rtc_path_diagnostics());window.member.close();window.member.free();window.member=null;return {result,diagnostics};}),'actual ordinary relay PING PONG',65000);
 report.nativeBusiness=await wait('business');
 report.businessQualified=report.business.result==='PONG'&&report.nativeMemberReleased.authorized===true&&report.nativeMemberReleased.selected_relay===true&&report.nativeMemberReleased.alpn==='miopunch/session/member/1'&&report.nativeBusiness.member_message===true&&report.nativeBusiness.reply==='PONG'&&report.nativeBusiness.selected_relay===true&&report.nativeBusiness.stable_id===report.nativeMemberReleased.stable_id;
 report.passed=report.passed&&report.businessQualified;
 stage('formal-member-relay-business-returned',{passed:report.businessQualified,reply:report.business.result,browserAndNativeIdentitiesReopened:true});

}catch(e){report.firstFailure={class:e.constructor.name,message:String(e)};stage('first-failure',{class:e.constructor.name});}
finally{
 clearInterval(observer);report.progress=observation;
 if(page&&!page.isClosed()){
  try{report.cleanup.browser=await bounded(page.evaluate(async()=>{if(!window.net)return {sdkStarted:false};const t=performance.now();await window.net.close();window.net.free();window.net=null;return {sdkStarted:true,closeReturned:true,freed:true,elapsedMs:performance.now()-t};}),'SDK close',120000);}catch(e){report.cleanup.browser={closeReturned:false,error:String(e)};}
  await new Promise(r=>setTimeout(r,500));report.cleanup.socketImmediateObservationPassed=report.websockets?.every(s=>s.closed===true)??false;
  const lateSocketStarted=performance.now();while(!report.websockets.every(s=>s.closed===true)&&performance.now()-lateSocketStarted<60000)await new Promise(r=>setTimeout(r,100));
  report.cleanup.lateSocketWaitMs=performance.now()-lateSocketStarted;report.cleanup.websocketsClosedBeforeContext=report.websockets.every(s=>s.closed===true);
  stage('late-socket-observation',{immediatePassed:report.cleanup.socketImmediateObservationPassed,latePassed:report.cleanup.websocketsClosedBeforeContext,waitMs:report.cleanup.lateSocketWaitMs});
 }
 if(native&&native.exitCode===null&&native.signalCode===null){native.stdin.end('stop\n');try{await bounded(exited,'owned native natural close',130000);}catch(e){report.cleanup.nativeForced=true;native.kill('SIGTERM');await bounded(exited,'owned native forced close',2000).catch(()=>{});}}
 report.cleanup.nativeExit=nativeExit??null;report.cleanup.nativeClosedReceipt=report.nativeEvents.findLast(v=>v.event==='closed')??null;
 if(browser){await browser.close();report.cleanup.ownedBrowserContextClosed=true;await browserProcess.close();report.cleanup.ownedBrowserProcessClosed=true;}
 const absent=native?(()=>{try{process.kill(native.pid,0);return false;}catch(e){return e.code==='ESRCH';}})():true;report.cleanup.nativeProcessAbsentImmediate=absent;await new Promise(r=>setTimeout(r,500));report.cleanup.nativeProcessAbsentLate=native?(()=>{try{process.kill(native.pid,0);return false;}catch(e){return e.code==='ESRCH';}})():true;
 if(report.cleanup.nativeProcessAbsentLate){fs.rmSync(fixture,{recursive:true,force:true});report.cleanup.ownedFixtureRemoved=!fs.existsSync(fixture);}
 save();stage('case-cleaned',{functionalPassed:report.passed,nativeExit:nativeExit?.code,forced:report.cleanup.nativeForced===true});
}
report.cleanupQualified=report.cleanup.browser?.closeReturned===true&&report.cleanup.browser?.freed===true&&report.cleanup.websocketsClosedBeforeContext===true&&report.cleanup.nativeExit?.code===0&&report.cleanup.nativeForced!==true&&report.cleanup.ownedBrowserContextClosed===true&&report.cleanup.ownedBrowserProcessClosed===true&&report.cleanup.nativeProcessAbsentImmediate===true&&report.cleanup.nativeProcessAbsentLate===true&&report.cleanup.ownedFixtureRemoved===true;
report.qualified=report.passed&&report.cleanupQualified;save();stage('final-qualification',{functionalPassed:report.passed,cleanupPassed:report.cleanupQualified,qualified:report.qualified});process.exitCode=report.qualified?0:1;
