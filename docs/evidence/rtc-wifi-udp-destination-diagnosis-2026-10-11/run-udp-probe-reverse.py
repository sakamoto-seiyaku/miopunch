import hashlib
import json
import os
from pathlib import Path
import sys
import uuid

os.umask(0o077)
sys.path.insert(0,'/home/js/Git/miopunch/scripts')
from android_pair_runtime_current import device_for
HERE=Path(__file__).resolve().parent
binary=HERE/'udp-destination-probe'
expected=json.loads((HERE/'udp-probe-compile.json').read_text())['artifactSha256']
assert hashlib.sha256(binary.read_bytes()).hexdigest()==expected
result={}
for role in ('native','browser'):
    device=device_for(role)
    directory='/data/local/tmp/mio84-udp-diagnosis-'+uuid.uuid4().hex
    remote=directory+'/probe'
    row={}
    try:
        device.shell('mkdir','-m','700',directory)
        device.run('push',str(binary),remote)
        device.shell('chmod','700',remote)
        out,code=device.shell('sha256sum',remote)
        assert code==0 and out.split()[0]==expected
        out,code=device.shell(remote,'54.65.63.236','3478','162.159.207.0','3478')
        (HERE/('private-udp-probe-reverse-'+role+'.jsonl')).write_text(out)
        row['runtimeExit']=code
        assert code==0
        row['observations']=[{k:v for k,v in json.loads(line).items() if k not in ('source','sourcePort')} for line in out.splitlines()]
    finally:
        _,cleanup=device.shell('rm','-r',directory,required=False)
        _,absent=device.shell('test','-e',directory,required=False)
        row['ownedDirectoryRemoved']=cleanup==0 and absent==1
        result[role]=row
        (HERE/'udp-probe-result-reverse.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
