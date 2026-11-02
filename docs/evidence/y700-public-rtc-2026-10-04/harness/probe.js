(function () {
  const text = new TextEncoder(), utf = new TextDecoder();
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function timeout(p, ms, what) {
    let timer;
    return Promise.race([p, new Promise((_, reject) => timer = setTimeout(() => reject(new Error(what+' deadline')), ms))]).finally(() => clearTimeout(timer));
  }
  const b64 = bytes => { let s=''; for (const b of bytes) s+=String.fromCharCode(b); return btoa(s); };
  const un64 = s => Uint8Array.from(atob(s), c=>c.charCodeAt(0));
  const join = (...parts) => { const out=new Uint8Array(parts.reduce((n,p)=>n+p.length,0)); let n=0; for(const p of parts){out.set(p,n);n+=p.length;} return out; };
  function str(s) { const b=text.encode(s); return join(Uint8Array.of(b.length>>8,b.length&255),b); }
  function mqttPacket(kind, body) { let n=body.length, len=[]; do{let v=n%128;n=Math.floor(n/128);len.push(v|(n?128:0));}while(n);return join(Uint8Array.of(kind,...len),body); }
  async function signal(config) {
    const incoming=[], waiting=[], assemblies=new Map();
    const report={carrier:config.carrier,url:config.url,normalTLS:true,ready:false,txMessages:0,rxMessages:0,txFragments:0,rxFragments:0,events:[]};
    const key=await crypto.subtle.importKey('raw',un64(config.key),'AES-GCM',false,['encrypt','decrypt']);
    let failure, closed=false, mqttPending=new Uint8Array(), messageNumber=0, keepalive;
    let readyResolve,readyReject;
    const ready=new Promise((resolve,reject)=>{readyResolve=resolve;readyReject=reject;}); ready.catch(()=>{});
    const ws=config.nativeIrc ? new MioY700IrcSocket() : new WebSocket(config.url, config.carrier==='mqtt' ? ['mqtt'] : ['text.ircv3.net']);
    if(config.nativeIrc)report.url='ircs://irc.ergo.chat:6697';
    ws.binaryType='arraybuffer';
    const mark=(kind, extra={})=>report.events.push({kind,at:Date.now(),...extra});
    function fail(error) { failure=String(error); mark('failure',{failure}); readyReject(new Error(failure)); for(const w of waiting.splice(0))w.reject(new Error(failure)); }
    async function deliver(encoded) {
      try {
        const packet=JSON.parse(encoded);
        if(packet.run!==config.run || packet.to!==config.role || packet.from===config.role)return;
        const bytes=un64(packet.data);
        const aad=text.encode(config.run+'|'+packet.from+'|'+packet.to);
        const body=await crypto.subtle.decrypt({name:'AES-GCM',iv:bytes.slice(0,12),additionalData:aad},key,bytes.slice(12));
        const value=JSON.parse(utf.decode(body));
        report.rxMessages++; mark('received',{kind:value.type,bytes:bytes.length});
        if(waiting.length)waiting.shift().resolve(value);else incoming.push(value);
      } catch(error) { fail(error); }
    }
    const ourTopic=`miopunch/y700/${config.run}/${config.role}`;
    const theirTopic=`miopunch/y700/${config.run}/${config.other}`;
    const room='#mioy'+config.run.slice(0,12), nick='my'+config.role[0]+config.run.slice(-8);
    function mqttReceive(chunk) {
      mqttPending=join(mqttPending,new Uint8Array(chunk));
      if(mqttPending.length>65536)throw new Error('MQTT receive bound');
      while(mqttPending.length>=2){
        let n=0,i=1,m=1,d;
        do{if(i>=mqttPending.length)return;d=mqttPending[i++];n+=(d&127)*m;m*=128;if(i>5)throw new Error('MQTT length');}while(d&128);
        if(mqttPending.length<i+n)return;
        const p=mqttPending.slice(i,i+n), kind=mqttPending[0]>>4; mqttPending=mqttPending.slice(i+n);
        if(kind===2){if(p[1]!==0)throw new Error('CONNACK '+p[1]);mark('connack',{code:p[1]});ws.send(mqttPacket(0x82,join(Uint8Array.of(0,1),str(ourTopic),Uint8Array.of(0))));}
        if(kind===9){if(p[2]!==0)throw new Error('SUBACK '+p[2]);report.ready=true;mark('suback',{code:p[2]});keepalive=setInterval(()=>{if(ws.readyState===1){ws.send(Uint8Array.of(0xc0,0));mark('pingreq');}},15000);readyResolve();}
        if(kind===13)mark('pingresp');
        if(kind===3){const count=(p[0]<<8)|p[1],topic=utf.decode(p.slice(2,2+count));if(topic===ourTopic)deliver(utf.decode(p.slice(2+count)));}
      }
    }
    function ircLine(line) {
      if(line.startsWith('PING ')){ws.send('PONG '+line.slice(5)+'\r\n');return;}
      const code=line.match(/^[^ ]+ (\d{3}) /)?.[1];
      if(code==='001'){mark('registered');ws.send('JOIN '+room+'\r\n');}
      if(code==='366' && line.includes(room)){report.ready=true;mark('join-ready',{room});readyResolve();}
      if(code==='433'||code==='464'||code==='465'||code==='471'||code==='473'||code==='474'||code==='475')throw new Error('IRC numeric '+code);
      if(line.startsWith('ERROR'))throw new Error(line);
      const match=line.match(/ PRIVMSG [^ ]+ :MIOY:([a-z0-9]+):(\d+):(\d+):(.+)$/);
      if(!match)return;
      const [,id,offset,total,fragment]=match;
      const index=Number(offset),count=Number(total);
      if(count>128||index<0||index>=count)throw new Error('IRC fragment bound');
      if(!assemblies.has(id)){if(assemblies.size>4)throw new Error('IRC assembly bound');assemblies.set(id,{chunks:new Array(count),start:Date.now()});}
      const assembly=assemblies.get(id); if(Date.now()-assembly.start>60000)throw new Error('IRC assembly expired');
      assembly.chunks[index]=fragment;report.rxFragments++;
      if(assembly.chunks.filter(v=>v!==undefined).length===count){assemblies.delete(id);deliver(assembly.chunks.join(''));}
    }
    ws.addEventListener('open',()=>{mark('websocket-open',{protocol:ws.protocol});
      if(config.carrier==='mqtt')ws.send(mqttPacket(0x10,join(str('MQTT'),Uint8Array.of(4,2,0,30),str('mioy-'+config.run+'-'+config.role))));
      else for(const line of [`CAP LS 302`,`NICK ${nick}`,`USER ${nick} 0 * :temporary RTC interoperability test`,`CAP END`])ws.send(line+'\r\n');
    });
    ws.addEventListener('message',event=>{try{if(config.carrier==='mqtt')mqttReceive(event.data);else for(const line of String(event.data).split(/\r?\n/).filter(Boolean))ircLine(line);}catch(error){fail(error);}});
    ws.addEventListener('error',()=>fail('WebSocket error at '+config.url));
    ws.addEventListener('close',event=>{mark('websocket-close',{code:event.code,reason:event.reason});if(!closed)fail('unexpected WebSocket close '+event.code);});
    try { await timeout(ready,15000,'public '+config.carrier+' ready'); }
    catch(error) { closed=true;clearInterval(keepalive);ws.close();error.signalReport=report;throw error; }
    return {report,
      async send(value) {
        if(failure)throw new Error(failure);
        const iv=crypto.getRandomValues(new Uint8Array(12));
        const aad=text.encode(config.run+'|'+config.role+'|'+config.other);
        const bytes=new Uint8Array(await crypto.subtle.encrypt({name:'AES-GCM',iv,additionalData:aad},key,text.encode(JSON.stringify(value))));
        const encoded=JSON.stringify({run:config.run,from:config.role,to:config.other,data:b64(join(iv,bytes))});
        if(config.carrier==='mqtt')ws.send(mqttPacket(0x30,join(str(theirTopic),text.encode(encoded))));
        else {const count=Math.ceil(encoded.length/280),id=config.role[0]+(++messageNumber).toString(36)+config.run.slice(-4);
          for(let i=0;i<count;i++){if(failure)throw new Error(failure);ws.send(`PRIVMSG ${room} :MIOY:${id}:${i}:${count}:${encoded.slice(i*280,(i+1)*280)}\r\n`);report.txFragments++;await delay(600);}}
        report.txMessages++;mark('sent',{kind:value.type,bytes:encoded.length});
      },
      receive(ms=60000){if(failure)return Promise.reject(new Error(failure));if(incoming.length)return Promise.resolve(incoming.shift());let w;return timeout(new Promise((resolve,reject)=>{w={resolve,reject};waiting.push(w);}),ms,'public signaling receive').finally(()=>{const i=waiting.indexOf(w);if(i>=0)waiting.splice(i,1);});},
      close(){closed=true;clearInterval(keepalive);if(ws.readyState===1){if(config.carrier==='mqtt')ws.send(Uint8Array.of(0xe0,0));else {ws.send(`PART ${room} :test complete\r\n`);ws.send('QUIT :test complete\r\n');}}ws.close();for(const w of waiting.splice(0))w.reject(new Error('signal closed'));report.closeRequested=true;}
    };
  }
  async function stats(pc) {
    const rows=[];(await pc.getStats()).forEach(v=>{if(['transport','candidate-pair','local-candidate','remote-candidate','data-channel'].includes(v.type))rows.push({...v});});
    const transport=rows.find(v=>v.type==='transport'&&v.selectedCandidatePairId);
    const pair=rows.find(v=>v.id===transport?.selectedCandidatePairId)??rows.find(v=>v.type==='candidate-pair'&&v.nominated&&v.state==='succeeded');
    return {rows,selectedPair:pair?{pair,local:rows.find(v=>v.id===pair.localCandidateId),remote:rows.find(v=>v.id===pair.remoteCandidateId)}:null};
  }
  async function peer(config, session, offerer) {
    const report={role:config.role,offerer,iceServers:[config.stun],turnServers:[],sentBytes:0,matchedBytes:0,events:[],passed:false};
    const pc=new RTCPeerConnection({iceServers:[{urls:config.stun}]});
    const dc=pc.createDataChannel('mio-quic-datagram/1',{id:0,negotiated:true,ordered:false,maxRetransmits:0,protocol:'miopunch/quic-datagram/1'});dc.binaryType='arraybuffer';
    const started=performance.now();const event=(kind,state)=>report.events.push({kind,state,ms:performance.now()-started});
    for(const kind of ['icegatheringstatechange','iceconnectionstatechange','connectionstatechange'])pc.addEventListener(kind,()=>event(kind,pc.iceGatheringState+'|'+pc.iceConnectionState+'|'+pc.connectionState));
    let receivedResolve,receivedReject,openedResolve,openedReject;
    const received=new Promise((resolve,reject)=>{receivedResolve=resolve;receivedReject=reject;}); received.catch(()=>{});
    const opened=new Promise((resolve,reject)=>{openedResolve=resolve;openedReject=reject;});opened.catch(()=>{});
    dc.onopen=()=>{event('datachannel','open');openedResolve();};
    dc.onmessage=event=>{const bytes=new Uint8Array(event.data);if(bytes.length===4&&utf.decode(bytes)==='DONE')return;const expected=offerer?0xa7:0x5c;if(bytes.length!==1200||!bytes.every(v=>v===expected)){receivedReject(new Error('RTC payload mismatch'));return;}report.matchedBytes=bytes.length;receivedResolve();};
    dc.onerror=()=>{openedReject(new Error('data channel error'));receivedReject(new Error('data channel error'));};
    async function gather() {if(pc.iceGatheringState==='complete')return;await timeout(new Promise(resolve=>pc.addEventListener('icegatheringstatechange',()=>{if(pc.iceGatheringState==='complete')resolve();})),12000,'ICE gathering');}
    try {
      if(offerer){await pc.setLocalDescription(await pc.createOffer());await gather();await session.send({type:'offer',description:pc.localDescription.toJSON()});const response=await session.receive();if(response.type!=='answer')throw new Error('answer expected');await pc.setRemoteDescription(response.description);}
      else {const request=await session.receive();if(request.type!=='offer')throw new Error('offer expected');await pc.setRemoteDescription(request.description);await pc.setLocalDescription(await pc.createAnswer());await gather();await session.send({type:'answer',description:pc.localDescription.toJSON()});}
      await timeout(opened,25000,'RTC open');
      await session.send({type:'rtc-open'});const ready=await session.receive();if(ready.type!=='rtc-open')throw new Error('peer RTC-open receipt expected');
      const payload=new Uint8Array(1200).fill(offerer?0x5c:0xa7);dc.send(payload);report.sentBytes=payload.length;
      await timeout(received,25000,'RTC bidirectional payload');
      report.channel={id:dc.id,label:dc.label,protocol:dc.protocol,ordered:dc.ordered,maxRetransmits:dc.maxRetransmits,negotiated:dc.negotiated};
      report.ice=pc.iceConnectionState;report.connection=pc.connectionState;report.stats=await stats(pc);report.durationMs=performance.now()-started;
      // Completion is still acknowledged through the PUBLIC signaling service.
      await session.send({type:'payload-ok',bytes:1200});const receipt=await session.receive();if(receipt.type!=='payload-ok'||receipt.bytes!==1200)throw new Error('peer payload receipt expected');
      report.passed=true;
    } catch(error){report.failure=String(error);report.ice=pc.iceConnectionState;report.connection=pc.connectionState;report.stats=await stats(pc).catch(()=>null);}
    finally{dc.close();pc.close();report.pcClosed=pc.connectionState==='closed';report.dataChannelClosed=dc.readyState==='closed'||dc.readyState==='closing';}
    return report;
  }
  globalThis.MioY700={signal,peer,timeout,delay};
  globalThis.startMioY700=async config=>{let session;const report={config:{carrier:config.carrier,url:config.url,run:config.run,role:config.role},scope:'physical RTC datagram interoperability only; no SDK admission/current-M/replica assertion',passed:false};try{session=await signal(config);report.signaling=session.report;report.rtc=await peer(config,session,config.role==='y700');report.passed=report.rtc.passed;}catch(error){report.failure=String(error);if(error.signalReport)report.signaling=error.signalReport;}finally{session?.close();report.endedUtc=new Date().toISOString();}return report;};
})();
