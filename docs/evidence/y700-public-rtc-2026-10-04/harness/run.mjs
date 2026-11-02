import {createServer} from 'node:http';
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {spawn,execFileSync} from 'node:child_process';
import {createInterface} from 'node:readline';
import {randomBytes,createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {connect as tlsConnect} from 'node:tls';

const base='/tmp/miopunch-y700-public-rtc';
const require=createRequire('/home/js/Git/miopunch/browser-probe/harness/package.json');
const {chromium}=require('playwright-core');
const adb='/home/js/Android/Sdk/platform-tools/adb', serial='100.64.10.11:44117';
const output=process.argv[2]; if(!output)throw new Error('output required'); mkdirSync(output,{recursive:false});
const source=readFileSync(base+'/probe.js','utf8');(0,eval)(source);
globalThis.MioY700IrcSocket=class extends EventTarget {
 constructor(){super();this.readyState=0;this.protocol='native-irc-tls';let pending='';this.socket=tlsConnect({host:'irc.ergo.chat',port:6697,servername:'irc.ergo.chat',rejectUnauthorized:true});this.socket.setTimeout(10000,()=>this.socket.destroy(new Error('IRC TLS socket timeout')));this.socket.on('secureConnect',()=>{this.socket.setTimeout(0);this.readyState=1;this.dispatchEvent(new Event('open'));});this.socket.on('data',data=>{pending+=String(data);let end;while((end=pending.indexOf('\n'))>=0){const line=pending.slice(0,end).replace(/\r$/,'');pending=pending.slice(end+1);this.dispatchEvent(new MessageEvent('message',{data:line}));}});this.socket.on('error',error=>{this.error=String(error);this.dispatchEvent(new Event('error'));});this.socket.on('close',()=>{this.readyState=3;const event=new Event('close');Object.assign(event,{code:1000,reason:this.error??'normal'});this.dispatchEvent(event);});}
 send(value){this.socket.write(value);}
 close(){this.readyState=2;this.socket.end();this.socket.destroy();}
};
const adbRun=(...args)=>execFileSync('rtk',['proxy',adb,'-s',serial,...args],{encoding:'utf8',timeout:10000}).trim();
const report={scope:'Y700 real Chrome versus 192.168.4.5 Rust RTC or Chrome; public MQTT and IRC signaling only, no self-hosted message facility; carrier interoperability, not full SDK membership',startedUtc:new Date().toISOString(),adbControlOnly:true,vpnAllowed:true,cases:[],cleanup:[],sourceSha256:createHash('sha256').update(source).digest('hex')};
let web,forward,reverse,androidWs,hostBrowser,webPort;
const ownTargets=new Set(), children=new Set();
const save=()=>writeFileSync(output+'/receipt.json',JSON.stringify(report,null,2)+'\n');
class Cdp {
 constructor(url){this.ws=new WebSocket(url);this.pending=new Map();this.next=1;this.open=new Promise((resolve,reject)=>{this.ws.addEventListener('open',resolve);this.ws.addEventListener('error',()=>reject(new Error('CDP socket error')));});this.ws.addEventListener('message',ev=>{const m=JSON.parse(ev.data);const p=this.pending.get(m.id);if(p){this.pending.delete(m.id);m.error?p.reject(new Error(JSON.stringify(m.error))):p.resolve(m.result);}});this.ws.addEventListener('close',()=>{for(const p of this.pending.values())p.reject(new Error('CDP closed'));this.pending.clear();});}
 async send(method,params={},sessionId){await this.open;const id=this.next++;const p=new Promise((resolve,reject)=>this.pending.set(id,{resolve,reject}));this.ws.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));return MioY700.timeout(p,150000,'CDP '+method);}
 async eval(sessionId,expression){const r=await this.send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true},sessionId);if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description??r.exceptionDetails.text);return r.result.value;}
 close(){this.ws.close();}
}
function nativeProcess(){
 const process=spawn('rtk',['proxy',base+'/native'],{stdio:['pipe','pipe','pipe'],detached:true});children.add(process);const entries=[],waiting=[];let stderr='';
 createInterface({input:process.stdout}).on('line',line=>{let v;try{v=JSON.parse(line);}catch{return;}entries.push(v);for(const w of [...waiting])if(w.event===v.event){waiting.splice(waiting.indexOf(w),1);w.resolve(v);}});
 process.stderr.on('data',d=>stderr+=String(d));
 process.once('exit',(code,signal)=>{process.endResult={code,signal};children.delete(process);for(const w of waiting.splice(0))w.reject(new Error('native exit '+code+'/'+signal+' '+stderr));});
 function wait(event,ms=50000){const current=entries.find(v=>v.event===event);if(current)return Promise.resolve(current);let w;return MioY700.timeout(new Promise((resolve,reject)=>{w={event,resolve,reject};waiting.push(w);}),ms,'native '+event).finally(()=>{const i=waiting.indexOf(w);if(i>=0)waiting.splice(i,1);});}
 return {process,wait,entries,stderr:()=>stderr};
}
async function stopNative(native){if(!native||native.process.endResult)return;native.process.stdin.end();try{process.kill(-native.process.pid,'SIGTERM');}catch{} await MioY700.delay(200);if(!native.process.endResult){try{process.kill(-native.process.pid,'SIGKILL');}catch{}}await MioY700.timeout(new Promise(resolve=>{if(native.process.endResult)resolve();else native.process.once('exit',resolve);}),3000,'native stop');}
async function nativeCase(config){let session,native;const result={role:'host-native',passed:false};try{
 session=await MioY700.signal(config);result.signaling=session.report;
 const offer=await session.receive();if(offer.type!=='offer')throw new Error('native offer expected');
 native=nativeProcess();native.process.stdin.write(JSON.stringify({offer:offer.description,stun:process.argv.includes('--native-no-stun')?'':config.stun})+'\n');result.nativeIceServers=process.argv.includes('--native-no-stun')?[]:[config.stun];
 const answer=await native.wait('answer');await session.send({type:'answer',description:answer.description});
 await native.wait('open');await session.send({type:'rtc-open'});const ready=await session.receive();if(ready.type!=='rtc-open')throw new Error('browser RTC-open receipt missing');native.process.stdin.write('send\n');
 const matched=await native.wait('payload-matched');const got=await session.receive();if(got.type!=='payload-ok'||got.bytes!==matched.bytes)throw new Error('browser payload receipt missing');
 native.process.stdin.write('finish\n');const completed=await native.wait('complete');result.rtc=completed;result.passed=completed.passed;await session.send({type:'payload-ok',bytes:matched.bytes});
 await MioY700.delay(200);result.nativeExit=native.process.endResult;
 }catch(error){result.failure=String(error);if(error.signalReport)result.signaling=error.signalReport;if(native){result.nativeEvents=native.entries.map(({description,...rest})=>rest);result.nativeStderr=native.stderr();}}
 finally{session?.close();await stopNative(native);result.nativeStopped=!native||!!native.process.endResult;}
 return result;
}
try{
 web=createServer((req,res)=>{if(req.url==='/probe.js'){res.setHeader('Content-Type','text/javascript');res.end(source);}else{res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>miopunch Y700 temporary RTC test</title><p>Temporary real-device RTC test</p><script src="/probe.js"></script>');}});
 await new Promise(resolve=>web.listen(0,'127.0.0.1',resolve));webPort=web.address().port;
 reverse=`tcp:${webPort}`;adbRun('reverse',reverse,reverse);
 const setupUrl=`http://127.0.0.1:${webPort}/?mio-y700-setup=${Date.now()}`;
 adbRun('shell','am','start','--user','0','-a','android.intent.action.VIEW','-d',setupUrl,'com.android.chrome');
 forward=adbRun('forward','tcp:0','localabstract:chrome_devtools_remote');
 const metadata=await(await fetch(`http://127.0.0.1:${forward}/json/version`,{signal:AbortSignal.timeout(8000)})).json();report.androidBrowser={Browser:metadata.Browser,ProtocolVersion:metadata['Protocol-Version'],UserAgent:metadata['User-Agent']};
 androidWs=new Cdp(metadata.webSocketDebuggerUrl);await MioY700.timeout(androidWs.open,8000,'Android browser CDP');
 for(const target of (await androidWs.send('Target.getTargets')).targetInfos)if(target.url===setupUrl||target.url==='http://127.0.0.1:44856/?miopunch-y700-setup=run01')ownTargets.add(target.targetId);
 hostBrowser=await chromium.launch({executablePath:'/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome',headless:true,args:['--no-sandbox','--disable-features=WebRtcHideLocalIpsWithMdns'],ignoreHTTPSErrors:false});report.hostBrowser=hostBrowser.version();
 const context=await hostBrowser.newContext();
 const stun='stun:74.125.250.129:19302';
 for(const carrier of ['mqtt','irc'])for(const kind of (process.argv.includes('--native-only')?['native']:['native','browser'])){
  const run=randomBytes(9).toString('hex'),key=randomBytes(32).toString('base64');
  const url=carrier==='mqtt'?'wss://broker.emqx.io:8084/mqtt':'wss://irc.ergo.chat/webirc';
  const row={carrier,kind,run,url,startedUtc:new Date().toISOString(),passed:false};report.cases.push(row);save();console.log(JSON.stringify({event:'case-start',carrier,kind,url}));
  let target,sessionId,page;
  try{
   const testUrl=carrier==='irc'?'https://irc.ergo.chat/':`http://127.0.0.1:${webPort}/`;
   ({targetId:target}=await androidWs.send('Target.createTarget',{url:testUrl}));ownTargets.add(target);
   ({sessionId}=await androidWs.send('Target.attachToTarget',{targetId:target,flatten:true}));await androidWs.send('Page.bringToFront',{},sessionId);
   let loaded=false;for(let n=0;n<80;n++){loaded=await androidWs.eval(sessionId,carrier==='irc'?'document.readyState==="complete"&&location.origin==="https://irc.ergo.chat"':'typeof startMioY700==="function"');if(loaded)break;await MioY700.delay(100);}if(!loaded)throw new Error('Android test page did not load');
   if(carrier==='irc')await androidWs.eval(sessionId,source+'; true');
   const common={carrier,url,run,key,stun};
   const remoteConfig={...common,role:'y700',other:'host'};const hostConfig={...common,role:'host',other:'y700',nativeIrc:carrier==='irc'&&kind==='native'};
   let host;
   if(kind==='native')host=nativeCase(hostConfig);
   else{page=await context.newPage();await page.goto(testUrl);if(carrier==='irc')await page.addScriptTag({content:source});host=page.evaluate(config=>startMioY700(config),hostConfig);}
   const remote=androidWs.eval(sessionId,`startMioY700(${JSON.stringify(remoteConfig)})`);
   [row.y700,row.host]=await MioY700.timeout(Promise.all([remote,host]),120000,'physical case total');
   row.passed=row.y700.passed&&row.host.passed;
  }catch(error){row.failure=String(error);}
  finally{if(page)await page.close().catch(()=>{});if(target){await androidWs.send('Target.closeTarget',{targetId:target}).catch(()=>{});ownTargets.delete(target);}row.endedUtc=new Date().toISOString();save();console.log(JSON.stringify({event:'case-end',carrier,kind,passed:row.passed,failure:row.failure??row.y700?.failure??row.y700?.rtc?.failure??row.host?.failure}));}
 }
 report.passed=report.cases.length===(process.argv.includes('--native-only')?2:4)&&report.cases.every(v=>v.passed);
}catch(error){report.failure=String(error);report.passed=false;}
finally{
 for(const targetId of ownTargets){await androidWs?.send('Target.closeTarget',{targetId}).catch(()=>{});}ownTargets.clear();
 for(const child of children){try{process.kill(-child.pid,'SIGKILL');}catch{} await new Promise(resolve=>{if(child.endResult)resolve();else child.once('exit',resolve);});}
 if(hostBrowser)await hostBrowser.close().catch(error=>report.cleanup.push({hostBrowser:String(error)}));
 androidWs?.close();
 for(const [kind,port] of [['forward',forward?`tcp:${forward}`:null],['reverse',reverse]])if(port){try{adbRun(kind,'--remove',port);report.cleanup.push({kind,port,removed:true});}catch(error){report.cleanup.push({kind,port,removed:false,error:String(error)});}}
 if(web){web.closeAllConnections();await new Promise(resolve=>web.close(resolve));}
 report.endedUtc=new Date().toISOString();save();
}
console.log(JSON.stringify({event:'finished',passed:report.passed,output}));
process.exitCode=report.passed?0:1;
