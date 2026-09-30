#!/usr/bin/env python3
"""Frozen cell matrix: complete old indices in both dialing directions."""
import argparse, hashlib, json, os, pathlib, subprocess, time

def cells():
    result=[]
    for topology,mode,count in [("eim",0,12),("up1",1,6),("up1-up1",3,6)]:
        for dialer in "ab":
            for index in range(count):
                result.append(dict(case="positive",topology=topology,dialer=dialer,history=index,mode=mode,index=index,variant="normal"))
    for dialer in "ab":
        for history,mode in [(0,0),(12,1),(18,3)]:
            result.append(dict(case="positive",topology="up2-up2",dialer=dialer,history=history,mode=mode,index=0,variant="normal"))
        for topology in ["down1","nonmonotone","spread5","lowedge","highedge"]:
            result.append(dict(case="positive",topology=topology,dialer=dialer,history=0,mode=1,index=0,variant="normal"))
        result.append(dict(case="positive",topology="fallback",dialer=dialer,history=0,mode=0,index=6,variant="normal"))
        result.append(dict(case="blocked",topology="up1-up1",dialer=dialer,history=0,mode=3,index=0,variant="normal"))
        result.append(dict(case="positive",topology="eim",dialer=dialer,history=0,mode=0,index=0,variant="ttl_unavailable"))
    for case in ["nonmember","revoked","no_secret","wrong_secret"]:
        result.append(dict(case=case,topology="eim",dialer="a",history=0,mode=None,index=None,variant="normal"))
    return result

def name(c):return f"{c['case']}-{c['topology']}-{c['dialer']}-h{c['history']}-{c['variant']}"

def main():
    p=argparse.ArgumentParser();p.add_argument("runtime",type=pathlib.Path);p.add_argument("evidence",type=pathlib.Path);a=p.parse_args()
    if os.geteuid()!=0:p.error("run inside disposable privileged lab")
    repo=pathlib.Path(__file__).resolve().parents[1]
    source=json.loads((a.evidence/"verification/source-sha256.json").read_text())
    runtime=json.loads((a.runtime/"sha256.json").read_text())
    def unchanged():
        for file,digest in source.items():
            if hashlib.sha256((repo/file).read_bytes()).hexdigest()!=digest:raise RuntimeError("runtime/driver changed during matrix: "+file)
        for file,digest in runtime.items():
            if hashlib.sha256((a.runtime/file).read_bytes()).hexdigest()!=digest:raise RuntimeError("frozen binary changed: "+file)
    matrix=cells();(a.evidence/"verification/cells.json").write_text(json.dumps(matrix,indent=2))
    output=a.evidence/"canonical";output.mkdir(exist_ok=True)
    rows=[]
    for i,c in enumerate(matrix):
        unchanged();target=output/name(c);target.mkdir(exist_ok=True)
        env=dict(os.environ);env["MIOPUNCH_LEGACY_TTL_UNAVAILABLE"]="1" if c["variant"]=="ttl_unavailable" else "0"
        started=time.monotonic();r=subprocess.run(["python3","-B",str(repo/"scripts/legacy_nat_scheduler_lab.py"),c["case"],c["topology"],c["dialer"],str(c["history"]),str(a.runtime/"udp_nat_probe"),str(a.runtime/"udp_observation_probe"),str(target)],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
        target.joinpath("harness.log").write_text(r.stdout);target.joinpath("cell.json").write_text(json.dumps(c,indent=2))
        rows.append(dict(cell=name(c),exit=r.returncode,elapsed_ms=round((time.monotonic()-started)*1000)))
        (a.evidence/"verification/runner.json").write_text(json.dumps(rows,indent=2))
        print(f"CELL {i+1}/{len(matrix)} {name(c)} exit={r.returncode}",flush=True)
        if r.returncode:raise RuntimeError("fixture/harness failure: "+name(c))
        if c["mode"] is not None:
            for side in ["client","server"]:
                log=target.joinpath(side+".log").read_text()
                if f"LEGACY_SELECTED mode={c['mode']} index={c['index']} " not in log:raise RuntimeError("expected cell plan not actually selected: "+name(c))
            if c["topology"]!="fallback" and f"LEGACY_TEST_HISTORY previous_selections={c['history']}" not in target.joinpath("server.log").read_text():raise RuntimeError("lab history hook unavailable")
    unchanged()
if __name__=="__main__":main()
