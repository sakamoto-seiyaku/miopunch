import hashlib,importlib.util,json,os,shlex,subprocess,sys,time
from pathlib import Path
os.umask(0o077)
R=Path('/home/js/Git/miopunch');P=Path(__file__).resolve().parent
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
final=json.loads(Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/context.json').read_text())
assert sha(final['utility'])==final['utilitySha256']
sp=importlib.util.spec_from_file_location('sdk',final['utility']);sdk=importlib.util.module_from_spec(sp);sys.modules[sp.name]=sdk;sp.loader.exec_module(sdk)
def guard():
    assert sdk.current_sources()==final['sources'] and sdk.tool_snapshot()==final['tools']
    pins={}
    for name in ['cid-only-1','combined-1','combined-2','cid-only-2']:
        C=P/name;c=json.loads((C/'context.json').read_text());S=Path(c['SDK'])
        assert c['currentRootSources']==final['sources']
        assert c['tools']==final['tools']
        for path,h in c['selectorPins'].items():assert sha(path)==h
        for label in ['native','wasm']:
            B=S/label;r=json.loads((B/'receipt.json').read_text())
            assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild']
            assert all(sha(p)==h for p,h in r['artifacts'].items())
            selected=json.loads((B/'selected-dependency-inputs.json').read_text());union=set()
            for raw,h in selected['depfiles'].items():
                capture=B/'depfiles'/Path(raw).resolve().relative_to(R/'target')
                assert sha(capture)==h
                first=capture.read_text().replace('\\\n',' ').splitlines()[0]
                for token in shlex.split(first.split(': ',1)[1]):
                    f=Path(token);union.add(str((f if f.is_absolute() else R/f).resolve()))
            assert union==set(selected['inputs'])
            for path,h in selected['inputs'].items():
                f=Path(path)
                if f.is_relative_to(R) and str(f.relative_to(R)) in c['historicalSourceDelta']:
                    d=c['historicalSourceDelta'][str(f.relative_to(R))]
                    assert h==d['compiled'] and sha(f)==d['current']
                else:assert sha(f)==h,path
            for path,h in selected['copiedRepositoryInputs'].items():
                assert sha(B/'selected-source-copies'/path)==h
            for path in [B/'receipt.json',B/'selected-dependency-inputs.json']:
                pins[str(path)]=sha(path)
        for f in [C/'context.json',C/'run.mjs',C/'public-payload-scope.json']:
            pins[str(f)]=sha(f)
    for f in [Path(__file__),P/'local.py',R/'scripts/browser_mqtt_trust.py',R/'browser-probe/harness/owned-irc-fixture.mjs',R/'browser-probe/harness/owned-mixed-mqtt-proxy.mjs',R/'tests/fixtures/irc/ergo.yaml',R/'scripts/mosquitto-test.conf',Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome')]:
        pins[str(f)]=sha(f)
    return pins
before=guard()
binding={'sourceLocksArtifactsAndControllersPassedBeforeActors':True,'pins':before,'actorsStarted':0,'comparison':'Pinned historical CID-only SDK versus final combined SDK. Historical selected source copies are verified; only explicitly declared repository source deltas may differ from the current root. No historical artifact is relabelled as current.','order':['cid-only-1','combined-1','combined-2','cid-only-2'],'publicTraffic':False,'profile':{'pacing_ms':10,'assembly_ttl_ms':8000,'queue_ttl_ms':30000,'attempt_ms':120000},'callerMs':120000}
assert not (P/'runtime').exists()
if sys.argv[1]=='prepare':
    assert not (P/'execution-preconditions.json').exists()
    (P/'execution-preconditions.json').write_text(json.dumps(binding,indent=2)+'\n')
    print(json.dumps({'preconditionsPassed':True,'actorsStarted':0}));raise SystemExit(0)
assert binding==json.loads((P/'execution-preconditions.json').read_text())
start=time.monotonic();result=subprocess.run(['rtk','proxy','python3',str(P/'local.py')],cwd=R)
after=guard();report={'runtimeExit':result.returncode,'sourceAndArtifactGuardsPassed':before==after,'wholeWallMs':(time.monotonic()-start)*1000,'rawReceiptSha256':sha(P/'runtime/private-runtime-receipt.json')}
(P/'execution.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(result.returncode if before==after else 1)
