import os,json,hashlib
from pathlib import Path
os.umask(0o077)
ROOT=Path('/home/js/Git/miopunch');OUT=ROOT/'docs/evidence/irc-base64-saferate-2026-10-11';P=Path('/tmp/mio88-base64-public-full-sdk-20261011');OUT.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
raw=P/'runtime/private-runtime-receipt.json';r=json.loads(raw.read_text());e=json.loads((P/'execution.json').read_text())
assert e['runtimeExit']==0 and e['sourceAndArtifactGuardsPassed'] and r['qualified'] and r['cleanupQualified'] and r['businessQualified']
g=r['request']['diagnostics']['gate'];c=g['coordinator'];b=r['business']['diagnostics']['bootstrap']['coordinator']
requests=r['nativeStatus']['requests'];assert len(requests)==1 and requests[0]['approved'] is False
assert r['request']['elapsedMs']<300000 and r['request']['result']=='AwaitingApproval'
assert c['version']==1 and c['purposeCode']==2 and c['restricted'] and c['ordinaryHandle'] is False
assert g['tls_authenticated'] and g['expected_contact_verified'] and g['business_handle'] is False
assert b['version']==1 and b['purposeCode']==1 and b['freshAck'] and b['bindingUnchanged'] and b['stableUnchanged'] and b['selected']
assert b['selectedRoute']=='relay:https://aps1-1.relay.n0.iroh.link./'
assert r['business']['result']=='PONG' and r['nativeBusiness']['stream_progress']
assert r['nativeMemberReleased']['authorized'] and r['nativeMemberReleased']['selected_relay']
assert r['nativeMemberReleased']['stable_id']==r['nativeBusiness']['stable_id']
assert r['materialCompletion']['result']=='Complete' and r['completeReopen']['status']['stage']=='Complete'
assert r['pendingReopen']['id']==r['completeReopen']['id']
assert r['startup']['status']['id']==r['completeReopen']['id']
assert r['approval']['new_records']==1
cleanup=r['cleanup'];assert cleanup['websocketsClosedBeforeContext'] and cleanup['nativeExit']['code']==0
assert r['firstNativeClose']['exit']['code']==0
assert r['firstNativeClose']['closed']['graceful_close'] is True
assert cleanup['nativeClosedReceipt']['graceful_close'] is True
assert r['firstNativeClose']['closed']['traffic']['driver_active']==0
assert cleanup['nativeClosedReceipt']['traffic']['driver_active']==0
events=r['nativeEvents'];purpose2=[v for v in events if v.get('event')=='unified-control-ready' and v.get('version')==1 and v.get('purposeCode')==2 and v.get('restricted') is True]
assert purpose2
readies=[v for v in events if v.get('event')=='ready'];assert len(readies)==2 and readies[0]['id']==readies[1]['id']
assert g['remote_id']==readies[0]['id']
native_member=[v for v in events if v.get('event')=='unified-path-ready' and v.get('version')==1 and v.get('purposeCode')==1]
assert native_member
paths=r['business']['diagnostics']['paths'];assert any(v.get('selected') and v.get('isRelay') for v in paths)
value={'qualified':True,'functionalPassed':True,'cleanupQualified':True,'privateDirectory':str(P),'rawReceiptSha256':sha(raw),'executionReceiptSha256':sha(P/'execution.json'),'sourceAndArtifactGuardsPassed':True,'wholeWallMs':e['wholeWallMs'],
 'callerBudgetMs':r['sdkCallerBudgetMs'],'profile':r['profile'],'initialIrcRequestMs':r['request']['elapsedMs'],
 'publicOrigin':r['actualPageOrigin'],'HTTPSStatus':r['pageStatus'],'initialBrowserDataPaths':{'relays':0,'rtcEnabled':False},
 'restrictedAdmission':{'version':1,'purposeCode':2,'TLSAuthenticated':True,'expectedContactVerified':True,'ordinaryHandle':False,'result':'AwaitingApproval','actualNativeUnapprovedRequests':1,'actualNativeStrictPurpose2Events':len(purpose2)},
 'approvalAndDurability':{'newApprovalRecords':1,'approvedRelayRequestMs':r['approvedRequest']['elapsedMs'],'materialResult':'Complete','authorizationSaved':r['materialCompletion']['status']['authorization_saved'],'secretSaved':r['materialCompletion']['status']['secret_saved'],'pendingAndCompleteBrowserIdentityPreserved':True,'nativeIdentityPreservedOnReopen':True,'completeStageAfterReopen':'Complete'},
 'ordinaryBusiness':{'version':1,'purposeCode':1,'alpn':r['nativeMemberReleased']['alpn'],'selectedRoute':b['selectedRoute'],'freshAck':True,'bindingUnchanged':True,'stableUnchanged':True,'memberAuthorized':True,'nativeAndBrowserSelectedRelay':True,'actualNativeMemberReadyEvents':len(native_member),'memberOpenMs':r['memberOpen']['elapsedMs'],'reply':'PONG','nativeOrdinaryStreamProgress':True,'nativeReleaseAndBusinessSameStableId':True},
 'cleanup':{'sdkBrowserCloseReturned':cleanup['browser']['closeReturned'],'sdkBrowserFreed':cleanup['browser']['freed'],'sdkBrowserCloseMs':cleanup['browser']['elapsedMs'],'socketImmediateObservationMs':500,'socketImmediateObservationPassed':cleanup['socketImmediateObservationPassed'],'socketLateObserverCeilingMs':60000,'socketLateWaitMs':cleanup['lateSocketWaitMs'],'allWssCloseEventsBeforeContext':True,'wssSocketCount':len(r['websockets']),'initialIRCIntermediateCloseMs':r['afterIrcBrowserClose']['elapsedMs'],'firstNativeExit':r['firstNativeClose']['exit']['code'],'firstNativeGracefulClose':r['firstNativeClose']['closed']['graceful_close'],'firstNativeCloseMs':r['firstNativeClose']['closed']['cleanup_elapsed_ms'],'firstNativeCloseObserverCeilingMs':120000,'finalNativeExit':cleanup['nativeExit']['code'],'finalNativeGracefulClose':cleanup['nativeClosedReceipt']['graceful_close'],'finalNativeCloseMs':cleanup['nativeClosedReceipt']['cleanup_elapsed_ms'],'nativeProcessesAbsentImmediate':cleanup['nativeProcessAbsentImmediate'],'nativeProcessesAbsentLate':cleanup['nativeProcessAbsentLate'],'ownedBrowserContextClosed':cleanup['ownedBrowserContextClosed'],'ownedBrowserProcessClosed':cleanup['ownedBrowserProcessClosed'],'ownedFixtureRemoved':cleanup['ownedFixtureRemoved']},
 'privacy':'only measured scalar assertions and hashes published; no identity strings, SDP, ciphertext, keys, actual grant IDs, secrets or stored materials'}
