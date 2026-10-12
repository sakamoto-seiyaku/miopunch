import argparse,hashlib,importlib.util,json,subprocess,sys,time
from pathlib import Path
ROOT=Path('/home/js/Git/miopunch');HERE=Path(__file__).parent
UTILITY=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(UTILITY)=='2b7e969a1f90fc93eacc80d8b62890f0bb0ef8f40dc140d9398ef1a6029671e1'
spec=importlib.util.spec_from_file_location('socket_runtime_sdk',UTILITY);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
a=argparse.ArgumentParser();a.add_argument('build');a.add_argument('binary');a.add_argument('name');a.add_argument('args',nargs=argparse.REMAINDER);o=a.parse_args()
build=HERE/o.build;receipt=json.loads((build/'receipt.json').read_text());context=json.loads((HERE/(o.build+'-context.json')).read_text())
assert receipt['exit']==0 and receipt['qualifiedForFrozenRun'] and receipt['sourceFrozenDuringBuild']
assert sdk.current_sources()==context['sources'];assert sdk.tool_snapshot()==context['tools']
assert all(sha(p)==h for p,h in context['selectorPins'].items())
assert all(sha(p)==h for p,h in context['artifactPins'].items())
f=build/'pinned'/o.binary;assert sha(f)==receipt['artifacts'][str(f)]
out=HERE/o.name;assert not out.exists();out.mkdir()
command=['rtk','proxy',str(f)]+(o.args[1:] if o.args[:1]==['--'] else o.args)
started=time.monotonic()
with (out/'stdout.log').open('w') as stdout,(out/'stderr.log').open('w') as stderr:
 try:r=subprocess.run(command,cwd=ROOT,stdout=stdout,stderr=stderr,timeout=300);code=r.returncode
 except subprocess.TimeoutExpired:code=124
same=sdk.current_sources()==context['sources'] and sha(f)==receipt['artifacts'][str(f)]
result={'exit':code,'seconds':time.monotonic()-started,'command':command,'buildReceiptSha256':sha(build/'receipt.json'),'sourceDigest':receipt['sourceDigestAfter'],'sourceAndArtifactUnchangedAfterRuntime':same,'qualified':code==0 and same}
(out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));print((out/'stdout.log').read_text()[-10000:]);raise SystemExit(0 if result['qualified'] else 1)
