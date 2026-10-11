import hashlib,importlib.util,json,subprocess,sys,time
from pathlib import Path
p=Path(__file__).resolve().parent;c=json.loads((p/'context.json').read_text());r=json.loads((p/'tests/receipt.json').read_text())
spec=importlib.util.spec_from_file_location('sdk88',c['utility']);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
assert sdk.current_sources()==c['sources'] and sdk.tool_snapshot()==c['tools']
assert all(hashlib.sha256(Path(x).read_bytes()).hexdigest()==h for x,h in r['artifacts'].items())
out=p/'runtime';out.mkdir(mode=0o700);results=[]
for name in ['restricted_control_excludes_periodic_quality_pings','ordinary_data_retains_periodic_quality_pings']:
 command=['rtk','proxy','timeout','20s',str(p/'tests/pinned/unified_connection'),name,'--exact','--nocapture']
 start=time.monotonic();v=subprocess.run(command,capture_output=True,text=True)
 (out/(name+'.stdout.log')).write_text(v.stdout);(out/(name+'.stderr.log')).write_text(v.stderr)
 result={'name':name,'command':command,'exit':v.returncode,'seconds':time.monotonic()-start,'stdout':v.stdout}
 results.append(result);print(json.dumps(result),flush=True)
assert sdk.current_sources()==c['sources']
(out/'receipt.json').write_text(json.dumps({'results':results,'sourceAndArtifactGuardsPassed':True},indent=2)+'\n')
