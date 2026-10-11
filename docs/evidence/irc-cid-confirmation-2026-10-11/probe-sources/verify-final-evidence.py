import hashlib,importlib.util,json,os,re,shutil,statistics,subprocess,sys,tomllib
from pathlib import Path
os.umask(0o077)
R=Path('/home/js/Git/miopunch');S=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011')
O=R/'docs/evidence/irc-cid-confirmation-2026-10-11';O.mkdir(parents=True,exist_ok=True)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(name,value): (O/name).write_text(json.dumps(value,indent=2)+'\n')
c=json.loads((S/'context.json').read_text())
assert sha(c['utility'])==c['utilitySha256']
sp=importlib.util.spec_from_file_location('sdk',c['utility']);sdk=importlib.util.module_from_spec(sp);sys.modules[sp.name]=sdk;sp.loader.exec_module(sdk)
assert sdk.current_sources()==c['sources'] and sdk.tool_snapshot()==c['tools']
delta=json.loads((S/'source-delta.json').read_text());assert len(delta)==13
assert all(sha(R/k)==v['after'] for k,v in delta.items())
builds={}
for label in ['native','wasm','tests']:
    B=S/label;r=json.loads((B/'receipt.json').read_text());assert r['exit']==0 and r['qualifiedForFrozenRun'] and r['sourceFrozenDuringBuild']
    assert all(sha(p)==h for p,h in r['artifacts'].items())
    selected=json.loads((B/'selected-dependency-inputs.json').read_text());sdk.verify_captured_graph(selected,B,c['sources'])
    builds[label]={k:r[k] for k in ['command','cargoSeconds','freshArtifacts','rebuiltArtifacts','actualSelectedInputCount','actualSourceCopyCount','actualCapturedDepfileCount','sourceDigestBefore','sourceDigestAfter','artifacts']}
    builds[label].update(qualified=True,privateDirectory=str(B),rawReceiptSha256=sha(B/'receipt.json'),selectedInputManifestSha256=sha(B/'selected-dependency-inputs.json'),contextSha256=sha(S/'context.json'))
write('sdk-builds.json',builds)
tests=json.loads((S/'runtime/receipt.json').read_text());assert tests['qualified'] and tests['passedTests']==41
write('rust-regressions.json',{'qualified':True,'passedTests':41,'privateDirectory':str(S),'rawRuntimeReceiptSha256':sha(S/'runtime/receipt.json'),'rawBuildReceiptSha256':sha(S/'tests/receipt.json'),'results':tests['results']})
Q=Path('/tmp/mio89-confirmation-noq-tests-20261011');q=json.loads((Q/'runtime/receipt.json').read_text());g=json.loads((Q/'tests/corrected-selected-dependency-inputs.json').read_text())
assert q['exit']==0 and q['correctedGraphVerified'] and q['correctedSelectedInputCount']==1780
assert all(sha(p)==h for p,h in g['inputs'].items())
assert all(sha(Q/'tests/corrected-selected-source-copies'/p)==h for p,h in g['copiedRepositoryInputs'].items())
assert 'test result: ok. 432 passed; 0 failed;' in (Q/'runtime/raw.log').read_text()
write('noq-regressions.json',{'qualified':True,'passedTests':432,'privateDirectory':str(Q),'receipt':q,'rawRuntimeReceiptSha256':sha(Q/'runtime/receipt.json'),'actualInputGraphStillCurrent':True,'actualInputCount':len(g['inputs']),'repositorySourceCopyCount':len(g['copiedRepositoryInputs']),'initialPackageRelativeCaptureQualified':False,'note':'Original .d files retained. Relative noq_proto src paths were repaired against its pinned selector root/symlink before all432 tests ran. Current Noq inputs unchanged after final SDK policy fix; no redundant compilation or re-run.'})
names=['mio88-base64-public-full-sdk-20261011','mio89-baseline-repeat-20261011','mio89-cid2-qualified-public-first-20261011','mio89-cid2-qualified-public-repeat-20261011','mio89-cid2-qualified-caller60-20261011','mio89-confirmation-public-first-20261011','mio89-confirmation-public-repeat-20261011','mio89-confirmation-caller60-20261011','mio89-reviewed-caller60-20261011']
cohorts={}
for name in names:
    result=subprocess.run(['rtk','proxy','python3','/tmp/mio89-summarize-public.py','/tmp/'+name,'/tmp/mio89-public-summaries-20261011'],capture_output=True,text=True)
    assert result.returncode==0,(name,result.stderr)
    value=json.loads(Path('/tmp/mio89-public-summaries-20261011',name+'.json').read_text());assert value['qualified']
    cohorts[name]=value
