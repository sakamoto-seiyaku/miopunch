import hashlib,json,os,shutil,signal,socket,subprocess,sys,tempfile,time
from pathlib import Path
ROOT=Path('/home/js/Git/miopunch');sys.path.insert(0,str(ROOT/'scripts'))
from browser_mqtt_trust import certificates,private_nss
name=sys.argv[1] if len(sys.argv)>1 else 'disabled'
assert name in {'disabled','pc-denied','dc-denied','no-data-path'}
out=Path('/tmp/mio76-controlled-capability-'+name+'-'+os.environ.get('MIO76_ACCEPTANCE_ATTEMPT','first')+'-20261005');out.mkdir(mode=0o700,exist_ok=False)
build=Path('/tmp/mio76-ack-private-probe-build-20261005')
pkg=build/'pkg'
runner=Path('/tmp/mio76-controlled-capability-acceptance.mjs')
projection=Path('/tmp/mio76-ack-controlled-source-projection-20261005')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
report={'scope':'actual controlled host Browser/Native SDK with owned TLS/MQTT and injected runtime or resource condition only; no current-main, Android, physical, public or NAT qualification','case':name,'commands':[],'cleanup':[],'runnerSha256':sha(runner),'outerRunnerSha256':sha(Path(__file__)),'buildReceiptSha256':sha(build/'receipt.json'),'privateTrust':'temporary fixture CA + private NSS bwrap only, certificate verification enabled','immutableBaselineHealthyReceipt':'/tmp/mio76-controlled-browser-native-ack-warm-home-zero-control-20261005/receipt.json'}
dependencies=json.loads((build/'selected-dependencies-canonical.json').read_text())
report['compiledDependencies']={'path':str(build/'selected-dependencies-canonical.json'),'sha256':sha(build/'selected-dependencies-canonical.json'),'platforms':{}}
for platform in ('native','browser'):
 selected=dependencies[platform]['selectedProjectAndVendorInputs']
 assert all(sha(projection/p)==h for p,h in selected.items()),'frozen selected dependency changed'
 deltas={p:{'compiledSha256':h,'currentMainSha256':sha(ROOT/p) if (ROOT/p).is_file() else None} for p,h in selected.items() if not (ROOT/p).is_file() or sha(ROOT/p)!=h}
 report['compiledDependencies']['platforms'][platform]={'selectedCount':len(selected),'matchesFrozenProjection':True,'currentMainQualified':False,'currentMainDeltas':deltas,'compiledInputHashes':selected}
report['helperSources']={str(p):sha(p) for p in [ROOT/'scripts/browser_mqtt_trust.py',ROOT/'scripts/mosquitto-test.conf',ROOT/'browser-probe/harness/owned-mixed-mqtt-proxy.mjs',ROOT/'browser-probe/harness/rtc-capability-fixture.mjs']}
def run(cmd,timeout=15):
 start=time.monotonic();r=subprocess.run(cmd,cwd=ROOT,capture_output=True,text=True,timeout=timeout);report['commands'].append({'command':cmd,'exit':r.returncode,'seconds':time.monotonic()-start,'stdout':r.stdout,'stderr':r.stderr});r.check_returncode();return r.stdout.strip()
def hashes():
 return {str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [build/'browser_wss_member_native',build/'browser-probe-native',*pkg.rglob('*')] if p.is_file()}
report['artifactBefore']=hashes()
frozen=json.loads((build/'receipt.json').read_text());assert frozen['exit']==0 and frozen['sourceStable'] is True
assert json.loads((build/'sources-before.json').read_text())==json.loads((build/'sources-after.json').read_text())
assert all(sha(Path(p))==v['sha256'] for p,v in frozen['artifacts'].items()),'private artifact changed'
broker=None;process=None
fixture_path=None
try:
 with tempfile.TemporaryDirectory(prefix='mio76-controlled-ack-private-') as f:
  fixture=Path(f);fixture_path=fixture;ca,leaf,key=certificates(fixture);private_nss(fixture/'nssdb',ca)
  for source,destination in [(ca,'ca.der'),(leaf,'leaf.der')]:run(['rtk','proxy','openssl','x509','-in',str(source),'-outform','DER','-out',str(fixture/destination)])
  run(['rtk','proxy','openssl','pkcs8','-topk8','-nocrypt','-in',str(key),'-outform','DER','-out',str(fixture/'leaf-key.der')])
  broker='miopunch-mio76-capability-'+str(time.time_ns())
  run(['rtk','proxy','docker','run','--pull','never','--detach','--name',broker,'--publish','127.0.0.1::1883','--mount',f'type=bind,src={ROOT / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly','eclipse-mosquitto:2'])
  address=run(['rtk','proxy','docker','port',broker,'1883/tcp']);host,port=address.rsplit(':',1)
  deadline=time.monotonic()+5
  while True:
   try:
    with socket.create_connection((host,int(port)),timeout=.5):break
   except OSError:
    if time.monotonic()>=deadline:raise
    time.sleep(.05)
  cmd=['rtk','proxy','node',str(runner),'--fixture',str(fixture),'--output',str(out),'--package',str(pkg),'--broker',address,'--native',str(build/'browser_wss_member_native'),'--relay-native',str(build/'browser-probe-native')]
  start=time.monotonic()
  with (out/'run.log').open('w') as log:
   environment={**os.environ,'MIO76_CAPABILITY_CASE':name,'MIO76_REQUIRE_ACTUAL_HOME':'1','MIO76_RELAY_CHUNK_DELAY_MS':'0'}
   process=subprocess.Popen(cmd,cwd=ROOT,env=environment,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
   try:code=process.wait(timeout=65)
   except subprocess.TimeoutExpired:
    os.killpg(process.pid,signal.SIGTERM)
    try:code=process.wait(timeout=8)
    except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);code=process.wait();report['forcedOuterKill']=True
  report['commands'].append({'command':cmd,'exit':code,'seconds':time.monotonic()-start});report['runnerExit']=code
except Exception as e:report['failure']=type(e).__name__+': '+str(e)
finally:
 if broker:
  r=subprocess.run(['rtk','proxy','docker','rm','--force','--volumes',broker],cwd=ROOT,capture_output=True,text=True,timeout=15);report['cleanup'].append({'ownedBrokerRemovalExit':r.returncode})
 report['cleanup'].append({'ownedPrivateCertProfileDirectoryRemoved':fixture_path is not None and not fixture_path.exists(),'globalNssTrustChanged':False})
 report['artifactAfter']=hashes();report['artifactsStable']=report['artifactBefore']==report['artifactAfter']
 report['helpersStable']=all(sha(Path(p))==h for p,h in report['helperSources'].items())
 report['projectionDependenciesStillMatch']=all(sha(projection/p)==h for platform in ('native','browser') for p,h in dependencies[platform]['selectedProjectAndVendorInputs'].items())
 (out/'outer-receipt.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({'runnerExit':report.get('runnerExit'),'failure':report.get('failure'),'output':str(out),'artifactsStable':report['artifactsStable']}))
raise SystemExit(report.get('runnerExit',1))
