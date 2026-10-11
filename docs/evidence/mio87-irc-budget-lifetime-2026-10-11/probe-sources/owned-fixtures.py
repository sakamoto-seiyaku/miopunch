import os, sys, json, subprocess, socket, time, hashlib
from pathlib import Path
os.umask(0o077)
ROOT=Path('/home/js/Git/miopunch');P=Path(__file__).parent
sys.path.insert(0,str(ROOT/'scripts'))
from browser_mqtt_trust import certificates, private_nss
F=P/'private-fixture';F.mkdir(mode=0o700)
containers=[];r={'publicTraffic':False,'scope':'actual current SDK short caller, maintained listener beyond profile age, explicit cancellation, disabled IRC no I/O, and unaffected MQTT restricted request','passed':False,'cases':{},'cleanup':[]}
def cmd(*args, timeout=15):
    return subprocess.run(['rtk','proxy',*args],capture_output=True,text=True,check=True,timeout=timeout).stdout.strip()
def ready_port(name,port):
    address=cmd('docker','port',name,str(port)+'/tcp')
    host,n=address.rsplit(':',1)
    deadline=time.monotonic()+10
    while True:
        try:
            with socket.create_connection((host,int(n)),timeout=1):return address
        except OSError:
            if time.monotonic()>deadline:raise
            time.sleep(.1)
try:
    ergo='ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7'
    mqtt='eclipse-mosquitto:2'
    # Missing image fails before Docker run; never implicitly pull a different version.
    r['images']={name:json.loads(cmd('docker','image','inspect',name))[0]['Id'] for name in [ergo,mqtt]}
    ca,leaf,key=certificates(F);private_nss(F/'nssdb',ca)
    r['ownedCaSha256']=hashlib.sha256(ca.read_bytes()).hexdigest()
    config=(ROOT/'tests/fixtures/irc/ergo.yaml').read_text()
    assert 'fakelag:\n  enabled: false' in config
    config=config.replace('    :6667: {}','    :6697:\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key\n    :8097:\n      websocket: true\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key').replace('path: ircd.db','path: /tmp/irc-owned.db').replace('motd: ergo.motd','motd: /ircd-bin/ergo.motd')
    (F/'ergo.yaml').write_text(config)
    F.chmod(0o755)
    # Only local fixture TLS files must be readable by the container's user.
    for f in [F/'ergo.yaml',leaf,key]:f.chmod(0o644)
    name='miopunch-m86-irc-'+str(time.time_ns());containers.append(name)
    cmd('docker','run','--detach','--name',name,'--publish','127.0.0.1::6697','--publish','127.0.0.1::8097','--mount',f'type=bind,src={F},target=/fixture,readonly','--entrypoint','/ircd-bin/ergo',ergo,'run','--conf','/fixture/ergo.yaml')
    native=ready_port(name,6697).rsplit(':',1)[1];wss=ready_port(name,8097).rsplit(':',1)[1]
    (P/'local-facility.json').write_text(json.dumps({'logical_id':'m86-owned-irc','native_url':f'ircs://localhost:{native}/','wss_url':f'wss://localhost:{wss}/webirc','channel':'#m86-owned'}))
    name='miopunch-m86-mqtt-'+str(time.time_ns());containers.append(name)
    cmd('docker','run','--detach','--name',name,'--publish','127.0.0.1::1883','--mount',f'type=bind,src={ROOT}/scripts/mosquitto-test.conf,target=/mosquitto/config/mosquitto.conf,readonly',mqtt)
    (P/'local-broker.json').write_text(json.dumps({'address':ready_port(name,1883)}))
    for mode in ['invalid','short','aged','cancel','disabled','mqtt']:
        env=dict(os.environ);env['MIO87_REGRESSION']=mode
        v=subprocess.run(['rtk','proxy','node',str(P/'run.mjs')],cwd=ROOT,env=env,timeout=300)
        report=json.loads((P/'runtime'/mode/'private-runtime-receipt.json').read_text())
        r['cases'][mode]={'runtimeExit':v.returncode,'passed':report['qualified'],'currentStrictPurpose2Qualified':report.get('currentStrictPurpose2Qualified'),'nativeRequestAccepted':report.get('nativeRequestAccepted'),'callerElapsedMs':report.get('request',{}).get('elapsedMs'),'requestFailure':report.get('request',{}).get('failure'),'cleanup':{k:a for k,a in report['cleanup'].items() if k!='nativeClosedReceipt'}}
    r['passed']=all(v['passed'] for v in r['cases'].values())
except Exception as e:
    r['failure']=str(e)
finally:
    for name in reversed(containers):
        try:
            (P/(name+'.log')).write_text(cmd('docker','logs',name,timeout=3))
        except Exception:pass
        try:
            cmd('docker','rm','--force',name,timeout=5)
            exists=subprocess.run(['rtk','proxy','docker','inspect',name],capture_output=True).returncode==0
            r['cleanup'].append({'container':name,'removed':not exists})
        except Exception as e:r['cleanup'].append({'container':name,'removed':False,'error':str(e)})
    for f in F.glob('*.key'):f.chmod(0o600)
    F.chmod(0o700)
    (P/'runtime').mkdir(exist_ok=True)
    (P/'runtime/private-runtime-receipt.json').write_text(json.dumps(r,indent=2))
    print(json.dumps(r))
raise SystemExit(0 if r['passed'] and all(v['removed'] for v in r['cleanup']) else 1)
