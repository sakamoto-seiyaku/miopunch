#!/usr/bin/env python3
import argparse,base64,json,subprocess,shlex,time
from pathlib import Path

parser=argparse.ArgumentParser(description='Run one fixed-device MIOPUNCH-138 relay comparison; nodes must already be admitted and the actor deployed.')
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--case',required=True,choices=[d+'-'+c for d in ['windows-to-linux','linux-to-windows'] for c in ['normal-auto','relay-auto','relay-fixed','relay-fixed-early']])
args=parser.parse_args()
OUT=args.output/args.case;OUT.mkdir(parents=True,exist_ok=False)
WINDOWS_EXE='C:/Users/zy/miopunch-network-demo/miopunch-network-demo.exe'
WINDOWS_SERVER=None
LIN='/tmp/miopunch138'
WIN='C:/Users/zy/miopunch138'
CONTAINER='miopunch-network-demo'
RELAYS=['https://'+region+'-1.relay.n0.iroh.link./' for region in ['aps1','euc1','use1','usw1']]

def run(args,timeout=120,check=True):
 return subprocess.run(['rtk','proxy',*map(str,args)],capture_output=True,text=True,errors="replace",timeout=timeout,check=check)
def ssh(host,args,**kw):
 return run(['ssh','-o','BatchMode=yes',host,shlex.join(list(map(str,args)))],**kw)
def linux(args,**kw):
 return ssh('r_proxy',['docker','exec',CONTAINER,*args],**kw)
def powershell(script,**kw):
 return ssh('u22sz',['powershell','-NoProfile','-EncodedCommand',base64.b64encode(('$ProgressPreference="SilentlyContinue"\n'+script).encode('utf-16le')).decode()],**kw)
def put(side,local,name):
 if side=='linux':
  run(['scp','-q',local,'r_proxy:'+LIN+'/'+name])
  ssh('r_proxy',['docker','cp',LIN+'/'+name,CONTAINER+':'+LIN+'/'+name])
 else: run(['scp','-q',local,'u22sz:'+WIN+'/'+name])
def get(side,name,local):
 if side=='linux':
  ssh('r_proxy',['docker','cp',CONTAINER+':'+LIN+'/'+name,LIN+'/'+name])
  run(['scp','-q','r_proxy:'+LIN+'/'+name,local])
 else: run(['scp','-q','u22sz:'+WIN+'/'+name,local])
def config(side,mode,name,relay_only,home,early):
 root=LIN if side=='linux' else WIN
 return {'mode':mode,'directory':root+'/node','target_file':root+'/'+name+'.target.json','relays':RELAYS,'home':home,
  'relay_only':relay_only,'early_ms':early,'connect_ms':30000,'lifetime_ms':90000,'bind_port':47138}
def start_server(side,name):
 global WINDOWS_SERVER
 if side=='linux':
  linux(['rm','-f',LIN+'/'+name+'.target.json',LIN+'/'+name+'.target.stop'])
  command=f'echo $$ > {LIN}/{name}.pid; exec env MIO138_ACTOR={LIN}/{name}.json {LIN}/relay-probe --exact relay_probe_actor --nocapture > {LIN}/{name}.stdout.log 2> {LIN}/{name}.stderr.log'
  ssh('r_proxy',['docker','exec','-d',CONTAINER,'sh','-c',command])
 else:
  powershell(f"Remove-Item -LiteralPath '{WIN}/{name}.target.json','{WIN}/{name}.target.stop' -Force -ErrorAction SilentlyContinue")
  script=f"$env:MIO138_ACTOR='{WIN}/{name}.json'\n$p=Start-Process -FilePath '{WINDOWS_EXE}' -ArgumentList @('--exact','relay_probe_actor','--nocapture') -PassThru -RedirectStandardOutput '{WIN}/{name}.stdout.log' -RedirectStandardError '{WIN}/{name}.stderr.log'\n$null=$p.Handle\n[IO.File]::WriteAllText('{WIN}/{name}.pid',[string]$p.Id)\n$p.WaitForExit()\nexit $p.ExitCode"
  encoded=base64.b64encode(('$ProgressPreference="SilentlyContinue"\n'+script).encode('utf-16le')).decode()
  command=['rtk','proxy','ssh','-T','-o','BatchMode=yes','u22sz',shlex.join(['powershell','-NoProfile','-EncodedCommand',encoded])]
  with (OUT/(name+'-server-session.log')).open('wb') as log:
   WINDOWS_SERVER=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT)

