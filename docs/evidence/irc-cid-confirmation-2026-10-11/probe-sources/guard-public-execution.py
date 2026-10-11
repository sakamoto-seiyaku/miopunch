import os,json,hashlib,subprocess,importlib.util,sys,time
from pathlib import Path
os.umask(0o077);ROOT=Path('/home/js/Git/miopunch');P=Path(__file__).resolve().parent;W=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm');N=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/native');ctx=json.loads((P/'context.json').read_text())
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(ctx['utility'])==ctx['utilitySha256'];spec=importlib.util.spec_from_file_location('irc_sdk_guard',ctx['utility']);sdk=importlib.util.module_from_spec(spec);sys.modules[spec.name]=sdk;spec.loader.exec_module(sdk)
def guard():
 assert sdk.current_sources()==ctx['sources'] and sdk.tool_snapshot()==ctx['tools']
 assert all(sha(p)==h for p,h in ctx['selectorPins'].items())
 for out in [N,W]:
  r=json.loads((out/'receipt.json').read_text());assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild'];assert all(sha(p)==h for p,h in r['artifacts'].items());sdk.verify_captured_graph(json.loads((out/'selected-dependency-inputs.json').read_text()),out,ctx['sources'])
 paths=[Path(__file__),P/'run.mjs',P/'context.json',N/'receipt.json',W/'receipt.json',*map(Path,ctx['selectorPins']),Path('/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'),ROOT/'docs/evidence/public-irc-restricted-2026-10-05/facility.json',P/'public-payload-scope.json',Path('/tmp/mio84-current-public-irc-60-root-20261008/context.json'),Path('/tmp/mio84-current-public-irc-60-root-20261008/selector/Cargo.toml'),Path('/tmp/mio84-current-public-irc-60-root-20261008/selector/Cargo.lock')]
 return {str(p):sha(p) for p in paths}
before=guard();assert not (P/'runtime').exists();binding={'sourceLocksArtifactsAndControllerPassedBeforeActors':True,'pins':before,'productSourceChanged':True,'sdkBudgetMs':60000,'profilePacingMs':600,'actorsStarted':0}
if sys.argv[1]=='prepare':
 assert not (P/'execution-preconditions.json').exists();(P/'execution-preconditions.json').write_text(json.dumps(binding,indent=2));print(json.dumps({'preconditionsPassed':True,'actorsStarted':0}));raise SystemExit(0)
assert binding==json.loads((P/'execution-preconditions.json').read_text());started=time.monotonic()
r=subprocess.run(['rtk','proxy','node',str(P/'run.mjs')],cwd=ROOT);after=guard();report={'runtimeExit':r.returncode,'sourceAndArtifactGuardsPassed':before==after,'wholeWallMs':round((time.monotonic()-started)*1000,3),'rawRuntimeReceiptSha256':sha(P/'runtime/private-runtime-receipt.json') if (P/'runtime/private-runtime-receipt.json').exists() else None,'productSourceChanged':True};(P/'execution.json').write_text(json.dumps(report,indent=2));print(json.dumps(report));raise SystemExit(r.returncode if before==after else 1)
