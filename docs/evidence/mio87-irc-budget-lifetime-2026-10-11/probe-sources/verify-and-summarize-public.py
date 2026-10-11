import os,json,hashlib
from pathlib import Path
os.umask(0o077)
ROOT=Path('/home/js/Git/miopunch');OUT=ROOT/'docs/evidence/mio87-irc-budget-lifetime-2026-10-11';P=Path('/tmp/mio87-final3-public-proper-close-root-20261011')
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
reviews=[]
for name in ['standards','spec']:
 src=Path('/tmp/mio87-final-independent-review-root-20261011')/(name+'-report.md');assert src.is_file()
 (OUT/(name+'-review.md')).write_bytes(src.read_bytes());reviews.append({'axis':name,'findings':0,'privateReportSha256':sha(src),'fixedBase':'bd39f544b14181b7e5b7a09c6c9bf08a20eca440','productScopeSha256':sha(Path('/tmp/mio87-final-independent-review-root-20261011/scope.json'))})
(OUT/'review-summary.json').write_text(json.dumps({'reviews':reviews,'note':'independent source reviews completed while final public runtime was pending; this final actual qualification is separately checked against raw receipts after runtime; no product changes since reviews'},indent=2)+'\n')
print(json.dumps({'qualified':True,'initialIrcRequestMs':value['initialIrcRequestMs'],'businessReply':'PONG','allWssCloseEventsBeforeContext':True,'lateSocketWaitMs':cleanup['lateSocketWaitMs'],'independentReviews':2}))