(OUT/'final-public-story.json').write_text(json.dumps(value,indent=2)+'\n')

w=r['wireObservation'];assert r['base64WireQualified']
assert w['protocol']=='MIR2' and w['encoding']=='standard-padded-Base64'
assert w['sentFragments']>0 and w['receivedFragments']>0 and w['invalidBase64']==0 and w['legacyFragments']==0
assert w['maxSentClientLineBytes']+r['browserPrefixBytes']+2<=512 and w['maxReceivedForwardedLineBytes']<=512
assert r['browserFragmentCapacity']==(512-r['browserPrefixBytes']-len(r['facility']['channel'].encode())-14-94)//4*3
assert r['nativeFragmentCapacity']==(512-r['nativePrefixBytes']-len(r['facility']['channel'].encode())-14-94)//4*3
assert r['firstSentDatagram']['decodedBytes']==1200 and r['firstSentDatagram']['fragments']==5
assert r['browserSaferate'] and r['nativeSaferate']
value.update(wireQualified=True,wireObservation=w,actualBrowserPrefixBytes=r['browserPrefixBytes'],actualNativePrefixBytes=r['nativePrefixBytes'],actualTargetBytes=len(r['facility']['channel'].encode()),actualBrowserFragmentCapacity=r['browserFragmentCapacity'],actualNativeFragmentCapacity=r['nativeFragmentCapacity'],firstActualSentDatagram=r['firstSentDatagram'],actualSaferate={'browser':True,'native':True},testBudgetQualification='Explicit 300-second caller budget. This measured 60.488-second request exceeds the current 60-second default; no default-budget success claimed.')
previous=json.loads((ROOT/'docs/evidence/mio87-irc-budget-lifetime-2026-10-11/final-public-story.json').read_text())
value['comparison']={'samePublicFacilityAndExplicitCallerProfile':True,'priorM87RequestMs':previous['initialIrcRequestMs'],'currentM88RequestMs':value['initialIrcRequestMs'],'reductionPercent':(1-value['initialIrcRequestMs']/previous['initialIrcRequestMs'])*100,'scope':'One measured public run for each combined implementation; not a guaranteed latency or isolated causal attribution to each change.'}
(OUT/'final-public-story.json').write_text(json.dumps(value,indent=2)+'\n')
S=Path('/tmp/mio88-base64-current-sdk-v2-20261011');ctx=json.loads((S/'context.json').read_text());builds={}
for label in ['native','wasm']:
 rr=json.loads((S/label/'receipt.json').read_text());assert rr['exit']==0 and rr['qualifiedForFrozenRun'] and rr['sourceFrozenDuringBuild']
 assert all(sha(p)==h for p,h in rr['artifacts'].items())
 builds[label]={'qualified':True,'privateDirectory':str(S/label),'rawReceiptSha256':sha(S/label/'receipt.json'),'selectedInputManifestSha256':sha(S/label/'selected-dependency-inputs.json'),'contextSha256':sha(S/'context.json'),'command':rr['command'],'sourceDigestBefore':rr['sourceDigestBefore'],'sourceDigestAfter':rr['sourceDigestAfter'],'cargoSeconds':rr['cargoSeconds'],'freshArtifacts':rr['freshArtifacts'],'rebuiltArtifacts':rr['rebuiltArtifacts'],'actualSelectedInputCount':rr['actualSelectedInputCount'],'actualRepositorySourceCopies':rr['actualSourceCopyCount'],'actualCapturedDepfiles':rr['actualCapturedDepfileCount'],'artifacts':rr['artifacts']}