def ready(side,name):
 if side=='linux': return linux(['test','-s',LIN+'/'+name+'.target.json'],check=False).returncode==0
 return 'True' in powershell(f"Test-Path '{WIN}/{name}.target.json'").stdout
def client(side,name):
 if side=='linux': return linux(['env','MIO138_ACTOR='+LIN+'/'+name+'.json',LIN+'/relay-probe','--exact','relay_probe_actor','--nocapture'])
 return powershell(f"$env:MIO138_ACTOR='{WIN}/{name}.json'\n$p=Start-Process -FilePath '{WINDOWS_EXE}' -ArgumentList @('--exact','relay_probe_actor','--nocapture') -Wait -PassThru -RedirectStandardOutput '{WIN}/{name}.stdout.log' -RedirectStandardError '{WIN}/{name}.stderr.log'\nexit $p.ExitCode")
def stop(side,name):
 if side=='linux':
  linux(['sh','-c',f'printf x > {LIN}/{name}.target.stop'])
  for _ in range(40):
   r=linux(['sh','-c',f'kill -0 $(cat {LIN}/{name}.pid) 2>/dev/null'],check=False)
   if r.returncode: return
   time.sleep(.25)
  raise RuntimeError('owned Linux server did not stop')
 else:
  powershell(f"[IO.File]::WriteAllText('{WIN}/{name}.target.stop','done')")
  if WINDOWS_SERVER is None: raise RuntimeError('no owned Windows server session')
  code=WINDOWS_SERVER.wait(timeout=15)
  if code: raise RuntimeError('Windows server did not exit successfully: '+str(code))


results=[]
for direction,server_side,client_side in [('windows-to-linux','linux','windows'),('linux-to-windows','windows','linux')]:
 for condition,relay_only,home,early in [
  ('normal-auto',False,None,None),('relay-auto',True,None,None),('relay-fixed',True,RELAYS[0],None),('relay-fixed-early',True,RELAYS[0],250)]:
  selected=direction+'-'+condition
  if selected!=args.case: continue
  name='final-'+selected
  print(json.dumps({'event':'case-start','case':name,'at':time.time()}),flush=True)
  for side,mode in [(server_side,'serve'),(client_side,'connect')]:
   path=OUT/(name+'-'+side+'.json');path.write_text(json.dumps(config(side,mode,name,relay_only,home,early)))
   put(side,path,name+'.json')
  start_server(server_side,name)
  try:
   limit=time.monotonic()+20
   while not ready(server_side,name):
    if time.monotonic()>=limit: raise RuntimeError('server target not published: '+name)
    time.sleep(.25)
   target=OUT/(name+'.target.json');get(server_side,name+'.target.json',target);put(client_side,target,name+'.target.json')
   r=client(client_side,name)
   if client_side=='linux':
    (OUT/(name+'-client.stdout.log')).write_text(r.stdout)
    (OUT/(name+'-client.stderr.log')).write_text(r.stderr)
   else:
    get(client_side,name+'.stdout.log',OUT/(name+'-client.stdout.log'))
    get(client_side,name+'.stderr.log',OUT/(name+'-client.stderr.log'))
  finally:
   stop(server_side,name)
   get(server_side,name+'.stdout.log',OUT/(name+'-server.stdout.log'))
   get(server_side,name+'.stderr.log',OUT/(name+'-server.stderr.log'))
  events=[]
  for line in (OUT/(name+'-client.stdout.log')).read_text().splitlines():
   if line.startswith('{'):
    value=json.loads(line)
    if value.get('event')=='mio138-result': events.append(value['outcome'])
  if len(events)!=1: raise RuntimeError('missing definitive client result: '+name)
  result={'case':name,'condition':condition,'direction':direction,'connectBudgetMs':30000,'requestBudgetsMs':[3000,10000],
   'permissions':RELAYS,'home':home,'earlyMs':early,'clientOutcome':events[0]}
  results.append(result);(OUT/'results.json').write_text(json.dumps(results,indent=2))
  print(json.dumps({'event':'case-result',**result}),flush=True)
