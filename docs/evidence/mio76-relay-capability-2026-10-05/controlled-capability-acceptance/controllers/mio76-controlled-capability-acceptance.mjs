import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {createServer as tcpServer,connect as tcpConnect} from 'node:net';
import {readFileSync,writeFileSync,existsSync,readdirSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {createInterface} from 'node:readline';
import {chromium} from '/home/js/Git/miopunch/browser-probe/harness/node_modules/playwright-core/index.mjs';
import {startMixedMqttProxy} from '/home/js/Git/miopunch/browser-probe/harness/owned-mixed-mqtt-proxy.mjs';
import {rtcCapabilityPrelude} from '/home/js/Git/miopunch/browser-probe/harness/rtc-capability-fixture.mjs';

const arg=name=>process.argv[process.argv.indexOf(name)+1];
const fixture=resolve(arg('--fixture')),output=resolve(arg('--output')),pkg=resolve(arg('--package'));
const caseName=process.env.MIO76_CAPABILITY_CASE??'disabled';
const cases={disabled:['observe',false,'disabled-by-policy'],'pc-denied':['peer-connection-denied',true,'peer-connection-rejected'],'dc-denied':['data-channel-denied',true,'data-channel-rejected'],'no-data-path':['observe',false,'disabled-by-policy']};
assert(caseName in cases);const [fixtureMode,rtcEnabled,capabilityReason]=cases[caseName];
const report={scope:'controlled actual Browser SDK and reopened Native SDK with owned MQTT/TLS relay; no public/device/NAT qualification',passed:false,startedUtc:new Date().toISOString(),nativeEvents:[],nativeRelay:[],stages:[],cleanup:[]};
report.case=caseName;report.callerBudgetMs=12000;report.nativeRelayStageBudgetMs=6000;report.trust='owned private CA/NSS only; certificate verification enabled';
const servers=[],sockets=new Set();
let relay,native,context,page,web;
const hide=new Set(['id','peer','remoteId','localId','invitation','request','applicant','knownMembers','description','sdp','binding','exporter','secret','networkAdmissionSecret','address','route','selectedRoute','stableId','memberStableId','pathId','privateStreamId','streamId','authorization','records']);
function publicValue(value){if(Array.isArray(value))return value.map(publicValue);if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).filter(([key,v])=>!hide.has(key)&&!(key==='code'&&typeof v==='string')).map(([key,v])=>[key,publicValue(v)]));return value;}
function save(){writeFileSync(join(output,'receipt.json'),JSON.stringify(publicValue(report),null,2)+'\n');}
let rejectWork;
const stopWork=new Promise((_,reject)=>{rejectWork=reject;});stopWork.catch(()=>{});
const timer=setTimeout(()=>rejectWork(new Error('controlled fixture work deadline')),55000);
for(const name of ['SIGTERM','SIGINT'])process.once(name,()=>rejectWork(new Error('controlled fixture cancelled by '+name)));
const bounded=p=>Promise.race([p,stopWork]);
const relayDelayMs=Number(process.env.MIO76_RELAY_CHUNK_DELAY_MS??0);assert(Number.isInteger(relayDelayMs)&&relayDelayMs>=0&&relayDelayMs<=1500);
let relayDelayEnabled=false;
const relayTimers=new Set();
async function relayProxy(backend){
 const target=new URL(backend);report.relayProxy={controlledChunkDelayMs:relayDelayMs,enabledAtMember:false,connections:[]};
 const server=tcpServer(client=>{
  sockets.add(client);client.on('error',()=>{});client.once('close',()=>sockets.delete(client));
  const remote=tcpConnect({host:'127.0.0.1',port:Number(target.port)});sockets.add(remote);remote.on('error',()=>client.destroy());remote.once('close',()=>{sockets.delete(remote);client.destroy();});client.once('close',()=>remote.destroy());
  const item={index:report.relayProxy.connections.length,clientToRelay:{chunks:0,bytes:0,delayedChunks:0},relayToClient:{chunks:0,bytes:0,delayedChunks:0}};report.relayProxy.connections.push(item);
  function copy(source,destination,key){source.on('data',bytes=>{const observation=item[key];observation.chunks++;observation.bytes+=bytes.length;if(relayDelayEnabled&&relayDelayMs){observation.delayedChunks++;const timer=setTimeout(()=>{relayTimers.delete(timer);if(!destination.destroyed)destination.write(bytes);},relayDelayMs);relayTimers.add(timer);}else destination.write(bytes);});}
  copy(client,remote,'clientToRelay');copy(remote,client,'relayToClient');
 });servers.push(server);await bounded(new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);}));return `https://localhost:${server.address().port}/`;
}
async function stage(name,action){let start=performance.now();try{const value=await bounded(action());report.stages.push({name,ok:true,elapsedMs:performance.now()-start});save();return value;}catch(e){report.stages.push({name,ok:false,elapsedMs:performance.now()-start,failure:String(e)});save();throw e;}}
function actor(command,args,label){
 const child=spawn('rtk',['proxy',command,...args],{stdio:['pipe','pipe','pipe']});const events=[],waiters=[];
 const stdout=createInterface({input:child.stdout}),stderr=createInterface({input:child.stderr});
 const readers=Promise.all([stdout,stderr].map(reader=>new Promise(resolve=>reader.once('close',resolve))));
 stdout.on('line',line=>{let value;try{value=JSON.parse(line);}catch{return;}events.push(value);if(label==='native')report.nativeEvents.push(publicValue(value));for(const w of [...waiters])if(w.event===value.event){waiters.splice(waiters.indexOf(w),1);w.resolve(value);}});
 stderr.on('line',line=>{if(line.startsWith('[DEBUG-mio76-relay] ')){try{report.nativeRelay.push(JSON.parse(line.slice('[DEBUG-mio76-relay] '.length)));}catch{report.malformedNative=(report.malformedNative??0)+1;}}else if(line.startsWith('UNIFIED_CANDIDATE_FAILED'))report.nativeFailure=line.replace(/[0-9a-f]{32,}/g,'<identifier>');else if(line.startsWith('Error:'))report.nativeFatalCategory=line.slice(0,128).replace(/[0-9a-f]{32,}/g,'<identifier>');});
 child.once('error',e=>rejectWork(e));child.once('exit',(code,signal)=>{for(const w of waiters.splice(0))w.reject(new Error(`${label} exited ${code}/${signal} before ${w.event}`));});
 return {child,events,readers,wait(event,fresh=false){const found=!fresh&&events.find(v=>v.event===event);if(found)return Promise.resolve(found);return bounded(new Promise((resolve,reject)=>waiters.push({event,resolve,reject})));},command(line,event){const p=this.wait(event,true);child.stdin.write(line+'\n');return p;}};
}
async function closeActor(owner,label){
 if(!owner)return;const p=owner.child;if(p.exitCode!==null||p.signalCode!==null){await owner.readers;report.cleanup.push({label,code:p.exitCode,signal:p.signalCode,alreadyExited:true,readersEnded:true,sdkRetirement:owner.events.find(v=>v.event==='closed')??null});return;}
 let forced=false;const ended=new Promise(resolve=>p.once('exit',(code,signal)=>resolve({code,signal})));
 p.stdin.end('stop\n');const term=setTimeout(()=>{forced=true;p.kill('SIGTERM');},3000);const kill=setTimeout(()=>{forced=true;p.kill('SIGKILL');},5000);
 const result=await ended;clearTimeout(term);clearTimeout(kill);await owner.readers;report.cleanup.push({label,...result,forced,readersEnded:true,sdkRetirement:owner.events.find(v=>v.event==='closed')??null});
}
function startNative(mode,relayUrl){return actor(arg('--native'),[join(fixture,'native-store'),join(fixture,'facility.json'),join(fixture,'ca.pem'),mode,...(relayUrl?[relayUrl]:[])],'native');}
async function checkCapability(generation){
 const observed=await page.evaluate(()=>({paths:JSON.parse(window.network.path_diagnostics()),objects:window.__mio76RtcFixture.snapshot(),status:JSON.parse(window.network.status())}));
 const cap=observed.paths.rtcCapability;assert(cap);assert.equal(cap.reason,capabilityReason);assert.equal(cap.supported,rtcEnabled?false:null);assert.equal(cap.permitted,rtcEnabled);assert.equal(cap.effective,false);
 assert.equal(observed.objects.openPeerConnections,0);assert.equal(observed.objects.openDataChannels,0);assert.equal(observed.objects.channelCreated,0);
 assert.equal(observed.objects.peerCreated,caseName==='dc-denied'?1:0);assert.equal(observed.objects.peerCloseCalls,caseName==='dc-denied'?1:0);
 assert.equal(observed.status.running,true);assert.equal(observed.status.stage,'Complete');assert.equal(observed.status.authorization_saved,true);assert.equal(observed.status.secret_saved,true);
 report.capabilityObservations??=[];report.capabilityObservations.push({generation,capability:cap,objects:observed.objects,status:observed.status});return observed;
}
async function closeBrowserSdk(label){
 const observed=await page.evaluate(async()=>{
  const members=[];if(window.member){await window.member.close();members.push(JSON.parse(window.member.rtc_path_diagnostics()));window.member.free();window.member=null;}
  if(window.network){await window.network.close();window.network.free();window.network=null;}
  return{members,objects:window.__mio76RtcFixture.snapshot()};
 });
 await new Promise(resolve=>setTimeout(resolve,100));const late=await page.evaluate(()=>window.__mio76RtcFixture.snapshot());
 assert.equal(late.openPeerConnections,0);assert.equal(late.openDataChannels,0);assert.equal(late.openWebSockets,0);
 for(const member of observed.members){assert.equal(member.activeAssociations,0);assert.equal(member.activePumpTasks,0);}
 report.cleanup.push({label,closed:true,observed,late});
}
const sessionIds=new Set();
async function openAndExchange(nativeId,browserId,label){
 const releaseStart=native.events.length;
 const member=await stage(label,()=>page.evaluate(async id=>{const start=performance.now();try{window.member=await window.network.open_member_connection(id);return{returned:true,callerMs:performance.now()-start,diagnostics:JSON.parse(window.member.rtc_path_diagnostics()),session:JSON.parse(window.member.session_diagnostics())};}catch(e){return{returned:false,callerMs:performance.now()-start,failure:String(e)};}},nativeId));
 report.members??=[];report.members.push(member);assert.equal(member.returned,true,member.failure);assert(member.callerMs<=12000);
 const release=native.events.slice(releaseStart).find(v=>v.event==='member-released')??await native.wait('member-released',true);
 assert.equal(release.remoteId,browserId);assert.equal(release.alpn,'miopunch/session/member/1');assert.equal(release.memberAuthorized,true);assert(release.paths.some(v=>v.isRelay&&v.selected));
 assert(!sessionIds.has(release.stableId));sessionIds.add(release.stableId);
 const nativePath=native.events.slice(releaseStart).find(v=>v.event==='unified-path-ready');assert(nativePath);assert.equal(nativePath.freshAck,true);assert.equal(nativePath.role,'responder');assert.equal(nativePath.version,1);
 const paths=member.diagnostics;assert.equal(paths.activeAssociations,0);assert.equal(paths.activePumpTasks,0);assert.equal(paths.closed,false);assert(paths.paths.some(v=>v.isRelay&&v.selected));assert(!paths.paths.some(v=>v.isRtc));
 assert.equal(member.session.newProtocol,true);assert.equal(member.session.alpn,'miopunch/session/member/1');assert.equal(member.session.ordinaryHandle,true);assert.equal(member.session.expectedPeerMatched,true);assert.equal(member.session.memberAuthorized,true);
 assert.equal(paths.bootstrap.capabilitiesVersion,1);assert.equal(paths.bootstrap.stableUnchanged,true);assert.equal(paths.bootstrap.bindingUnchanged,true);assert.equal(paths.bootstrap.coordinator.freshAck,true);assert.equal(paths.bootstrap.coordinator.role,'initiator');assert.equal(paths.bootstrap.coordinator.rtcRole,'None');assert.equal(paths.bootstrap.validatedRelay.freshAck,true);
 const relayChecks=JSON.parse(await page.evaluate(()=>window.network.path_diagnostics())).relayChecks;assert(relayChecks.some(v=>v.cachedGateValid===true&&v.storageGateCheckPassed===true&&v.localSupportedMask===4&&v.localPermittedMask===4&&v.rtcRole===0));
 const text='MIO76 '+caseName+' controlled real SDK PONG';assert.equal(await page.evaluate(async text=>window.member.exchange(text),text),text);
 const business=native.events.slice(releaseStart).find(v=>v.event==='business')??await native.wait('business',true);assert.equal(business.stableId,release.stableId);assert.equal(business.bytes,Buffer.byteLength(text));assert.equal(business.alpn,'miopunch/session/member/1');
 Object.assign(member,{strictMemberAlpn:true,bothIdentitiesMatched:true,bothMemberGatesVerified:true,bothFreshAck:true,businessMatched:true,requestBytes:Buffer.byteLength(text),responseBytes:Buffer.byteLength(text),nativePath,nativeRelease:release,nativeBusiness:business,newOrdinarySession:true});
 report.browserRelay=JSON.parse(await page.evaluate(()=>window.network.path_diagnostics()));
}
try{
 relay=actor(arg('--relay-native'),['relay-tls-files',join(fixture,'leaf.der'),join(fixture,'leaf-key.der')],'relay');
 const backendRelay=await bounded(new Promise((resolve,reject)=>{createInterface({input:relay.child.stdout}).on('line',line=>{if(line.startsWith('RELAY '))resolve(line.slice(6));});relay.child.once('exit',()=>reject(new Error('owned relay exited before ready')));}));
 const relayUrl=await relayProxy(backendRelay);
 report.relay=relayUrl;
 const facility=await startMixedMqttProxy({fixture,broker:arg('--broker'),report,ownedSockets:sockets,dropPublish:()=>false,listen:async server=>{servers.push(server);await bounded(new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);}));return server.address().port;}});
 writeFileSync(join(fixture,'facility.json'),JSON.stringify(facility));
 native=startNative('admission');const first=await stage('native-admission-ready',()=>native.wait('ready'));const nativeId=first.id;
 web=createServer((req,res)=>{const path=new URL(req.url,'http://localhost').pathname;if(path==='/'){res.setHeader('Content-Type','text/html');return res.end('<!doctype html><title>owned SDK ACK diagnostic</title>');}if(!path.startsWith('/pkg/')||path.includes('..'))return res.writeHead(404).end();const file=join(pkg,path.slice(5));if(!existsSync(file))return res.writeHead(404).end();res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':'text/javascript');res.end(readFileSync(file));});
 await bounded(new Promise(resolve=>web.listen(0,'127.0.0.1',resolve)));report.origin=`http://127.0.0.1:${web.address().port}`;
 const executable='/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome';assert(existsSync(executable));
 const wrapper=join(fixture,'chromium-private-nss.sh');
 const q=v=>"'"+v.replaceAll("'","'\\''")+"'";
 writeFileSync(wrapper,`#!/bin/sh\nexec rtk proxy bwrap --ro-bind / / --tmpfs /tmp --bind ${q(fixture)} ${q(fixture)} --bind ${q(join(fixture,'nssdb'))} /home/js/.pki/nssdb --dev-bind /dev /dev --proc /proc -- ${q(executable)} "$@"\n`,{mode:0o700});
 context=await stage('browser-private-context',()=>chromium.launchPersistentContext(join(fixture,'browser-profile'),{executablePath:wrapper,headless:true,args:['--no-sandbox'],ignoreHTTPSErrors:false}));
 await context.addInitScript(rtcCapabilityPrelude(fixtureMode));page=await context.newPage();
 await bounded(page.goto(report.origin));
 await bounded(page.evaluate(async()=>{const m=await import('/pkg/miopunch_browser_probe.js');await m.default();window.BrowserNetwork=m.BrowserNetwork;}));
 const cfg=ready=>({relays:[],disabled:[],irc:[],mqtt:[facility],rtc_enabled:rtcEnabled,contact_routes:[{contact:ready.id,facility:facility.logical_id,address:ready.route}]});
 const browserId=await stage('browser-admission-start',()=>page.evaluate(async configuration=>{window.network=await window.BrowserNetwork.start(JSON.stringify(configuration));return window.network.id();},cfg(first)));assert.notEqual(browserId,nativeId);
 assert.equal(await stage('request-admission',()=>page.evaluate(invitation=>window.network.request_admission(invitation),first.invitation)),'AwaitingApproval');
 const pending=await native.command('admission-status','admission-status');const requests=pending.requests.filter(v=>v.applicant===browserId);assert.equal(requests.length,1);
 await native.command(`approve-request ${requests[0].applicant} ${requests[0].request}`,'approved');
 assert.equal(await stage('request-approved-materials',()=>page.evaluate(invitation=>window.network.request_admission(invitation),first.invitation)),'PendingMaterials');
 assert.equal(await stage('complete-admission',()=>page.evaluate(()=>window.network.complete_admission())),'Complete');
 report.savedMaterials=JSON.parse(await page.evaluate(()=>window.network.status()));
 await closeBrowserSdk('browser-admission-sdk');await closeActor(native,'native-admission');native=null;
 native=startNative('unified-member',relayUrl);const second=await stage('native-member-reopened',()=>native.wait('ready'));assert.equal(second.id,nativeId);assert(second.knownMembers.includes(browserId));
 const memberCfg={...cfg(second),relays:[relayUrl]};
 await page.evaluate(()=>window.__mio76RtcFixture.resetCounts());
 const reopened=await stage('browser-member-reopened',()=>page.evaluate(async configuration=>{window.network=await window.BrowserNetwork.start(JSON.stringify(configuration));return window.network.id();},memberCfg));assert.equal(reopened,browserId);report.identitiesReopened={browser:true,native:true};
 await checkCapability('member-reopened');
 if(process.env.MIO76_REQUIRE_ACTUAL_HOME==='1'){
  await stage('browser-actual-home-connected',()=>page.waitForFunction(()=>JSON.parse(window.network.path_diagnostics()).relayHome?.connectedCount===1));
  report.actualHomeBeforeDelay=JSON.parse(await page.evaluate(()=>window.network.path_diagnostics())).relayHome;
 }else await bounded(page.waitForFunction(()=>window.__mio76RtcFixture.snapshot().sockets.some(socket=>socket.origin.includes('localhost')&&socket.readyState===1)));
 relayDelayEnabled=true;report.relayProxy.enabledAtMember=true;
 await openAndExchange(nativeId,browserId,'ordinary-sdk-member');
 if(caseName==='no-data-path'){
  await closeBrowserSdk('browser-before-no-data');await page.evaluate(()=>window.__mio76RtcFixture.resetCounts());
  const blockedCfg={...memberCfg,disabled:[relayUrl]};
  const blockedId=await stage('browser-no-data-reopened',()=>page.evaluate(async cfg=>{window.network=await window.BrowserNetwork.start(JSON.stringify(cfg));return window.network.id();},blockedCfg));assert.equal(blockedId,browserId);
  const before=await checkCapability('no-data-reopened');assert.equal(before.paths.carriers.length,1);assert.equal(before.paths.carriers[0].ready,true);assert.equal(before.paths.carriers[0].closed,false);assert.equal(before.paths.carriers[0].traffic.sent_bytes,0);assert.equal(before.paths.carriers[0].traffic.received_bytes,0);
  const nativeMark=native.events.length;
  const failure=await stage('no-data-sdk-member',()=>page.evaluate(async id=>{const start=performance.now();try{window.member=await window.network.open_member_connection(id);return{returned:true,callerMs:performance.now()-start};}catch(error){return{returned:false,callerMs:performance.now()-start,error:String(error),ordinaryHandle:!!window.member};}},nativeId));
  assert.equal(failure.returned,false);assert.equal(failure.ordinaryHandle,false);assert.match(failure.error,/DataPathUnavailable|no currently permitted data path/);assert(failure.callerMs<=12000);
  const after=await checkCapability('no-data-after-failure');assert.equal(after.status.id,browserId);assert.equal(after.paths.carriers[0].ready,true);assert.equal(after.paths.carriers[0].closed,false);assert.equal(after.paths.carriers[0].traffic.sent_bytes,0);assert.equal(after.paths.carriers[0].traffic.received_bytes,0);
  assert(!native.events.slice(nativeMark).some(v=>['member-released','business'].includes(v.event)));
  report.noDataPath={errorClass:'DataPathUnavailable',ordinaryHandleReturned:false,mqttContactReady:true,mqttQuicBytesSent:0,mqttQuicBytesReceived:0,nativeBusinessEvents:0,identityAndMembershipUnchanged:true,runningNodeUnchanged:true,failure};
  await closeBrowserSdk('browser-before-restore');await page.evaluate(()=>window.__mio76RtcFixture.resetCounts());
  const restored=await stage('browser-relay-restored',()=>page.evaluate(async cfg=>{window.network=await window.BrowserNetwork.start(JSON.stringify(cfg));return window.network.id();},memberCfg));assert.equal(restored,browserId);
  await checkCapability('restored');await stage('restored-actual-home-connected',()=>page.waitForFunction(()=>JSON.parse(window.network.path_diagnostics()).relayHome?.connectedCount===1));
  await openAndExchange(nativeId,browserId,'ordinary-sdk-member-after-restore');report.newAttemptAfterRestore=true;
 }
 report.passed=true;
}catch(e){report.failure=String(e);}
finally{
 clearTimeout(timer);
 if(page){try{report.browserFinal=JSON.parse(await page.evaluate(()=>window.network?.path_diagnostics()??'{}'));await closeBrowserSdk('browser-sdk');}catch(e){report.cleanup.push({label:'browser-sdk',closed:false,failure:String(e)});report.passed=false;}}
 if(context){try{await context.close();report.cleanup.push({label:'browser-context',closed:true});}catch(e){report.cleanup.push({label:'browser-context',closed:false,failure:String(e)});}}
 await closeActor(native,'native-member');await closeActor(relay,'owned-relay');
 for(const timer of relayTimers)clearTimeout(timer);relayTimers.clear();
 for(const socket of sockets)socket.destroy();
 for(const server of servers)await new Promise(resolve=>server.close(resolve));
 if(web)await new Promise(resolve=>web.close(resolve));
 report.cleanup.push({label:'local-services',socketSetEmpty:sockets.size===0,serversClosed:true});
 for(const item of report.cleanup.filter(v=>v.label==='native-admission'||v.label==='native-member')){
  const retired=item.sdkRetirement;const qualified=item.code===0&&item.forced===false&&item.readersEnded===true&&retired?.activeRtcAssociations===0&&retired?.activeRtcPumps===0&&retired?.traffic?.driver_active===0&&retired?.traffic?.queued_packets===0&&Number.isInteger(retired?.socketsBefore)&&Number.isInteger(retired?.socketsLate)&&retired.socketsLate<=retired.socketsBefore;
  item.sdkShutdownQualified=qualified;if(!qualified)report.passed=false;
 }
 if(report.cleanup.some(v=>v.closed===false||v.forced===true||v.socketSetEmpty===false||('code'in v&&v.code!==0)))report.passed=false;
 report.completedUtc=new Date().toISOString();save();
}
console.log(JSON.stringify({passed:report.passed,failure:report.failure,members:report.members?.length,receipt:join(output,'receipt.json')}));
process.exitCode=report.passed?0:1;