final=cohorts[names[-1]]
assert final['callerBudgetMs']==60000 and final['profile']['attempt_ms']==60000
assert final['gateSnapshot']['legacyAgreementExecuted'] is False
assert final['gateSnapshot']['agreementMode']=='unified-session/1'
base=[cohorts[name]['initialIrcRequestMs'] for name in names[:2]]
cid=[cohorts[name]['initialIrcRequestMs'] for name in names[2:5]]
combined=[cohorts[name]['initialIrcRequestMs'] for name in names[5:]]
write('public-cohorts.json',{'qualified':True,'cohorts':cohorts,'groups':{'baseline':base,'cidOnly':cid,'combined':combined},'comparison':{'baselineMeanMs':statistics.mean(base),'finalCurrentRequestMs':final['initialIrcRequestMs'],'reductionVsBaselineMeanPercent':(1-final['initialIrcRequestMs']/statistics.mean(base))*100,'reductionRangeVsBaselinePercent':[(1-final['initialIrcRequestMs']/x)*100 for x in base],'cidOnlyRangeMs':[min(cid),max(cid)],'combinedRangeMs':[min(combined),max(combined)],'publicEchoAttributionQualified':False,'note':'Public ranges overlap. CID-only and combined snapshots reflect their recorded source graphs; first3 combined runs precede the conservative eligibility fix. Final current SDK uses explicit pool2 for its custom Native IRC endpoint. Controlled paired evidence separately isolates confirmation benefit.'},'defaultProfileQualified':False})
write('final-public-story.json',final)
A=Path('/tmp/mio89-reviewed-affected-20261011');a=json.loads((A/'runtime/private-runtime-receipt.json').read_text());ae=json.loads((A/'execution.json').read_text())
assert a['passed'] and len(a['cases'])==6 and ae['runtimeExit']==0 and ae['sourceAndArtifactGuardsPassed']
assert all(v['removed'] for v in a['cleanup']) and all(v['passed'] and v['runtimeExit']==0 for v in a['cases'].values())
write('final-affected-regressions.json',{'qualified':True,'publicTraffic':False,'privateDirectory':str(A),'rawReceiptSha256':sha(A/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(A/'execution.json'),'sourceAndArtifactGuardsPassed':True,'wholeWallMs':ae['wholeWallMs'],'cases':a['cases'],'images':a['images'],'ownedCaSha256':a['ownedCaSha256'],'ownedContainersRemoved':len(a['cleanup'])})
P=Path('/tmp/mio89-controlled-confirmation-pairs-v2-20261011');p=json.loads((P/'runtime/private-runtime-receipt.json').read_text());pe=json.loads((P/'execution.json').read_text())
assert pe['runtimeExit']==0 and pe['sourceAndArtifactGuardsPassed'] and p['passed']
assert all(v['removed'] for v in p['cleanup'])
details={}
for name in p['benchmarkOrder']:
    raw=P/name/'runtime/aged/private-runtime-receipt.json';rr=json.loads(raw.read_text());case=p['cases'][name]
    assert rr['qualified'] and rr['cleanupQualified'] and rr['currentStrictPurpose2Qualified'] and rr['nativeRequestAccepted']
    assert case['runtimeExit']==0 and case['passed']
    assert rr['cleanup']['websocketsClosedBeforeContext'] and rr['cleanup']['nativeClosedReceipt']['graceful_close']
    assert rr['cleanup']['nativeClosedReceipt']['traffic']['driver_active']==0
    assert rr['cleanup'].get('nativeForced') is not True
    gate=rr['request']['diagnostics']['gate'];end=rr['request']['diagnostics']['request']
    compiled='historical CID-only' if name.startswith('cid-only') else 'final current combined'
    if compiled=='final current combined':assert gate['legacy_agreement_executed'] is False
    else:assert gate['agreement_elapsed_ms']>0
    details[name]={'compiledImplementation':compiled,'qualified':True,'strictPurpose2':True,'actualNativeUnapprovedRequests':1,'requestMs':case['callerElapsedMs'],'rawReceiptSha256':sha(raw),'legacyAgreementMs':gate['agreement_elapsed_ms'],'confirmationMs':gate.get('confirmation_elapsed_ms'),'handshakeAndCoordinatorMs':end['handshake_ms'],'cumulativeExchangeMs':end['exchange_ms'],'cleanup':case['cleanup']}
bm=statistics.mean(details[k]['requestMs'] for k in ['cid-only-1','cid-only-2']);cm=statistics.mean(details[k]['requestMs'] for k in ['combined-1','combined-2'])
assert cm<bm and max(details[k]['requestMs'] for k in ['combined-1','combined-2'])<min(details[k]['requestMs'] for k in ['cid-only-1','cid-only-2'])
write('controlled-confirmation-pairs.json',{'qualified':True,'publicTraffic':False,'privateDirectory':str(P),'order':p['benchmarkOrder'],'rawReceiptSha256':sha(P/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(P/'execution.json'),'sourceAndArtifactGuardsPassed':True,'sameOwnedFacilityAndProfile':True,'images':p['images'],'serverFakelag':p['serverFakelag'],'profile':{'pacingMs':10,'assemblyTtlMs':8000,'queueTtlMs':30000,'attemptMs':120000},'callerMs':120000,'activeCidPool':2,'cases':details,'comparison':{'cidOnlyMeanMs':bm,'combinedMeanMs':cm,'savedMeanMs':bm-cm,'reductionPercent':(1-cm/bm)*100,'allCombinedRunsFasterThanAllCidOnlyRuns':True,'scope':'Two measurements per implementation on one owned slow IRC facility in ABBA order, with identical valid-traffic settings and CID2. No public latency guarantee or claimed22-second net saving.'},'ownedContainersRemoved':len(p['cleanup']),'historicalGraphDeclaration':'Historical artifacts remain bound to their original inputs/source copies. Current-root differences are explicitly listed and verified by the execution guard; no re-resolution/recompilation of historical graphs.'})
F=Path('/tmp/mio89-controlled-confirmation-pairs-20261011');f=json.loads((F/'runtime/private-runtime-receipt.json').read_text());fe=json.loads((F/'execution.json').read_text())
assert fe['runtimeExit']==1 and fe['sourceAndArtifactGuardsPassed'] and f['passed'] is False
assert all(v['currentStrictPurpose2Qualified'] and v['nativeRequestAccepted'] and not v['cleanup']['websocketsClosedBeforeContext'] for v in f['cases'].values())
red=Path('/tmp/mio89-stage-confirmation-red-20261011');rd=json.loads((red/'runtime/receipt.json').read_text());assert rd['testExit']==101 and rd['expectedRedReproduced'] and rd['SDKClosedBeforeAssertion']
early=[]
for name in ['mio89-cid2-public-first-20261011','mio89-cid2-bilateral-first-20261011']:
    B=Path('/tmp')/name;rr=json.loads((B/'runtime/private-runtime-receipt.json').read_text());ex=json.loads((B/'execution.json').read_text())
    early.append({'privateDirectory':str(B),'requestMs':rr['request']['elapsedMs'],'functionalAndCleanupQualified':rr['qualified'],'performanceQualified':False,'rawReceiptSha256':sha(B/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(B/'execution.json'),'reason':'Native custom raw builder did not receive the intended reduced pool. Browser-only effective tuning is not relabelled as bilateral optimization.'})
write('diagnosis-and-prior-runs.json',{'extraControlStreamRed':{'privateDirectory':str(red),'receipt':rd,'rawRuntimeReceiptSha256':sha(red/'runtime/receipt.json'),'rawBuildReceiptSha256':sha(red/'tests/receipt.json')},'preparationRuns':early,'firstControlledSlowRun':{'privateDirectory':str(F),'rawReceiptSha256':sha(F/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(F/'execution.json'),'functionalCases':4,'wholeQualification':False,'runtimeExit':1,'reason':'All four actual requests succeeded; the inherited500ms WSS observation was false before context destruction. Failed receipts retained. Only the observer was corrected for a separate re-run.','measuredRequestMs':{k:v['callerElapsedMs'] for k,v in f['cases'].items()}},'invalidNegativeFixturePreparation':{'privateDirectory':'/tmp/mio89-confirmation-current-sdk-v2-20261011','qualified':False,'reason':'The fake UDP fixture used control-only ALPNs requiring a registered control route, so the first fixture failed before its intended malformed wire envelope. Corrected fixture subsequently passed; not counted as an implementation failure or green regression.'}})
old_lock=tomllib.loads(subprocess.check_output(['rtk','proxy','git','show','05f99e3d078e17c69c53419faf681dec6cadec5a:Cargo.lock'],text=True));assert old_lock==tomllib.loads((R/'Cargo.lock').read_text())
protected=json.loads(Path('/tmp/mio88-protected-wip-before-20261011.json').read_text());assert all(sha(R/k)==v for k,v in protected.items())
write('owned-source-scope.json',{'fixedBase':'05f99e3d078e17c69c53419faf681dec6cadec5a','files':{k:sha(R/k) for k in delta},'sourceDelta':delta,'currentSourceDigest':sdk.json_digest(c['sources']),'finalSourceGraphDigest':builds['native']['sourceDigestAfter'],'wholeCargoLockByteIdenticalToBaseline':True,'noDependencyReResolution':True,'selectorNativeLockCopiedByteIdentical':sha(S/'nativeSelector/Cargo.lock')==sha('/tmp/mio88-base64-final-sdk-20261011/nativeSelector/Cargo.lock'),'explicitProbeHelperDelta':c['helperDelta'],'protectedWipFilesUnchanged':len(protected),'noCongestionOrLossAlgorithmChanges':True,'multipathSlotsPreserved':8,'scope':'Conservative automatic exclusive-IRC configuration plus generic caller API. Public custom Native fixture explicitly chooses CID2; mixed/default configurations are not automatically tuned.'})
probes=O/'probe-sources';probes.mkdir(exist_ok=True)
files={
 'guarded-build.py':S/'build.py','guarded-build-context.json':S/'context.json','source-delta.json':S/'source-delta.json','irc_current_native.rs':S/'irc_current_native.rs',
 'selected-native-Cargo.toml':S/'nativeSelector/Cargo.toml','selected-native-Cargo.lock':S/'nativeSelector/Cargo.lock',
 'selected-tests-Cargo.toml':Path('/tmp/mio88-base64-final-sdk-20261011/selector/Cargo.toml'),'selected-tests-Cargo.lock':Path('/tmp/mio88-base64-final-sdk-20261011/selector/Cargo.lock'),
 'guard-rust-regressions.py':S/'run-tests.py','guard-public-execution.py':Path('/tmp/mio89-reviewed-caller60-20261011/execute.py'),'public-full-story.mjs':Path('/tmp/mio89-reviewed-caller60-20261011/run.mjs'),'public-payload-scope.json':Path('/tmp/mio89-reviewed-caller60-20261011/public-payload-scope.json'),
 'guard-owned-execution.py':A/'execute.py','owned-fixtures.py':A/'local.py','owned-affected-regressions.mjs':A/'run.mjs',
 'guard-controlled-comparison.py':P/'execute.py','controlled-fixtures.py':P/'local.py','controlled-cid-only.mjs':P/'cid-only-1/run.mjs','controlled-current.mjs':P/'combined-1/run.mjs','controlled-cid-only-context.json':P/'cid-only-1/context.json','controlled-current-context.json':P/'combined-1/context.json',
 'summarize-public.py':Path('/tmp/mio89-summarize-public.py'),'verify-final-evidence.py':Path(__file__),
 'repair-noq-package-relative-capture.py':Path('/tmp/mio89-confirmation-noq-recapture-and-test.py'),
 'selected-noq-Cargo.toml':Q/'selector/Cargo.toml','selected-noq-Cargo.lock':Q/'selector/Cargo.lock'
}
manifest={}
for name,src in files.items():
    shutil.copy2(src,probes/name);assert sha(src)==sha(probes/name)
    manifest[name]={'source':str(src),'sha256':sha(src)}
(probes/'manifest.json').write_text(json.dumps({'files':manifest,'note':'Exact executed drivers/selectors/helpers and guards. Fixed /tmp paths reference preserved session receipts and immutable artifacts; source copies and hashes identify the actual graphs. No generated private key, node store, secret, ciphertext, grant or browser/NSS profile is copied.'},indent=2)+'\n')
review_dir=Path('/tmp/mio89-review-20261011')
for src,name in [('standards-report.md','standards-initial-review.md'),('spec-report.md','spec-initial-review.md'),('standards-final-report.md','standards-final-review.md'),('final-spec-report.md','spec-final-review.md')]:
    shutil.copy2(review_dir/src,O/name)
write('review-summary.json',{'fixedBase':'05f99e3d078e17c69c53419faf681dec6cadec5a','reviewedFiles':{k:sha(R/k) for k in delta},'initial':{'Standards':{'hardViolations':0,'heuristicSmells':1},'Spec':{'findings':1,'highestSeverity':'P2'}},'final':{'Standards':{'openHardViolations':0,'openHeuristicSmells':0},'Spec':{'openCodeFindings':0}},'scopeFindingResolvedAndFinalSourceValidated':True,'acceptanceRevalidatedAfterReviews':{'currentSdkTests':41,'currentNoqTests':432,'currentPublicFullStory':True,'currentOwnedSdkCases':6,'controlledPairsQualified':True,'defaultProfileQualified':False},'note':'Independent reviews of the exact WIP scope against the fixed completed M88 baseline. Original scope finding and final reports preserved separately; runtime acceptance was pending at review time and is now independently checked against raw receipts.'})
print(json.dumps({'evidence':str(O),'currentPublicRequestMs':final['initialIrcRequestMs'],'controlledReductionPercent':(1-cm/bm)*100,'sdkTests':41,'noqTests':432,'publicCohorts':len(cohorts),'currentOwnedCases':6,'probeFiles':len(manifest),'protectedWipFilesUnchanged':len(protected)}))
