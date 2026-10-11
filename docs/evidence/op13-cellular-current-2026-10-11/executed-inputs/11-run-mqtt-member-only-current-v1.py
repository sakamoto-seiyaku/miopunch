"""One current two-phone SDK story at a time; preserve each original receipt."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from types import SimpleNamespace

os.umask(0o077)
sys.dont_write_bytecode=True
ROOT=Path('/home/js/Git/miopunch')
HERE=Path(__file__).resolve().parent
UTILITY=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
WASM=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json')
WASM_META=Path('/tmp/mio84-current-integration-20261011/wasm-metadata/receipt.json')
PACKAGE=WASM.parent/'pkg'
FACILITY=Path('/tmp/mio76-explicit-public-emqx-facility-20261005.json')
RELAY='https://aps1-1.relay.n0.iroh.link./'
sys.path.insert(0,str(ROOT/'scripts'))

def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

def write(p,value):
    p.write_text(json.dumps(value,indent=2)+'\n')

def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec)
    sys.modules[name]=module
    spec.loader.exec_module(module)
    return module

context=json.loads((HERE/'build-context.json').read_text())
assert sha(UTILITY)==context['utilitySha256']
sdk=load('m84_two_phone_story_guard',UTILITY)

def guard(case):
    assert sdk.current_sources()==context['sources'] and sdk.tool_snapshot()==context['tools']
    result={}
    for name,p in [('android',HERE/'android/receipt.json'),('wasm',WASM)]:
        raw=json.loads(p.read_text())
        assert raw['exit']==0 and raw['qualifiedForFrozenRun'] and raw['sourceFrozenDuringBuild']
        graph=json.loads((p.parent/'selected-dependency-inputs.json').read_text())
        sdk.verify_captured_graph(graph,p.parent,context['sources'])
        assert all(sha(path)==digest for path,digest in raw['artifacts'].items())
        result[name]={'receiptSha256':sha(p),'selectedGraphSha256':sha(p.parent/'selected-dependency-inputs.json'),
            'actualSelectedInputCount':len(graph['inputs']),'repositorySourceCopies':len(graph['copiedRepositoryInputs']),
            'capturedDepfiles':len(graph['depfiles']),'batchBuildCountersUnchanged':True}
    raw=json.loads((HERE/'android/receipt.json').read_text())
    deployment=raw['deployment']['unified_admission_story']
    assert deployment['compiledSha256']==sha(deployment['compiledPath'])
    assert deployment['deployedSha256']==sha(deployment['deployedPath'])
    assert deployment['transformation']['exit']==0
    result['native']={**deployment,'receiptSha256':sha(HERE/'android/receipt.json'),
        'platform':'Android/PJZ110','selectedSdkInputsVerified':True,
        'selectionScope':'one deployed binary from the frozen three-binary Cargo batch',
        'currentSourceDigest':context['sourceDigest']}
    import check_unified_admission_current as root_controller
    result['browser']=root_controller._guard_frozen_browser_build(WASM_META,PACKAGE)
    assert json.loads(WASM_META.read_text())['rawReceiptSha256']==sha(WASM)
    files=[Path(__file__),HERE/'build-context.json',HERE/'build-current-android.py',
        HERE/'private-native-profile.json',HERE/'private-browser-profile.json',FACILITY,
        case/'controller.py',case/'capture-helper.py',
        Path('/tmp/mio82-android-capture-tools-root-20261007/tcpdump-arm64'),
        *sorted((ROOT/'scripts').glob('*.py')),
        *sorted((ROOT/'browser-probe/harness').glob('*.mjs')),
        *sorted((ROOT/'browser-probe/harness').glob('*.js'))]
    result['files']={str(p):sha(p) for p in files}
    result['sourceDigest']=context['sourceDigest']
    return result

def network_facts():
    from android_pair_runtime_current import device_for
    from android_pair_route_receipt_current import _agent_facts
    result={}
    for role in ('native','browser'):
        device=device_for(role)
        stdout,exit_code=device.shell('dumpsys','connectivity')
        assert exit_code==0
        facts=_agent_facts(stdout)
        assert facts['activeDefaultObserved'] and facts['defaultAgentObserved']
        assert facts['defaultNotVpnCapability'] and facts['currentVpnAgentCount']==0
        assert len(facts['interfaces'])==1
        assert facts['defaultTransport']==('CELLULAR' if role=='native' else 'WIFI')
        result[role]={k:v for k,v in facts.items() if k not in ('interfaces','selectedAgent')}
    return result

action,name=sys.argv[1:3]
assert name=='mqtt-rtc-whole-story'
case=HERE/'mqtt-member-only-current-v1'
if action=='prepare':
    assert not case.exists()
    case.mkdir(mode=0o700)
    original=Path('/tmp/mio82-op13-returned-runtime-root-20261008/controller.py') if name=='mqtt-rtc-whole-story' else Path('/tmp/mio84-op13-rtc-disabled-public-relay-root-20261008/controller.py')
    text=original.read_text()
    if name=='relay-with-rtc-allowed':
        before='''        if any(path.get("isRtc") is True for path in member_after.get("paths", [])):
            fail("RTC-disabled ordinary M retained RTC data candidate")
        if coordinator.get("rtcRole") != "None":
            fail("RTC-disabled member advertised RTC role")
        objects = checked(sdk("rtc_fixture_snapshot"), "rtc_fixture_snapshot")
        if objects.get("peerCreated") != 0 or objects.get("channelCreated") != 0:
            fail("RTC-disabled SDK created PC or DataChannel")
        private["relayOnlyMember"] = {"browser":member_after,"native":member_native,"objects":objects}
        report["rtcDisabledRelay"] = {"selectedRelay":True,"freshAck":True,"ordinaryOriginalM":True,
            "noRtcCandidates":True,"rtcRole":"None","peerCreated":0,"channelCreated":0,"businessMatched":True}
'''
        after='''        objects = checked(sdk("rtc_fixture_snapshot"), "rtc_fixture_snapshot")
        private["relayWithRtcAllowedMember"] = {"browser":member_after,"native":member_native,"objects":objects}
        report["relayWithRtcAllowed"] = {"selectedRelay":True,"freshAck":True,"ordinaryOriginalM":True,
            "rtcEnabled":True,"rtcRole":coordinator.get("rtcRole"),
            "peerCreated":objects.get("peerCreated"),"channelCreated":objects.get("channelCreated"),
            "businessMatched":True,"explicitRtcImprovementInvoked":False}
'''
        assert text.count(before)==1
        text=text.replace(before,after)
    if name!='mqtt-rtc-whole-story':
        text=text.replace('M82 restricted admission/Complete/reopen/nonempty replica/live ordinary-M RTC PING/PONG',
                          'Current two-phone admission/Complete/reopen/nonempty replica/live relay M PING/PONG')
    marker='report["cleanup"]["browserObserver"] = observer.close()'
    assert text.count(marker)==1
    text=text.replace(marker,'report["carrierNetworkMetadata"] = observer.receipt()\n                '+marker)
    first='        before_replica = checked(sdk("material_receipt"), "material_receipt")'
    last='        stage("ordinary-member-open", lambda: checked(sdk("open_member", {"target": ready["id"]}), "open_member"))'
    i=text.index(first);j=text.index(last,i)
    text=text[:i]+'        report["replica"] = {"attempted":False,"qualified":False,"scope":"separate ordinary member test; full nonempty replica previously failed"}\n'+text[j:]
    text=text.replace('report["m82StoryQualified"] = True','report["m82StoryQualified"] = False\n        report["ordinaryMemberStoryQualified"] = True')
    text=text.replace('M82 restricted admission/Complete/reopen/nonempty replica/live ordinary-M RTC PING/PONG','Current ordinary member only: MQTT admission/Complete/reopen/RTC PING/PONG; no nonempty replica qualification')
    key='            if operation == "open_member" and page is not None:'
    diagnostic='            if operation in ("open_member", "sync_replica") and page is not None:\n                for diagnostic_op in ("rtc_fixture_snapshot","replica_diagnostics","member_diagnostics","path_diagnostics"):\n                    try:\n                        observed=page.call(diagnostic_op, timeout=5)\n                        private.setdefault("failureDiagnostics",[]).append({"operation":operation,"diagnostic":diagnostic_op,"reply":observed})\n                    except Exception as diagnostic_error:\n                        private.setdefault("failureDiagnostics",[]).append({"operation":operation,"diagnostic":diagnostic_op,"failureClass":type(diagnostic_error).__name__})\n                save_private()\n'
    assert text.count(key)==1
    text=text.replace(key,diagnostic+key)
    (case/'controller.py').write_text(text)
    shutil.copyfile('/tmp/mio82-op13-returned-runtime-root-20261008/capture-helper.py',case/'capture-helper.py')
    write(case/'preparation.json',{'originalController':str(original),'originalControllerSha256':sha(original),
        'currentSdk':True,'rootSdkSourceChanged':False,
        'observerOnlyDelta':'Separate strict ordinary-member test without Replica synchronization; full m82StoryQualified must remain false; read-only failure diagnostics after real operation',
        'case':name,'actorsStarted':0})
    write(case/'binding.json',guard(case))
    print(json.dumps({'case':name,'prepared':True,'actorsStarted':0}))
elif action=='execute':
    before=guard(case)
    assert before==json.loads((case/'binding.json').read_text()) and not (case/'runtime').exists()
    write(case/'network-before.json',network_facts())
    controller=load('m84_current_two_phone_story',case/'controller.py')
    import android_pair_runtime_current as runtime
    runtime.install_for_frozen_controller()
    import check_unified_admission as frozen
    options=SimpleNamespace(output=case/'runtime',facility=FACILITY,
        native_profile=HERE/'private-native-profile.json',browser_profile=HERE/'private-browser-profile.json',
        package=PACKAGE,packet_capture_helper=case/'capture-helper.py',
        relay=None if name=='mqtt-rtc-whole-story' else RELAY,
        rtc_disabled=name=='relay-only-whole-story',native_platform='android')
    command=['rtk','proxy','python3',str(Path(__file__)),'execute',name]
    write(case/'command.json',{'command':command,'actualSdkOptions':{
        'relayConfigured':options.relay is not None,'rtcDisabled':options.rtc_disabled,
        'nativePlatform':options.native_platform,'actorModels':['PJZ110','Pixel_6a']}})
    started=time.monotonic()
    code=controller.run_m82_story(options,before['native'],before['browser'],runtime,frozen)
    stable=guard(case)==before
    network=network_facts()
    write(case/'network-after.json',network)
    raw=json.loads((case/'runtime/receipt.json').read_text())
    execution={'case':name,'runtimeExit':code,'sourceAndArtifactGuardsPassed':stable,
        'wholeWallMs':round((time.monotonic()-started)*1000,3),'passed':raw.get('passed') is True,
        'runtimeReceiptSha256':sha(case/'runtime/receipt.json'),'firstFailure':raw.get('firstFailure'),
        'physicalWanQualified':raw.get('physicalWanQualified') is True,
        'normalTlsVerified':raw.get('normalTlsVerified') is True,
        'bothDefaultPhysicalNetworksStillNoVpn':True}
    write(case/'execution.json',execution)
    print(json.dumps({k:v for k,v in execution.items() if k!='firstFailure'}),flush=True)
    if raw.get('firstFailure'):
        print(json.dumps({'firstFailure':{k:v for k,v in raw['firstFailure'].items() if k in ('phase','failureClass')}}),flush=True)
    raise SystemExit(code if stable else 1)
else:
    raise SystemExit('prepare or execute required')
