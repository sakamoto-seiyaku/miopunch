#!/usr/bin/env python3
"""Independent strict full-phase/identity/TTL/pcap checker for MIOPUNCH-63."""
import argparse, ipaddress, json, pathlib, re, struct
from legacy_nat_lab import replace_once
from legacy_nat_verify import require, SAMPLE, SCORE
from legacy_nat_scheduler_matrix import cells, name
PUBLIC={"a":"100.64.27.2","b":"100.64.27.6"}
PRIVATE={"a":"10.27.1.2","b":"10.27.2.2"}
BEHAVIOR=re.compile(r"Behavior \{ role: (Sender|Receiver), ttl: (\d+), send_delay_ms: (\d+), ports_range: (\d+), ports_random: (\d+), listen_random: (\d+) \}")
DATAGRAM=re.compile(r"^LEGACY_DATAGRAM target=(\S+) ttl=(\d+) response=(true|false) degraded=(true|false)$",re.M)


def capture(path,log):
    counts=[re.search(rf"^(\d+) packets {suffix}$",log,re.M) for suffix in ("captured","received by filter","dropped by kernel")]
    require(all(counts),"missing capture shutdown counters: "+str(path))
    captured,received,dropped=[int(m.group(1)) for m in counts]
    require(captured>0 and captured==received and dropped==0,"incomplete/dropped capture: "+str(path))
    events=[];frames=0
    with path.open("rb") as f:
        head=f.read(24);require(len(head)==24 and head[:4]==bytes.fromhex("d4c3b2a1"),"invalid capture header")
        kind=struct.unpack("<I",head[20:24])[0];require(kind in (1,276),"wrong pcap link type")
        while h:=f.read(16):
            require(len(h)==16,"truncated capture record")
            sec,usec,size,original=struct.unpack("<IIII",h);frame=f.read(size);frames+=1
            require(len(frame)==size and size==original,"truncated captured packet")
            offset=20 if kind==276 else 14
            if kind==276:
                if frame[:2]!=b"\x08\x00":continue
                packet_type=frame[10]
            else:
                if frame[12:14]!=b"\x08\x00":continue
                packet_type=None
            ip=frame[offset:]
            if len(ip)<28 or ip[0]>>4!=4 or ip[9]!=17:continue
            ihl=(ip[0]&15)*4;total=struct.unpack("!H",ip[2:4])[0]
            require(len(ip)>=total,"truncated IPv4 datagram")
            sport,dport,length=struct.unpack("!HHH",ip[ihl:ihl+6]);payload=ip[ihl+8:ihl+length]
            require(len(payload)==length-8,"truncated UDP datagram")
            events.append(dict(time=sec+usec/1000000,packet_type=packet_type,src=str(ipaddress.IPv4Address(ip[12:16])),dst=str(ipaddress.IPv4Address(ip[16:20])),sport=sport,dport=dport,ttl=ip[8],payload=payload))
    require(frames==captured,"pcap record count differs from capture shutdown")
    return events,frames


def raw(e):return e["payload"].startswith(b"\0MIOPUNCH63")
def target(e):return f"{e['dst']}:{e['dport']}"
def unique_adjacent(items):return [x for i,x in enumerate(items) if not i or x!=items[i-1]]
def feature(samples):
    ips=[x.rsplit(":",1)[0] for x in samples];ports=[int(x.rsplit(":",1)[1]) for x in samples]
    hard=len(set(samples))>1;difference=max(ports)-min(ports) if len(set(ips))==1 else 0
    return hard,0<difference<=5,difference

def expected_plan(samples,history,oracle):
    c=feature(samples["server"]);v=feature(samples["client"]);hard=c[0]+v[0];regular=c[1]+v[1]
    modes=[0] if hard==0 else ([1,2,0] if regular==1 else [2,1,0]) if hard==1 else [3,4] if regular==2 else [4] if regular==1 else [0,1,3]
    scores=[(row["Mode"],row["Index"],1 if hard==2 and regular==0 else 0) for mode in modes for row in oracle if row["Mode"]==mode]
    for _ in range(history):
        pos=max(range(len(scores)),key=lambda i:scores[i][2]);m,i,s=scores[pos];scores[pos]=(m,i,s-1)
    m,i,_=max(scores,key=lambda x:x[2]);row=next(row for row in oracle if (row["Mode"],row["Index"])==(m,i))
    cb,vb=row["C"],row["V"]
    if m==1 and not c[0] or m==2 and c[0] or m==4 and not c[1]:cb,vb=vb,cb
    return (m,i),scores,{"server":cb,"client":vb}


