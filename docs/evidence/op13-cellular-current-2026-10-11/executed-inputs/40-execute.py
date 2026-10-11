import json,os,sys,time,uuid,hashlib,importlib.util,subprocess,re
from pathlib import Path
os.umask(0o077);sys.dont_write_bytecode=True
ROOT=Path('/home/js/Git/miopunch');HERE=Path(__file__).resolve().parent;P=HERE.parent
sys.path.insert(0,str(ROOT/'scripts'))
import android_pair_runtime_current as runtime
from android_pair_route_receipt_current import _agent_facts
context=json.loads((P/'build-context.json').read_text());utility=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(utility)==context['utilitySha256'];spec=importlib.util.spec_from_file_location('irc_physical_sdk_guard',utility);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
W=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm');N=P/'irc-android'
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def guard():
 assert sdk.current_sources()==context['sources'] and sdk.tool_snapshot()==context['tools']
 reports={}
 for out in (N,W):
  r=json.loads((out/'receipt.json').read_text());assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild'];assert all(sha(p)==h for p,h in r['artifacts'].items());g=json.loads((out/'selected-dependency-inputs.json').read_text());sdk.verify_captured_graph(g,out,context['sources'])
  reports[str(out)]={'receiptSha256':sha(out/'receipt.json'),'selectedGraphSha256':sha(out/'selected-dependency-inputs.json'),'actualInputCount':len(g['inputs']),'copiedRepositoryInputCount':len(g['copiedRepositoryInputs']),'capturedDepfileCount':len(g['depfiles'])}
 files=[HERE/'run.mjs',HERE/'android-native-bridge.py',Path(__file__),P/'build-context.json',P/'build-irc-android.py',P/'irc-build-context.json',P/'extraSelector/Cargo.toml',P/'extraSelector/Cargo.lock',utility,ROOT/'docs/evidence/public-irc-restricted-2026-10-05/facility.json',*sorted((ROOT/'scripts').glob('*.py'))]
 reports['files']={str(p):sha(p) for p in files};reports['sourceDigest']=context['sourceDigest'];return reports
def facts():
 result={}
 for role in ('native','browser'):
  d=runtime.device_for(role);value,code=d.shell('dumpsys','connectivity');assert code==0;q=_agent_facts(value);assert q['activeDefaultObserved'] and q['defaultAgentObserved'] and q['defaultNotVpnCapability'] and q['currentVpnAgentCount']==0 and q['defaultTransport']==('CELLULAR' if role=='native' else 'WIFI')
  result[role]={k:v for k,v in q.items() if k not in ('interfaces','selectedAgent')}
 return result
before=guard();action=sys.argv[1]
if action=='prepare':
 assert not (HERE/'binding.json').exists();write(HERE/'binding.json',before);print(json.dumps({'prepared':True,'actorsStarted':0}));raise SystemExit(0)
assert before==json.loads((HERE/'binding.json').read_text()) and not (HERE/'runtime').exists()
write(HERE/'network-before.json',facts());browser=runtime.device_for('browser');native=runtime.device_for('native');forward=None;code=1;cleanup={};started=time.monotonic();store='/data/local/tmp/miopunch-irc-'+uuid.uuid4().hex
try:
 _,absent=native.shell('test','!','-e',store,required=False);assert absent==0;cleanup['ownedRemoteStoreAbsentBeforeNativeCreate']=True
 port,_=browser.run('forward','tcp:0','localabstract:chrome_devtools_remote');assert port.strip().isdigit();forward='tcp:'+port.strip();cdp='http://127.0.0.1:'+port.strip()
 write(HERE/'runtime-private-bindings.json',{'cdp':cdp,'remoteStore':store,'storeClaimedAbsentBeforeCreate':True,'forward':forward})
 with (HERE/'runtime.stdout.log').open('w') as stdout,(HERE/'runtime.stderr.log').open('w') as stderr:
  p=subprocess.Popen(['rtk','proxy','node',str(HERE/'run.mjs')],cwd=ROOT,stdout=stdout,stderr=stderr)
  position=0
  while p.poll() is None:
   time.sleep(1)
   with (HERE/'runtime.stdout.log').open() as f:f.seek(position);new=f.read();position=f.tell()
   if new:print(new,end='',flush=True)
  code=p.wait()
 with (HERE/'runtime.stdout.log').open() as f:f.seek(position);new=f.read()
 if new:print(new,end='',flush=True)
finally:
 if forward:
  _,code_forward=browser.run('forward','--remove',forward,required=False);listing,listcode=browser.run('forward','--list',required=False);cleanup['ownedForwardRemoved']=code_forward==0 and listcode==0 and not any(forward in line.split() for line in listing.splitlines())
 bridge_rows=[]
 for path in sorted(HERE.glob('private-native-bridge-*.json')):
  value=json.loads(path.read_text());bridge_rows.append({'phase':path.stem,'receiptSha256':sha(path),'cleanup':value['cleanup'],'failure':value.get('failure')})
 cleanup['nativeBridgePhases']=bridge_rows
 cleanup['allOwnedRemoteActorsAbsent']=bool(bridge_rows) and all(row['cleanup'].get('ownedRemoteProcessAbsentLate') is True and row['cleanup'].get('forced') is False for row in bridge_rows)
 if cleanup['allOwnedRemoteActorsAbsent']:
  assert re.fullmatch(r'/data/local/tmp/miopunch-irc-[0-9a-f]{32}',store);_,removed=native.shell('rm','-rf','--',store,required=False);_,absent=native.shell('test','!','-e',store,required=False);cleanup['ownedRemoteStoreRemoved']=removed==0 and absent==0
 stable=guard()==before;network=facts();write(HERE/'network-after.json',network);raw_path=HERE/'runtime/private-runtime-receipt.json';raw=json.loads(raw_path.read_text()) if raw_path.exists() else {}
 qualified=code==0 and stable and cleanup.get('ownedForwardRemoved') is True and cleanup.get('allOwnedRemoteActorsAbsent') is True and cleanup.get('ownedRemoteStoreRemoved') is True
 report={'runtimeExit':code,'sourceAndArtifactGuardsPassed':stable,'wholeWallMs':round((time.monotonic()-started)*1000,3),'passed':qualified,'functionalPassed':raw.get('passed') is True,'normalPublicHttpsOrigin':raw.get('actualPageOrigin')=='https://testnet.ergo.chat','physicalDefaultNetworksNoVpnBeforeAfter':True,'physicalWanQualified':False,'requestElapsedMs':raw.get('request',{}).get('elapsedMs'),'memberElapsedMs':raw.get('memberOpen',{}).get('elapsedMs'),'rawReceiptSha256':sha(raw_path) if raw_path.exists() else None,'cleanup':cleanup,'firstFailure':raw.get('firstFailure')}
 write(HERE/'execution.json',report);print(json.dumps({k:v for k,v in report.items() if k not in ('cleanup','firstFailure')}) ,flush=True)
raise SystemExit(0 if qualified else 1)
