import argparse,hashlib,importlib.util,json,shutil,subprocess,sys,time,tomllib
from pathlib import Path

ROOT=Path('/home/js/Git/miopunch');HERE=Path('/tmp/miopunch-native-socket-api-20261012')
UTILITY=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(UTILITY)=='2b7e969a1f90fc93eacc80d8b62890f0bb0ef8f40dc140d9398ef1a6029671e1'
spec=importlib.util.spec_from_file_location('socket_sdk_capture',UTILITY)
sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
cache=sdk.cargo_cache

def prepare(name,command):
 p=HERE/(name+'-context.json');assert not p.exists()
 selectors={}
 if '--manifest-path' in command:
  manifest=Path(command[command.index('--manifest-path')+1]);selectors[str(manifest)]=sha(manifest)
  lock=manifest.parent/'Cargo.lock';selectors[str(lock)]=sha(lock)
  pins=lambda rows:{(r['name'],r['version'],r.get('source'),r.get('checksum')) for r in rows}
  selected=tomllib.loads(lock.read_text())['package'];root=tomllib.loads((ROOT/'Cargo.lock').read_text())['package']
  if manifest.parent.name == 'proto-selector':
   reviewed_lock=Path('/tmp/mio89-confirmation-noq-tests-20261011/selector/Cargo.lock')
   assert lock.read_bytes()==reviewed_lock.read_bytes(), 'proto selector changed reviewed lock'
   selectors[str(reviewed_lock)]=sha(reviewed_lock)
  else:
   assert all(v in pins(root) or v[2] is None for v in pins(selected)), 'selector changed external pins'
  declaration=tomllib.loads(manifest.read_text())
  for section in ('lib','bin','test'):
   targets=declaration.get(section,[])
   if isinstance(targets,dict): targets=[targets]
   for target in targets:
    if 'path' in target:
     path=Path(target['path']);path=(path if path.is_absolute() else manifest.parent/path).resolve()
     assert path.is_file(), 'missing selected source: '+str(path)
     selectors[str(path)]=sha(path)
  if manifest.parent.name in ('selector','relay-selector'):
   reviewed_lock=Path('/tmp/mio88-base64-final-sdk-20261011/selector/Cargo.lock')
   reviewed_rows=tomllib.loads(reviewed_lock.read_text())['package']
   assert [r for r in selected if r['name']!='mio83-replica-red-selected']==[r for r in reviewed_rows if r['name']!='mio83-replica-red-selected'], 'selector changed nonlocal dependency rows'
   selectors[str(reviewed_lock)]=sha(reviewed_lock)
  for dependency_name,value in declaration.get('dependencies',{}).items():
   version=value if isinstance(value,str) else value.get('version','')
   package=dependency_name if isinstance(value,str) else value.get('package',dependency_name)
   if version.startswith('='):
    assert any(r['name']==package and r['version']==version[1:] for r in selected), 'exact dependency differs from pinned lock: '+package
 artifacts={}
 for directory in ['mio89-confirmation-reviewed-sdk-v4-20261011/native','mio89-confirmation-reviewed-sdk-v4-20261011/wasm','mio84-current-integration-20261011/native']:
  receipt=Path('/tmp')/directory/'receipt.json';r=json.loads(receipt.read_text())
  assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild']
  artifacts[str(receipt)]=sha(receipt)
  for path,h in r['artifacts'].items():
   f=Path(path) if Path(path).is_absolute() else receipt.parent/'pkg'/path
   assert sha(f)==h;artifacts[str(f)]=h
 p.write_text(json.dumps({'sources':sdk.current_sources(),'tools':sdk.tool_snapshot(),'selectorPins':selectors,'artifactPins':artifacts,'command':command,'driverSha256':sha(__file__)},indent=2)+'\n')
 print(json.dumps({'prepared':name,'sourceDigest':sdk.json_digest(sdk.current_sources()),'contextSha256':sha(p)}))

