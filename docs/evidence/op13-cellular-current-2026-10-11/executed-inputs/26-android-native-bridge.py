import json,os,sys,threading,time,hashlib
from pathlib import Path
os.umask(0o077);sys.dont_write_bytecode=True
ROOT=Path('/home/js/Git/miopunch');HERE=Path(__file__).resolve().parent;P=HERE.parent
sys.path.insert(0,str(ROOT/'scripts'))
import android_pair_runtime_current as runtime
cfg=Path(sys.argv[1]);config=json.loads(cfg.read_text());phase='reopened' if config.get('reopen') else 'initial'
raw=json.loads((P/'irc-android/receipt.json').read_text());deploy=raw['deployment']['irc_current_native'];binary=Path(deploy['deployedPath'])
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(binary)==deploy['deployedSha256']
device=runtime.device_for('native');actor=None;cleanup={};events=[];failure=None;code=1
try:
 actor=runtime.NativeActor(device);actor.deploy(binary,'irc-phone',executable=True);remote_cfg=actor.deploy(cfg,'config.json')
 store=json.loads((HERE/'runtime-private-bindings.json').read_text())['remoteStore'];assert store.startswith('/data/local/tmp/miopunch-irc-')
 actor.start([remote_cfg,'-',store])
 def input_loop():
  for line in sys.stdin:
   try:actor.send(line.rstrip('\n'))
   except (OSError,RuntimeError):return
  try:actor.process.stdin.close()
  except OSError:pass
 thread=threading.Thread(target=input_loop,daemon=True);thread.start()
 while True:
  with actor.condition:
   pending=list(actor.events);actor.events.clear()
  for value in pending:
   events.append(value);print(json.dumps(value),flush=True)
  if actor.process.poll() is not None:
   for reader in actor.threads:reader.join(timeout=1)
   with actor.condition:pending=list(actor.events);actor.events.clear()
   for value in pending:events.append(value);print(json.dumps(value),flush=True)
   code=actor.process.returncode;break
  time.sleep(.02)
except Exception as error:
 failure={'class':type(error).__name__,'message':str(error)}
finally:
 if actor:
  cleanup=actor.close(graceful_seconds=130)
  (HERE/('private-native-bridge-'+phase+'.json')).write_text(json.dumps({'failure':failure,'cleanup':cleanup,'events':events,'stderr':actor.stderr,'actorDirectory':actor.directory,'runs':actor.runs},indent=2)+'\n')
  if cleanup.get('forced') or not cleanup.get('ownedRemoteProcessAbsentLate') or not cleanup.get('ownedDirectoryRemoved'):code=1
 if failure:code=1
raise SystemExit(code)