def behavior_tuple(b):return (b["Role"].capitalize(),str(b["TTL"]),str(b["SendDelayMs"]),str(b["PortsRangeNumber"]),str(b["PortsRandomNumber"]),str(b["ListenRandomPorts"]))

def verify(directory,cell,oracle):
    run=dict(x.split("=",1) for x in (directory/"run.txt").read_text().split())
    require(run==dict(case=cell["case"],strategy="legacy",topology=cell["topology"],dialer=cell["dialer"],client_rc="1" if cell["case"] in ("revoked","no_secret","wrong_secret") else "0",server_rc="1" if cell["case"] in ("revoked","no_secret","wrong_secret") else "0"),"unexpected run/exit provenance")
    require(json.loads((directory/"cell.json").read_text())==cell,"cell metadata mismatch")
    fixture=(directory/"fixture.sh").read_text();require("-t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP" in fixture,"missing pre-conntrack low TTL loss")
    require(any(f in fixture for f in ("udp and (host 100.64.27.2 or host 100.64.27.6)","udp and (host 100.64.27.2 or host 100.64.27.6 or host 100.64.27.10)")),"incomplete WAN peer filter")
    wan,nwan=capture(directory/"wan.pcap",(directory/"tcpdump.log").read_text())
    lan={};counts={"wan":nwan}
    for node in "ab":lan[node],counts[node]=capture(directory/f"lan-{node}.pcap",(directory/f"tcpdump-lan-{node}.log").read_text())
    logs={s:(directory/f"{s}.log").read_text() for s in ("server","client")}
    harness=(directory/"harness.log").read_text()
    require(not re.search(r"harness failure|Traceback|Operation not permitted|No such file or directory",harness,re.I),"harness error")
    for side,log in logs.items():
        errors=re.findall(r"^Error:.*$",log,re.M)
        allowed=[]
        if cell["case"] in ("revoked","no_secret","wrong_secret"):
            allowed=["Error: Connection was rejected locally" if side=="client" else "Error: Custom { kind: Other, error: Connection was rejected locally }"]
        require(errors==allowed and not re.search(r"Traceback|thread .*panicked|^error:",log,re.M),"unexpected runtime error is not an accepted outcome")
    for node in "ab":
        for suffix in ["wrong","wrong-ip","right"]:
            f=directory/f"filter-{node}-{suffix}.txt";require(f.exists(),"missing independent filter proof")
            content=f.read_text()
            if suffix!="right":require(re.search(r"policy DROP [1-9][0-9]* packets",content),"fixture failed wrong source filter")
            else:require(re.search(r"^\s*[1-9][0-9]*\s+\d+\s+ACCEPT.*wan0\s+lan0",content,re.M),"fixture failed matched reply filter")
    row={"cell":name(cell),"captured_packets":counts,"outcome":"admission_rejection","peer_probes":0,"peer_responses":0}
    peers={s:re.search(r"^CANDIDATE id=([0-9a-f]{64})",log,re.M).group(1) for s,log in logs.items()}
    negative=cell["case"] in ("nonmember","revoked","no_secret","wrong_secret")
    if negative:
        require(not any(raw(e) for e in wan),"admission negative spent traversal traffic")
        for side,log in logs.items():
            require(not any(marker in log for marker in ("LEGACY_SAMPLE","LEGACY_SELECTED","LEGACY_DATAGRAM","DATA_OK","learned=true")),"admission work before authorization")
            if cell["case"]=="nonmember":require(re.search(rf"^ADMISSION_REJECTED peer={peers['client' if side=='server' else 'server']} stable_id=\d+ probes=0$",log,re.M),"wrong/missing admission rejection identity")
            else:
                error="Error: Connection was rejected locally" if side=="client" else "Error: Custom { kind: Other, error: Connection was rejected locally }"
                require(error in log.splitlines(),"unrelated startup error is not admission rejection")
        return row
    samples={s:[m[2] for m in SAMPLE.findall(log)] for s,log in logs.items()}
    fallback=cell["topology"]=="fallback"
    if fallback:
        require(not any(samples.values()),"fallback silently acquired classification evidence")
        plan=(0,6);scores=[];behavior={s:{"Role":"sender" if s=="server" else "receiver","TTL":0,"SendDelayMs":0,"PortsRangeNumber":0,"PortsRandomNumber":0,"ListenRandomPorts":0} for s in logs}
    else:
        for side,log in logs.items():
            ss=SAMPLE.findall(log);require([int(x[0]) for x in ss]==[0,1,2,3],"missing ordered S evidence")
            for _,dst,src in ss:
                sip,sport=src.rsplit(":",1);dip,dport=dst.rsplit(":",1)
                require(any(e["src"]==sip and e["dst"]==dip and e["sport"]==int(sport) and e["dport"]==int(dport) for e in wan),"S outbound tuple missing")
                require(any(e["src"]==dip and e["dst"]==sip and e["sport"]==int(dport) and e["dport"]==int(sport) for e in wan),"S observer reply missing")
        plan,scores,behavior=expected_plan(samples,cell["history"],oracle)
    require(plan==(cell["mode"],cell["index"]),"cell did not select expected old mode/index")
    rounds=[];stable={};phase_budget=max(b["SendDelayMs"] for b in behavior.values())+5000
    all_raw=[e for e in wan if e["packet_type"]==0 and raw(e)]
    earliest=min(e["time"] for e in all_raw)
    successful=[]
    for side,log in logs.items():
        node=cell["dialer"] if side=="client" else ("b" if cell["dialer"]=="a" else "a")
        other="b" if node=="a" else "a"
        selected=next(x for x in log.splitlines() if x.startswith("LEGACY_SELECTED"))
        require(f"mode={plan[0]} index={plan[1]} " in selected,"wrong plan")
        require(SCORE.findall(selected)==[(str(m),str(i),str(s)) for m,i,s in scores],"protected score/tie history mismatch")
        require(BEHAVIOR.findall(selected)==[behavior_tuple(behavior["server"]),behavior_tuple(behavior["client"])],"legacy tuple/role mismatch")
        rounds.append(re.search(r" round=([0-9a-f]{32}) ",selected).group(1))
        ids=set(re.findall(r"stable_id=(\d+)",log));require(len(ids)==1,"connection stable identity changed");stable[side]=next(iter(ids))
        receive=next(x for x in log.splitlines() if x.startswith("LEGACY_RECV_START"))
        require(log.index(receive)<log.index("LEGACY_DATAGRAM"),"receive registration came after sending")
        b=behavior[side];peer_side="client" if side=="server" else "server"
        require(f"total_budget_ms: {phase_budget}, read_budget_ms: {phase_budget-b['SendDelayMs']}" in receive,"wrong legacy total/read budget")
        local=[e for e in lan[node] if raw(e) and e["src"]==PRIVATE[node]]
        listed=DATAGRAM.findall(log);require(len(local)==len(listed)>0,"LAN capture/log application packet count mismatch")
        for e,(dst,ttl,response,degraded) in zip(local,listed):
            require(len(e["payload"])==76 and e["payload"][11:27].hex()==rounds[-1],"wrong raw round/framing")
            require(target(e)==dst and (e["payload"][27]==1)==(response=="true"),"target/role differs from actual packet")
            require(e["sport"]==(42027 if node=="a" else 43027),"probe left original physical UDP socket")
            expected_ttl=64 if int(ttl)==0 or degraded=="true" else int(ttl)
            require(e["ttl"]==expected_ttl,"requested per-packet TTL not executed")
            require(degraded==("true" if cell["variant"]=="ttl_unavailable" and int(ttl)>0 else "false"),"TTL downgrade reporting mismatch")
        requests=[e for e in local if e["payload"][27]==0];responses=[e for e in local if e["payload"][27]==1]
        row["peer_probes"]+=len(requests);row["peer_responses"]+=len(responses)
        require(all(e["ttl"]==64 for e in responses),"responses did not use default TTL")
        if responses:
            require(b["Role"]=="receiver" and len(responses)==3,"response role/burst mismatch")
            require(all(.035<=responses[i+1]["time"]-responses[i]["time"]<=.14 for i in range(2)),"response burst is not paced at 50ms")
        if fallback: explicit=[re.search(r"observed=(\S+)",logs[peer_side]).group(1)]
        else:explicit=unique_adjacent(samples[peer_side]) if b["Role"]=="sender" or b["PortsRangeNumber"]==0 else []
        range_targets=[]
        if b["PortsRangeNumber"]:
            last=int(samples[peer_side][-1].rsplit(":",1)[1]);d=feature(samples[peer_side])[2];radius=b["PortsRangeNumber"]
            low=max(last-d-5,last-radius,1);high=min(last+d+5,last+radius,65535)
            ips=unique_adjacent([x.rsplit(":",1)[0] for x in samples[peer_side]])
            range_targets=[f"{ip}:{port}" for ip in ips for port in range(low,high+1)]
        expected_first=explicit+range_targets;actual=[target(e) for e in requests]
        require(actual[:len(expected_first)]==expected_first,"first burst explicit/range order/count mismatch")
        require(len(actual)>=len(expected_first),"first range burst truncated")
        if range_targets:
            burst=requests[len(explicit):len(expected_first)]
            require(all(.0015<=burst[i+1]["time"]-burst[i]["time"]<=.06 for i in range(len(burst)-1)),"range not paced at 2ms")
        remainder=actual[len(expected_first):]
        require((not remainder if not explicit else len(remainder)%len(explicit)==0 and remainder==explicit*(len(remainder)//len(explicit))),"range repeated or explicit order changed")
        if explicit and len(remainder)>=len(explicit):
            repeat_times=[requests[j]["time"] for j in range(len(expected_first),len(requests),len(explicit))]
            require(all(.17<=repeat_times[i+1]-repeat_times[i]<=.3 for i in range(len(repeat_times)-1)),"explicit repeat not paced at 200ms")
        if b["SendDelayMs"]:
            require(requests[0]["time"]-earliest>=b["SendDelayMs"]/1000-.08,"sender sent before its own delay")
        # A low-TTL request creates local mapping state but never reaches peer LAN.
        low=[e for e in requests if e["ttl"]<8]
        other_in=[e for e in lan[other] if raw(e) and e["dst"]==PRIVATE[other]]
        require(not any(e["payload"]==q["payload"] for q in low for e in other_in),"low TTL probe arrived beyond raw pre-conntrack drop")
        if low and cell["case"]!="blocked":
            content=(directory/f"nat-{other}-raw.txt").read_text();require(re.search(r"^\s*[1-9][0-9]*\s+\d+\s+DROP.*TTL",content,re.M),"raw TTL drop counter did not increment")
        if cell["case"]=="blocked":
            content=(directory/"wan-forward.txt").read_text()
            require(re.search(rf"^\s*[1-9][0-9]*\s+\d+\s+DROP.*{re.escape(PUBLIC[node])}\s+{re.escape(PUBLIC[other])}",content,re.M),"WAN block did not execute in this direction")
            require(not any(e["payload"]==q["payload"] for q in requests for e in other_in),"blocked WAN delivered a traversal request")
        # Independent WAN ingress must contain every local traversal payload.
        outgoing=[e for e in all_raw if e["src"]==PUBLIC[node]]
        require(len(outgoing)==len(local) and [e["payload"] for e in outgoing]==[e["payload"] for e in local],"WAN/LAN traversal coverage mismatch")
        for e in outgoing:
            require(e["ttl"]==next(q["ttl"] for q in local if q["payload"]==e["payload"])-1,"unexpected WAN TTL/hop result")
        # QUIC is never emitted with a lowered traversal TTL on either connection.
        quic=[e for e in lan[node] if e["src"]==PRIVATE[node] and e["sport"]==(42027 if node=="a" else 43027) and e["payload"] and e["payload"][0]&0x40]
        require(quic and all(e["ttl"]==64 for e in quic),"shared socket TTL contaminated QUIC/QAD traffic")
        stop=re.search(r"LEGACY_PHASE_STOP round=([0-9a-f]+) stable_id=(\d+) elapsed_ms=(\d+) outcome=(winner|failed)",log)
        require(stop and stop[1]==rounds[-1] and stop[2]==stable[side] and int(stop[3])<=phase_budget+250,"missing/unbounded phase cleanup result")
        success="DATA_OK" in log;successful.append(success)
        if success:
            winner=re.search(r"LEGACY_WINNER round=\S+ stable_id=\d+ remote=(\S+) response=(true|false)",log);require(winner,"missing actual provisional winner")
            ip,port=winner[1].rsplit(":",1);require(any(e["src"]==ip and e["sport"]==int(port) and e["payload"][11:27].hex()==rounds[-1] for e in lan[node] if raw(e) and e["dst"]==PRIVATE[node]),"winner has no incoming authenticated-round tuple")
            data=next(x for x in log.splitlines() if x.startswith("DATA_OK"))
            require(re.findall(r"\bpeer=(\S+)",data)==[peers[peer_side]],"business peer differs from protected pair")
            paths=dict(re.findall(r"PathData\((\d+), ip:([^),\]\s]+)\)",data))
            selected_path=re.search(r"selected: Some\(PathId\((\d+)\)\)",data)
            require(selected_path and paths.get(selected_path[1])==winner[1],"selected business path differs from actual winner")
            require("LEGACY_EXECUTION status=CompleteExecution" in log,"partial executor claimed full mode")
            if side=="server":
                expected="outcome=confirmed_business learned=false score_delta=0" if fallback else "outcome=confirmed_business learned=true score_delta=2"
                require(expected in log,"authorized business feedback mismatch")
        else:
            require("PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed" in log,"arbitrary failure is not completed bounded miss")
            require(stop[4]=="failed" and int(stop[3])>=phase_budget-50,"miss stopped before complete budget")
            require("CONTROL_ALIVE_AFTER_UDP_FAILURE" in log and "CONTROL_LIVENESS_FAILED" not in log and "learned=true" not in log,"miss lost original protected control or falsely learned")
    require(len(set(rounds))==1,"protected peer round mismatch")
    require(successful[0]==successful[1],"only one side claims business confirmation")
    if cell["case"]=="positive" and cell["topology"] in ("eim","fallback"):require(all(successful),"core easy scenario failed")
    if cell["case"]=="blocked":require(not any(successful),"WAN-blocked scenario gained business")
    row.update(mode=plan[0],index=plan[1],outcome="confirmed_business" if all(successful) else "bounded_miss",stable=stable,round=rounds[0],phase_budget_ms=phase_budget)
    return row


def main():
    p=argparse.ArgumentParser();p.add_argument("evidence",type=pathlib.Path);a=p.parse_args();root=pathlib.Path(__file__).resolve().parents[1]
    expected=cells();manifest=json.loads((a.evidence/"verification/cells.json").read_text());require(manifest==expected,"incomplete or changed cell matrix")
    directories={x.name for x in (a.evidence/"canonical").iterdir() if x.is_dir()};require(directories=={name(c) for c in expected},"missing/extra canonical cells")
    runners=json.loads((a.evidence/"verification/runner.json").read_text());require([r["cell"] for r in runners]==[name(c) for c in expected] and all(r["exit"]==0 for r in runners),"matrix runner omitted or failed cells")
    oracle=json.loads((root/"tests/fixtures/legacy_nat_behaviors.json").read_text());rows=[verify(a.evidence/"canonical"/name(c),c,oracle) for c in expected]
    print(json.dumps(dict(result="PASS",gate_cases=len(rows),connection_success=sum(r["outcome"]=="confirmed_business" for r in rows),bounded_misses=sum(r["outcome"]=="bounded_miss" for r in rows),admission_rejections=sum(r["outcome"]=="admission_rejection" for r in rows),rows=rows),indent=2))
if __name__=="__main__":main()