def guard(c):
 assert Path.cwd().resolve()==ROOT
 assert sha(__file__)==c['driverSha256']
 assert sdk.current_sources()==c['sources'],'source/lock/config changed'
 assert sdk.tool_snapshot()==c['tools'],'tool changed'
 assert all(sha(p)==h for p,h in c['selectorPins'].items()),'selector source/lock changed'
 assert all(sha(p)==h for p,h in c['artifactPins'].items()),'pinned baseline changed'

def run(name):
 p=HERE/(name+'-context.json');c=json.loads(p.read_text());guard(c)
 out=HERE/name;assert not out.exists()
 target=cache.target_directory(ROOT)
 assert target==ROOT/'target'
 with cache.cache_lease(target) as disk:
  guard(c);out.mkdir()
  cmd=cache.cargo_command(c['command'],target)
  (out/'preconditions.json').write_text(json.dumps({'sourceLockArtifactToolPreconditionsPassedBeforeCargo':True,'contextSha256':sha(p),'driverSha256':sha(__file__),'command':cmd,'targetDirectory':str(target),'diskBefore':disk},indent=2)+'\n')
  started=time.monotonic()
  with (out/'cargo.jsonl').open('w') as log,(out/'cargo.stderr.log').open('w') as err:
   result=subprocess.run(cmd,cwd=ROOT,env=cache.build_environment(target),stdout=log,stderr=err)
  records=sdk.parse_cargo_artifacts(out/'cargo.jsonl');artifacts={}
  report={'exit':result.returncode,'cargoExit':result.returncode,'cargoSeconds':time.monotonic()-started,'command':cmd,'freshArtifacts':sum(r.get('fresh') is True for r in records),'rebuiltArtifacts':sum(r.get('fresh') is False for r in records),'qualifiedForFrozenRun':False,'sourceFrozenDuringBuild':False,'artifacts':{}}
  (out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
  if result.returncode:
   print(json.dumps(report));return result.returncode
  for r in records:
   if r.get('executable') and not ('bin' in r['target']['kind'] and r.get('profile',{}).get('test')):
    f=Path(r['executable']);dest=out/'pinned'/r['target']['name'];dest.parent.mkdir(exist_ok=True)
    shutil.copy2(f,dest);assert sha(f)==sha(dest);artifacts[str(dest)]=sha(dest)
  if 'wasm32-unknown-unknown' in c['command']:
   f=sdk.wasm_artifact(records);dest=out/'miopunch_browser_probe.wasm';shutil.copy2(f,dest);assert sha(f)==sha(dest)
   artifacts[str(dest)]=sha(dest)
   binding=subprocess.run(['rtk','proxy',str(sdk.BINDGEN),'--target','web','--out-dir',str(out/'pkg'),str(dest)],cwd=ROOT,capture_output=True,text=True)
   (out/'bindgen.stdout.log').write_text(binding.stdout);(out/'bindgen.stderr.log').write_text(binding.stderr)
   assert binding.returncode==0
   artifacts.update({str(f):sha(f) for f in (out/'pkg').rglob('*') if f.is_file()})
  selected=sdk.capture_inputs(records,target,out,c['sources'])
  guard(c)
  report.update(sourceFrozenDuringBuild=True,qualifiedForFrozenRun=True,artifacts=artifacts,actualSelectedInputCount=len(selected['inputs']),actualCapturedDepfileCount=len(selected['depfiles']),actualSourceCopyCount=len(selected['copiedRepositoryInputs']),sourceDigestBefore=sdk.json_digest(c['sources']),sourceDigestAfter=sdk.json_digest(sdk.current_sources()),diskAfter=cache.disk_state(target))
  (out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
  print(json.dumps(report));return 0

parser=argparse.ArgumentParser();parser.add_argument('operation',choices=['prepare','run']);parser.add_argument('name');parser.add_argument('cargo',nargs=argparse.REMAINDER)
args=parser.parse_args()
if args.operation=='prepare':prepare(args.name,args.cargo[1:] if args.cargo[:1]==['--'] else args.cargo)
else:raise SystemExit(run(args.name))
