import datetime
import hashlib
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path('/home/js/Git/miopunch')
sys.path.insert(0, str(ROOT / 'scripts'))
from browser_mqtt_trust import certificates, private_nss

phase = sys.argv[1]
assert phase in {'red', 'green'}
build = Path(os.environ.get('MIO76_ANSWER_BUILD_DIR', '/tmp/mio74-current-role-positive-build-20261005'))
pkg = build / 'pkg'
runner = Path('/tmp/mio76-create-answer-startup.mjs')
output = Path('/tmp/mio76-create-answer-startup-' + phase + '-20261005')
output.mkdir(mode=0o700, exist_ok=False)
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
report = {'scope': 'actual owned Chromium SDK startup createAnswer absence only; no Native member, physical/public/STUN run',
          'phase': phase, 'startedUtc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'commands': [], 'cleanup': [], 'runnerSha256': digest(runner), 'outerRunnerSha256': digest(Path(__file__)),
          'buildReceiptPath': str(build / 'receipt.json'), 'buildReceiptSha256': digest(build / 'receipt.json')}

def source_path(name):
    return Path(name.removeprefix('external:')) if name.startswith('external:') else ROOT / name

def sources():
    before = json.loads((build / 'sources-before.json').read_text())
    after = json.loads((build / 'sources-after.json').read_text())
    assert before == after
    actual = {name: digest(source_path(name)) for name in before}
    assert actual == before, 'current compiled source differs from the frozen startup artifact'
    return actual

def artifacts():
    receipt = json.loads((build / 'receipt.json').read_text())
    assert receipt['exit'] == 0 and receipt['sourceStable'] is True
    hashes = {name: digest(Path(name)) for name in receipt['artifacts']}
    assert all(hashes[name] == value['sha256'] for name, value in receipt['artifacts'].items())
    return hashes

report['sourceBefore'] = sources()
report['artifactBefore'] = artifacts()
report['compiledSourceInputCount'] = len(report['sourceBefore'])
report['selectedDependenciesPath'] = str(build / 'selected-dependencies.json')
report['selectedDependenciesSha256'] = digest(build / 'selected-dependencies.json')
report['selectedCounts'] = json.loads((build / 'receipt.json').read_text())['selectedCounts']
helpers = [ROOT / 'scripts/browser_mqtt_trust.py', ROOT / 'scripts/mosquitto-test.conf',
           ROOT / 'browser-probe/harness/owned-mixed-mqtt-proxy.mjs', ROOT / 'browser-probe/harness/rtc-capability-fixture.mjs']
report['helperBefore'] = {str(path): digest(path) for path in helpers}
broker = process = fixture_path = None

def run(command, timeout=15):
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    report['commands'].append({'command': command, 'exit': result.returncode, 'seconds': time.monotonic() - started,
                               'stdout': result.stdout, 'stderr': result.stderr})
    result.check_returncode()
    return result.stdout.strip()

try:
    with tempfile.TemporaryDirectory(prefix='mio76-create-answer-private-') as directory:
        fixture_path = Path(directory)
        ca, leaf, key = certificates(fixture_path)
        private_nss(fixture_path / 'nssdb', ca)
        broker = 'miopunch-mio76-answer-' + str(time.time_ns())
        run(['rtk', 'proxy', 'docker', 'run', '--pull', 'never', '--detach', '--name', broker,
             '--publish', '127.0.0.1::1883', '--mount', f'type=bind,src={ROOT / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly',
             'eclipse-mosquitto:2'])
        address = run(['rtk', 'proxy', 'docker', 'port', broker, '1883/tcp'])
        host, port = address.rsplit(':', 1)
        ready_by = time.monotonic() + 5
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=.5):
                    break
            except OSError:
                if time.monotonic() >= ready_by:
                    raise
                time.sleep(.05)
        command = ['rtk', 'proxy', 'node', str(runner), '--fixture', str(fixture_path), '--output', str(output),
                   '--package', str(pkg), '--broker', address]
        started = time.monotonic()
        with (output / 'run.log').open('w') as log:
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    code = process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    code = process.wait()
                    report['forcedOuterKill'] = True
        report['commands'].append({'command': command, 'exit': code, 'seconds': time.monotonic() - started})
        report['runnerExit'] = code
except Exception as error:
    report['failure'] = {'class': type(error).__name__, 'observation': str(error)}
finally:
    if broker:
        removal = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', broker],
                                 cwd=ROOT, capture_output=True, text=True, timeout=15)
        report['cleanup'].append({'ownedBrokerRemovalExit': removal.returncode})
    report['cleanup'].append({'ownedPrivateTrustAndProfileDirectoryRemoved': fixture_path is not None and not fixture_path.exists(),
                              'globalTrustModified': False})
    report['artifactAfter'] = artifacts()
    report['sourceAfter'] = sources()
    report['helperAfter'] = {str(path): digest(path) for path in helpers}
    report['artifactsStable'] = report['artifactBefore'] == report['artifactAfter']
    report['sourcesStable'] = report['sourceBefore'] == report['sourceAfter']
    report['helpersStable'] = report['helperBefore'] == report['helperAfter']
    report['completedUtc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (output / 'outer-receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'output': str(output), 'runnerExit': report.get('runnerExit'),
                      'failure': report.get('failure'), 'artifactsStable': report['artifactsStable'],
                      'sourcesStable': report['sourcesStable']}))
raise SystemExit(report.get('runnerExit', 1))
