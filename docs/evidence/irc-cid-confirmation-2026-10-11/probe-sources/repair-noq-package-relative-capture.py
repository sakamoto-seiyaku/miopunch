import hashlib,json,os,shlex,shutil,subprocess
from pathlib import Path
os.umask(0o077)
R=Path('/home/js/Git/miopunch');P=Path('/tmp/mio89-confirmation-noq-tests-20261011');B=P/'tests';S=P/'selector'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
c=json.loads((P/'context.json').read_text());r=json.loads((B/'receipt.json').read_text());old=json.loads((B/'selected-dependency-inputs.json').read_text())
assert r['exit']==0 and r['sourceFrozenDuringBuild']
if not (B/'selected-dependency-inputs-before-package-relative-repair.json').exists():
    shutil.copy2(B/'selected-dependency-inputs.json',B/'selected-dependency-inputs-before-package-relative-repair.json')
    shutil.copy2(B/'receipt.json',B/'receipt-before-package-relative-repair.json')
inputs={};copies={}
for raw,expected in old['depfiles'].items():
    captured=B/'depfiles'/Path(raw).relative_to(R/'target')
    assert sha(captured)==expected
    line=captured.read_text().replace('\\\n',' ').splitlines()[0]
    for token in shlex.split(line.split(': ',1)[1]):
        q=Path(token)
        if not q.is_absolute():q=(S if Path(raw).name.startswith('noq_proto-') else R)/q
        q=q.resolve();assert q.is_file(),str(q)
        inputs[str(q)]=sha(q)
        if q.is_relative_to(R):
            key=str(q.relative_to(R));copies[key]=sha(q)
            if key in c['sources']:assert c['sources'][key]==sha(q)
            dest=B/'corrected-selected-source-copies'/key;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(q,dest)
value={'inputs':inputs,'copiedRepositoryInputs':copies,'depfiles':old['depfiles'],'sourceResolution':'Relative src paths in the noq_proto depfile resolve against its pinned selector package root, whose src symlink resolves to the current repository vendor/noq-proto/src. Other dependency paths retain their original root/absolute resolution.'}
(B/'corrected-selected-dependency-inputs.json').write_text(json.dumps(value,indent=2)+'\n')
assert all(sha(p)==h for p,h in r['artifacts'].items())
assert all(sha(p)==h for p,h in c['selectorPins'].items())
assert any('vendor/noq-proto/src/tests/multipath.rs' in p for p in inputs)
out=P/'runtime';out.mkdir(mode=0o700)
binary=next(iter(r['artifacts']))
command=['rtk','proxy',binary,'--test-threads=2']
with (out/'raw.log').open('w') as log:result=subprocess.run(command,cwd=R,stdout=log,stderr=subprocess.STDOUT)
assert all(sha(p)==h for p,h in inputs.items())
summary={'exit':result.returncode,'command':command,'binarySha256':sha(binary),'correctedSelectedInputCount':len(inputs),'correctedRepositoryInputCount':len(copies),'correctedGraphSha256':sha(B/'corrected-selected-dependency-inputs.json'),'rawLogSha256':sha(out/'raw.log'),'initialCapturePackageRelativeMappingQualified':False,'correctedGraphVerified':True,'selectorsUnchanged':True}
(out/'receipt.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary));raise SystemExit(result.returncode)
