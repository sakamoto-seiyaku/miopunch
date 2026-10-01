import sys,subprocess,pathlib,json,datetime,hashlib
p=pathlib.Path("docs/evidence/mio36-recovery"); p.mkdir(exist_ok=True)
name=sys.argv[1]; files=["Cargo.toml","Cargo.lock","tests/management_recovery.rs","src/management_recovery.rs","src/native_network.rs","src/genesis_recovery.rs","src/management_store.rs","src/admission_gate.rs","src/lib.rs"]
sha={f:hashlib.sha256(pathlib.Path(f).read_bytes()).hexdigest() for f in files}
sha["/tmp/mio36-short.py"]=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
assert not (p/(name+"-exit.json")).exists()
(p/(name+"-source.json")).write_text(json.dumps(sha,indent=2))
cmd=["rtk","proxy",*sys.argv[2:]]; start=datetime.datetime.now(datetime.timezone.utc).isoformat()
r=subprocess.run(cmd,stdout=open(p/(name+".log"),"w"),stderr=subprocess.STDOUT)
(p/(name+"-exit.json")).write_text(json.dumps({"argv":cmd,"started_utc":start,"finished_utc":datetime.datetime.now(datetime.timezone.utc).isoformat(),"exit_code":r.returncode,"source_unchanged":all(hashlib.sha256(pathlib.Path(f).read_bytes()).hexdigest()==s for f,s in sha.items())},indent=2))
print("actual exit",r.returncode); print((p/(name+".log")).read_text()[-5500:]); sys.exit(r.returncode)
