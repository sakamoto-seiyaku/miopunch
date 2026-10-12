import json,shutil,subprocess
from pathlib import Path
HERE=Path(__file__).parent;report={'fixtures':{},'keysRemoved':0,'nssDatabasesRemoved':0,'hostProcessChecksPassed':False}
for name in ['controlled-demo','controlled-demo-v2']:
 p=HERE/name;r=json.loads((p/'execution.json').read_text());assert r['cleanup']['relayExit']==0 and r['cleanup']['relayGraceful'] and r['cleanup']['relayProcessAbsent'];raw=json.loads((p/'runtime/private-runtime-receipt.json').read_text());assert raw['cleanup']['ownedBrowserProcessClosed'] and raw['cleanup']['nativeProcessAbsentLate'] and not raw['cleanup'].get('nativeForced')
 for c in r['containers']:
  assert c['removed'];x=subprocess.run(['rtk','proxy','docker','inspect',c['name']],capture_output=True,text=True);assert x.returncode!=0 and 'no such object' in x.stderr.lower() and json.loads(x.stdout)==[]
 f=p/'private-fixture';live=[]
 for proc in Path('/proc').iterdir():
  if not proc.name.isdigit():continue
  try:cmd=(proc/'cmdline').read_bytes().replace(b'\0',b' ').decode(errors='replace')
  except(FileNotFoundError,ProcessLookupError,PermissionError):continue
  if str(f) in cmd:live.append(int(proc.name))
 assert not live,'owned fixture process still active'
 keys=list(f.glob('*.key'));nss=f/'nssdb';kept=[x.name for x in f.iterdir() if x not in keys and x!=nss]
 for key in keys:key.unlink();report['keysRemoved']+=1
 if nss.exists():shutil.rmtree(nss);report['nssDatabasesRemoved']+=1
 report['fixtures'][name]={'hostVisibleFixtureProcesses':0,'containersIndependentlyAbsent':True,'relayNaturallyClosed':True,'browserAndNativeClosed':True,'keysRemoved':len(keys),'nssRemoved':not nss.exists(),'rawLogsSdkArtifactsConfigAndPublicCertificatesKept':True,'retainedFixtureFiles':kept}
report['hostProcessChecksPassed']=True;(HERE/'owned-generated-material-cleanup.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'hostProcessChecksPassed':True,'keysRemoved':report['keysRemoved'],'nssDatabasesRemoved':report['nssDatabasesRemoved']}))