(OUT/'sdk-builds.json').write_text(json.dumps(builds,indent=2)+'\n')
T=Path('/tmp/mio88-base64-final-sdk-20261011');test=json.loads((T/'runtime/receipt.json').read_text());current=json.loads((S/'native-test-current-input-revalidation.json').read_text());assert test['qualified'] and test['passedTests']==39 and current['qualified']
(OUT/'rust-regressions.json').write_text(json.dumps({'qualified':True,'passedTests':39,'rawRuntimeReceiptSha256':sha(T/'runtime/receipt.json'),'rawBuildReceiptSha256':sha(T/'tests/receipt.json'),'privateDirectory':str(T),'results':test['results'],'currentActualInputGraphRevalidation':current},indent=2)+'\n')
A=Path('/tmp/mio88-base64-affected-sdk-20261011');raw=json.loads((A/'runtime/private-runtime-receipt.json').read_text());ex=json.loads((A/'execution.json').read_text());assert raw['passed'] and len(raw['cases'])==6 and ex['runtimeExit']==0 and ex['sourceAndArtifactGuardsPassed'] and all(x['removed'] for x in raw['cleanup'])
assert all(x['passed'] and x['runtimeExit']==0 for x in raw['cases'].values())
owned={'qualified':True,'publicTraffic':False,'privateDirectory':str(A),'rawReceiptSha256':sha(A/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(A/'execution.json'),'sourceAndArtifactGuardsPassed':True,'wholeWallMs':ex['wholeWallMs'],'cases':raw['cases'],'ownedContainersRemoved':len(raw['cleanup']),'images':raw['images'],'ownedCaSha256':raw['ownedCaSha256']}
(OUT/'final-affected-regressions.json').write_text(json.dumps(owned,indent=2)+'\n')
D=Path('/tmp/mio88-quality-red-20261011');red=json.loads((D/'runtime/receipt.json').read_text());assert [x['exit'] for x in red['results']]==[101,0]
slow=Path('/tmp/mio88-public-full-sdk-20261011');slowraw=json.loads((slow/'runtime/private-runtime-receipt.json').read_text());slowex=json.loads((slow/'execution.json').read_text());assert slowraw['qualified'] and slowex['runtimeExit']==0 and slowraw['request']['elapsedMs']>previous['initialIrcRequestMs']
(OUT/'diagnosis-and-prior-runs.json').write_text(json.dumps({'qualityProbeBeforeFix':{'privateDirectory':str(D),'rawRuntimeReceiptSha256':sha(D/'runtime/receipt.json'),'rawBuildReceiptSha256':sha(D/'tests/receipt.json'),'controlTestExit':101,'controlObservedPeriodicPingCount':5,'controlObservationMs':1100,'ordinaryTestExit':0,'nativeEndpointsNaturallyClosedBeforeAssertions':True},'saferatePacingOriginalRed':{'privateDirectory':'/tmp/mio88-saferate-red-20261011','exit':101,'qualified':False,'symptom':'Configured 600ms wait prevented complete datagram inside the 400ms bound.'},'pacingOnlyPublicRun':{'privateDirectory':str(slow),'rawReceiptSha256':sha(slow/'runtime/private-runtime-receipt.json'),'executionReceiptSha256':sha(slow/'execution.json'),'functionalAndCleanupQualified':True,'requestMs':slowraw['request']['elapsedMs'],'performanceQualified':False,'reason':'Slower than the prior M87 complete request; kept as regression evidence, not relabelled as optimization success.'}},indent=2)+'\n')
import subprocess,tomllib
old_lock=tomllib.loads(subprocess.check_output(['rtk','proxy','git','show','b696ecbba54733277d2562e1a8a7ab78767dc494:Cargo.lock'],text=True))['package'];new_lock=tomllib.loads((ROOT/'Cargo.lock').read_text())['package'];foreign=lambda xs:[x for x in xs if x['name'] not in {'miopunch-native-probe','miopunch-browser-probe'}];assert foreign(old_lock)==foreign(new_lock)
for name in ['miopunch-native-probe','miopunch-browser-probe']:
 before=next(x for x in old_lock if x['name']==name);after=next(x for x in new_lock if x['name']==name);assert {k:v for k,v in before.items() if k!='dependencies'}=={k:v for k,v in after.items() if k!='dependencies'};assert sorted(after['dependencies'])==sorted(before['dependencies']+['base64 0.22.1'])
paths=['Cargo.toml','Cargo.lock','browser-probe/Cargo.toml','src/irc_datagram.rs','src/irc_transport.rs','browser-probe/src/irc.rs','tests/irc_datagram.rs','tests/unified_connection.rs','vendor/iroh/src/socket/remote_map/remote_state.rs']
(OUT/'owned-source-scope.json').write_text(json.dumps({'fixedBase':'b696ecbba54733277d2562e1a8a7ab78767dc494','files':{p:sha(ROOT/p) for p in paths},'currentSourceDigest':builds['native']['sourceDigestAfter'],'registryPackageRowsVersionsSourcesAndChecksumsUnchanged':True,'onlyLocalDependencyEdgesAdded':['miopunch-native-probe -> base64 0.22.1','miopunch-browser-probe -> base64 0.22.1'],'noCargoLockResolution':True,'noProductQuicAlgorithmOrDefaultCallerBudgetChange':True},indent=2)+'\n')
print(json.dumps({'fullPublicQualified':True,'initialIrcRequestMs':value['initialIrcRequestMs'],'wireCapacity':value['actualBrowserFragmentCapacity'],'firstDatagramFragments':value['firstActualSentDatagram']['fragments'],'ownedSdkRegressions':len(raw['cases']),'nativeRegressions':39,'cleanupPassed':True}))
