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
 for name in ['native-sdk-binding-final','wasm-sdk-binding-final']:
  build=TASK/name;r=json.loads((build/'receipt.json').read_text());c=json.loads((TASK/(name+'-context.json')).read_text())
  assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild']
  assert sdk.current_sources()==c['sources'];assert sdk.tool_snapshot()==c['tools']
  assert all(sha(p)==h for p,h in c['selectorPins'].items())
  assert all(sha(p)==h for p,h in c['artifactPins'].items())
  assert all(sha(p)==h for p,h in r['artifacts'].items())
  selected=json.loads((build/'selected-dependency-inputs.json').read_text());assert all(sha(p)==h for p,h in selected['inputs'].items())
  for f in [build/'receipt.json',build/'selected-dependency-inputs.json',TASK/(name+'-context.json')]:pins[str(f)]=sha(f)
  pins.update(r['artifacts'])
 for f in [HERE/'run.mjs',Path(__file__),ROOT/'docs/evidence/public-irc-restricted-2026-10-05/facility.json',Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome')]:pins[str(f)]=sha(f)
 return pins
before=guard();assert not(HERE/'runtime').exists()
(HERE/'execution-preconditions.json').write_text(json.dumps({'sourcesLocksArtifactsToolsControllersPassedBeforeActors':True,'pins':before,'publicTraffic':True,'facilities':'previously used explicit Ergo and permitted aps1-1 data relay','sourceDigest':sdk.json_digest(sdk.current_sources()),'profile':{'caller_ms':60000,'attempt_ms':60000,'pacing_ms':600,'assembly_ttl_ms':8000,'queue_ttl_ms':30000}},indent=2)+'\n')
start=time.monotonic();result=subprocess.run(['rtk','proxy','node',str(HERE/'run.mjs')],cwd=ROOT)
after=guard();raw=HERE/'runtime/private-runtime-receipt.json';r=json.loads(raw.read_text())
report={'runtimeExit':result.returncode,'qualified':r.get('qualified') is True and before==after and result.returncode==0,'sourceArtifactControllerUnchangedAfterRuntime':before==after,'wholeWallMs':(time.monotonic()-start)*1000,'rawReceiptSha256':sha(raw),'sourceDigest':sdk.json_digest(sdk.current_sources())}
(HERE/'execution.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(0 if report['qualified'] else 1)
