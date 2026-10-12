import hashlib,importlib.util,json,os,subprocess,sys,time
from pathlib import Path
os.umask(0o077)
ROOT=Path('/home/js/Git/miopunch');HERE=Path(__file__).parent;TASK=HERE.parent
UTILITY=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(UTILITY)=='2b7e969a1f90fc93eacc80d8b62890f0bb0ef8f40dc140d9398ef1a6029671e1'
spec=importlib.util.spec_from_file_location('socket_demo_sdk',UTILITY);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
def guard():
 assert Path.cwd().resolve()==ROOT
 pins={}
 for name in ['native-sdk-binding-final','wasm-sdk-binding-final','owned-relay-build']:
  build=TASK/name;r=json.loads((build/'receipt.json').read_text());c=json.loads((TASK/(name+'-context.json')).read_text())
  assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild']
  assert sdk.current_sources()==c['sources'];assert sdk.tool_snapshot()==c['tools']
  assert all(sha(p)==h for p,h in c['selectorPins'].items())
  assert all(sha(p)==h for p,h in c['artifactPins'].items())
  assert all(sha(p)==h for p,h in r['artifacts'].items())
  selected=json.loads((build/'selected-dependency-inputs.json').read_text());assert all(sha(p)==h for p,h in selected['inputs'].items())
  for f in [build/'receipt.json',build/'selected-dependency-inputs.json',TASK/(name+'-context.json')]:pins[str(f)]=sha(f)
  pins.update(r['artifacts'])
 for f in [HERE/'run.mjs',Path(__file__),ROOT/'tests/fixtures/irc/ergo.yaml',Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome')]:pins[str(f)]=sha(f)
 return pins

def cmd(*args,timeout=15):
 return subprocess.run(['rtk','proxy',*args],cwd=ROOT,capture_output=True,text=True,check=True,timeout=timeout).stdout.strip()
def ready_port(name,port):
 import socket
 address=cmd('docker','port',name,str(port)+'/tcp');host,n=address.rsplit(':',1);deadline=time.monotonic()+10
 while True:
  try:
   with socket.create_connection((host,int(n)),timeout=1):return int(n)
  except OSError:
   if time.monotonic()>deadline:raise
   time.sleep(.1)
before=guard();assert not(HERE/'runtime').exists();assert not(HERE/'private-fixture').exists()
(HERE/'execution-preconditions.json').write_text(json.dumps({'sourcesLocksArtifactsToolsControllersPassedBeforeActors':True,'pins':before,'publicTraffic':False,'facilities':'owned loopback Ergo TLS/WSS and HTTP data relay; fresh synthetic identities only','sourceDigest':sdk.json_digest(sdk.current_sources()),'profile':{'caller_ms':60000,'attempt_ms':60000,'pacing_ms':10,'assembly_ttl_ms':8000,'queue_ttl_ms':30000}},indent=2)+'\n')
sys.path.insert(0,str(ROOT/'scripts'));from browser_mqtt_trust import certificates,private_nss
F=HERE/'private-fixture';F.mkdir(mode=0o700);containers=[];relay=None;relayerr=None
report={'publicTraffic':False,'passed':False,'containers':[],'cleanup':{}};start=time.monotonic()
try:
 image='ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7'
 report['imageId']=json.loads(cmd('docker','image','inspect',image))[0]['Id']
 ca,leaf,key=certificates(F);private_nss(F/'nssdb',ca)
 config=(ROOT/'tests/fixtures/irc/ergo.yaml').read_text().replace('    :6667: {}','    :6697:\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key\n    :8097:\n      websocket: true\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key').replace('path: ircd.db','path: /tmp/irc-owned.db').replace('motd: ergo.motd','motd: /ircd-bin/ergo.motd')
 (F/'ergo.yaml').write_text(config);F.chmod(0o755)
 for f in [F/'ergo.yaml',leaf,key]:f.chmod(0o644)
 name='miopunch-m90-owned-irc-'+str(time.time_ns());containers.append(name)
 cmd('docker','run','--detach','--name',name,'--publish','127.0.0.1::6697','--publish','127.0.0.1::8097','--mount',f'type=bind,src={F},target=/fixture,readonly','--entrypoint','/ircd-bin/ergo',image,'run','--conf','/fixture/ergo.yaml')
 native,wss=ready_port(name,6697),ready_port(name,8097)
 (HERE/'local-facility.json').write_text(json.dumps({'logical_id':'m90-owned-irc','native_url':f'ircs://localhost:{native}/','wss_url':f'wss://localhost:{wss}/webirc','channel':'#m90-owned'}))
 relayerr=(HERE/'private-relay-stderr.log').open('w')
 relay=subprocess.Popen(['rtk','proxy',str(TASK/'owned-relay-build/pinned/owned_local_relay')],cwd=ROOT,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=relayerr,text=True)
 import select
 assert select.select([relay.stdout],[],[],15)[0],'owned relay ready timeout'
 ready=json.loads(relay.stdout.readline());assert ready['event']=='ready' and ready['url'].startswith('http://localhost:')
 (HERE/'local-relay.json').write_text(json.dumps(ready));report['relayStarted']=True
 assert guard()==before
 result=subprocess.run(['rtk','proxy','node',str(HERE/'run.mjs')],cwd=ROOT)
 raw=HERE/'runtime/private-runtime-receipt.json';r=json.loads(raw.read_text());report.update(runtimeExit=result.returncode,functionalAndSdkCleanupQualified=r.get('qualified') is True,rawReceiptSha256=sha(raw));report['passed']=result.returncode==0 and r.get('qualified') is True
except Exception as e:
 report['failure']={'class':type(e).__name__,'message':str(e)}
finally:
 if relay:
  try:
   stdout,_=relay.communicate('stop\n',timeout=15);report['cleanup']['relayExit']=relay.returncode;report['cleanup']['relayGraceful']=any(json.loads(line).get('graceful') is True for line in stdout.splitlines());report['cleanup']['relayProcessAbsent']=relay.poll() is not None
  except Exception as e:report['cleanup']['relayError']=str(e)
 if relayerr:relayerr.close()
 for name in containers:
  try:
   (HERE/(name+'.log')).write_text(cmd('docker','logs',name,timeout=3))
  except Exception:pass
  try:
   cmd('docker','rm','--force',name,timeout=5);absent=subprocess.run(['rtk','proxy','docker','inspect',name],capture_output=True).returncode!=0;report['containers'].append({'name':name,'removed':absent})
  except Exception as e:report['containers'].append({'name':name,'removed':False,'failure':str(e)})
 for f in F.glob('*.key'):f.chmod(0o600)
 F.chmod(0o700)
 after=guard();report.update(sourceArtifactControllerUnchangedAfterRuntime=before==after,wholeWallMs=(time.monotonic()-start)*1000,sourceDigest=sdk.json_digest(sdk.current_sources()))
 report['qualified']=report['passed'] and before==after and all(v['removed'] for v in report['containers']) and report['cleanup'].get('relayExit')==0 and report['cleanup'].get('relayGraceful') is True and report['cleanup'].get('relayProcessAbsent') is True
 (HERE/'execution.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
raise SystemExit(0 if report['qualified'] else 1)
