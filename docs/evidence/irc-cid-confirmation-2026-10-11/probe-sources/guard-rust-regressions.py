import hashlib,importlib.util,json,re,subprocess,sys,time
from pathlib import Path
p=Path(__file__).resolve().parent;c=json.loads((p/'context.json').read_text());r=json.loads((p/'tests/receipt.json').read_text())
spec=importlib.util.spec_from_file_location('sdk88',c['utility']);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
assert sdk.current_sources()==c['sources'] and sdk.tool_snapshot()==c['tools']
assert all(hashlib.sha256(Path(x).read_bytes()).hexdigest()==h for x,h in r['artifacts'].items())
out=p/'runtime';out.mkdir(mode=0o700);results=[]
for name in ['irc_datagram','unified_connection','unified_admission_story','negotiated_accept_context','member_path_protocol']:
 command=['rtk','proxy','timeout','45s',str(p/'tests/pinned'/name),'--nocapture']
 start=time.monotonic();v=subprocess.run(command,capture_output=True,text=True)
 (out/(name+'.stdout.log')).write_text(v.stdout);(out/(name+'.stderr.log')).write_text(v.stderr)
 found=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',v.stdout)
 result={'name':name,'command':command,'exit':v.returncode,'seconds':time.monotonic()-start,'passed':int(found[-1][0]) if found else None,'failed':int(found[-1][1]) if found else None,'ignored':int(found[-1][2]) if found else None}
 results.append(result);print(json.dumps(result),flush=True)
 if v.returncode:break
qualified=len(results)==5 and all(x['exit']==0 and x['failed']==0 for x in results) and sdk.current_sources()==c['sources']
report={'qualified':qualified,'sourceAndArtifactGuardsPassed':sdk.current_sources()==c['sources'],'results':results,'passedTests':sum(x['passed'] or 0 for x in results)}
(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(0 if qualified else 1)
