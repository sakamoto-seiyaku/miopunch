"""Minimise the actual Pixel carrier-startup failure, without an admission."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import time
import urllib.request

os.umask(0o077)
sys.dont_write_bytecode=True
ROOT=Path('/home/js/Git/miopunch')
HERE=Path(__file__).resolve().parent
MODE=sys.argv[1] if len(sys.argv)>1 else 'bare-wss'
assert MODE in ('bare-wss','mqtt-carrier')
OUT=HERE/('diagnose-pixel-'+MODE)
assert not OUT.exists()
OUT.mkdir(mode=0o700)
sys.path.insert(0,str(ROOT/'scripts'))
from android_pair_runtime_current import device_for, BrowserPage
from check_browser_relay_capability import OwnedCdpObserver
from check_unified_admission_current import _m82_prelude

UTILITY=Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
spec=importlib.util.spec_from_file_location('m84_wss_diagnostic_guard',UTILITY)
sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)
context=json.loads((HERE/'build-context.json').read_text())
assert sdk.current_sources()==context['sources']
WASM=Path('/tmp/mio89-confirmation-reviewed-sdk-v4-20261011/wasm/receipt.json')
PACKAGE=WASM.parent/'pkg'
raw=json.loads(WASM.read_text())
sdk.verify_captured_graph(json.loads((WASM.parent/'selected-dependency-inputs.json').read_text()),WASM.parent,context['sources'])
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in raw['artifacts'].items())
facility=json.loads(Path('/tmp/mio76-explicit-public-emqx-facility-20261005.json').read_text())
page=observer=None
report={'scope':'minimal Pixel Chrome configured MQTT WSS; no SDK network/admission/business',
    'publicTraffic':True,'command':['rtk','proxy','python3',str(Path(__file__))],
    'pageLoaded':False,'hypothesis':'Pixel configured public WSS unreachable independent of SDK',
    'diagnosticPassed':False,'mode':MODE}
shutil.copyfile(__file__,OUT/'executed-controller.py')
shutil.copyfile(HERE/'diagnose-pixel-wss.mjs',OUT/'executed-node-probe.mjs')
try:
    page=BrowserPage(device_for('browser'),PACKAGE,prelude=_m82_prelude(),environment_rtc_probe=False)
    page.start()
    report['pageLoaded']=True
    observer=OwnedCdpObserver(page);report['observerStart']=observer.start()
    with urllib.request.urlopen(page.cdp_base+'/json/list',timeout=3) as response:
        targets=[t for t in json.load(response) if t.get('type')=='page' and t.get('url')==page.url]
    assert len(targets)==1
    inputp=OUT/'private-input.json'
    inputp.write_text(json.dumps({'cdp':targets[0]['webSocketDebuggerUrl'],'wss':facility['wss_url'],
        'facility':facility,'mode':MODE})+'\n')
    command=['rtk','proxy','node',str(HERE/'diagnose-pixel-wss.mjs'),str(inputp)]
    run=subprocess.run(command,capture_output=True,text=True,timeout=40)
    (OUT/'node.stdout.log').write_text(run.stdout)
    (OUT/'node.stderr.log').write_text(run.stderr)
    report['diagnosticCommandExit']=run.returncode
    assert run.returncode==0
    report['bareWss']=json.loads(run.stdout)
    report['diagnosticPassed']=True
    time.sleep(0.2)
finally:
    if observer:
        report['networkMetadata']=observer.receipt()
        report['observerCleanup']=observer.close()
    if page:
        report['pageCleanup']=page.close()
    report['sdkSourcesStillCurrent']=sdk.current_sources()==context['sources']
    (OUT/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'diagnosticPassed':report['diagnosticPassed'],'bareWss':report.get('bareWss'),
    'networkEvents':report.get('networkMetadata',{}).get('events',[]),
    'observerExit':report.get('observerCleanup',{}).get('commandExit'),
    'pageCleanupSucceeded':report.get('pageCleanup',{}).get('cleanupSucceeded')}))
