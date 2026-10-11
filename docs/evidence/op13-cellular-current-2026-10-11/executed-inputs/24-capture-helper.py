"""Own device-side capture processes for one bounded SDK story."""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import threading
import time
from urllib.parse import urlsplit
import uuid

BINARY = Path('/tmp/mio82-android-capture-tools-root-20261007/tcpdump-arm64')
BINARY_SHA = 'a052fce9869718e6c66e1740ad012717c9c768dfbb7ae29b2af7aef504a49809'


class DeviceCapture:
    def __init__(self, role, device, output, interface):
        self.role, self.device, self.output, self.interface = role, device, output, interface
        self.remote = '/data/local/tmp/mio82-pcap-' + uuid.uuid4().hex
        self.records, self.processes = [], []
        self.output.mkdir(mode=0o700, parents=True)
        self.command_index = 0
        self.made = False
        assert hashlib.sha256(BINARY.read_bytes()).hexdigest() == BINARY_SHA
        try:
            self.command('mkdir', ['shell', 'mkdir', '-m', '700', self.remote])
            self.made = True
            self.command('push', ['push', str(BINARY), self.remote + '/tcpdump'])
            self.command('chmod', ['shell', 'chmod', '700', self.remote + '/tcpdump'])
            digest = self.command('sha', ['shell', 'sha256sum', self.remote + '/tcpdump'])
            assert digest.stdout.decode().split()[0] == BINARY_SHA
        except Exception:
            if self.made:
                self.close()
            raise

    def command(self, label, args, *, required=True, timeout=3):
        argv = ['rtk', 'proxy', self.device.adb, '-s', self.device._serial, *args]
        self.command_index += 1
        started = time.monotonic()
        r = subprocess.run(argv, capture_output=True, timeout=timeout)
        prefix = self.output / f'{self.command_index:03d}-{label}'
        prefix.with_suffix('.stdout').write_bytes(r.stdout)
        prefix.with_suffix('.stderr').write_bytes(r.stderr)
        prefix.with_suffix('.json').write_text(json.dumps({'argv': argv, 'exit': r.returncode,
                                                          'elapsedMs': (time.monotonic() - started) * 1000}) + '\n')
        if required and r.returncode:
            raise RuntimeError('owned capture command failed: ' + label)
        return r

    def root(self, label, words, **kwargs):
        return self.command(label, ['shell', shlex.join(['su', '-c', shlex.join(words)])], **kwargs)

    def start(self, name, expression):
        assert len(self.processes) < 3
        pcap = self.remote + '/' + name + '.pcap'
        pidfile = self.remote + '/' + name + '.pid'
        binary = self.remote + '/tcpdump'
        words = [binary, '-n', '-U', '-i', self.interface, '-s', '0', '-c', '20000', '-w', pcap, expression]
        command = 'umask 077; echo $$ > ' + shlex.quote(pidfile) + '; exec ' + shlex.join(words)
        argv = ['rtk', 'proxy', self.device.adb, '-s', self.device._serial, 'shell', shlex.join(['su', '-c', command])]
        process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        record = {'name': name, 'argv': argv, 'filter': expression, 'interface': self.interface,
                  'startedBeforeSdk': name == 'pre-sdk-udp-and-mqtt', 'remotePcap': pcap,
                  'ready': False, 'forced': False, 'limitPackets': 20000,
                  'startRootMonotonic': time.monotonic(), 'lines': []}
        ready = threading.Event()
        def read(stream, which):
            for raw in stream:
                text = raw.decode(errors='replace')[:4096]
                if len(record['lines']) < 128:
                    record['lines'].append({'stream': which, 'text': text})
                if 'listening on' in text:
                    record['ready'] = True
                    ready.set()
        readers = [threading.Thread(target=read, args=(process.stdout, 'stdout'), daemon=True),
                   threading.Thread(target=read, args=(process.stderr, 'stderr'), daemon=True)]
        for reader in readers:
            reader.start()
        owned = {'process': process, 'record': record, 'readers': readers,
                 'pidfile': pidfile, 'binary': binary, 'pcap': pcap, 'pid': None, 'startTime': None}
        self.processes.append(owned)
        self.records.append(record)
        assert ready.wait(5), 'capture did not become ready before SDK operation'
        assert process.poll() is None
        pid = int(self.root(name + '-pid', ['cat', pidfile]).stdout.decode().strip())
        cmd = self.root(name + '-cmdline', ['cat', f'/proc/{pid}/cmdline']).stdout.decode().split('\0')
        assert cmd[0] == binary and pcap in cmd
        stat = self.root(name + '-stat', ['cat', f'/proc/{pid}/stat']).stdout.decode()
        start_time = stat[stat.rfind(')') + 2:].split()[19]
        owned.update(pid=pid, startTime=start_time)
        record.update(pid=pid, processStartTime=start_time, exactOwnedCommandLineVerified=True)

    def close(self):
        failures = []
        for owned in self.processes:
            p, record, pid = owned['process'], owned['record'], owned['pid']
            try:
                if pid is None:
                    candidate = self.root(record['name'] + '-late-pid', ['cat', owned['pidfile']], required=False)
                    if candidate.returncode == 0 and candidate.stdout.decode().strip().isdigit():
                        pid = int(candidate.stdout.decode().strip())
                        owned['pid'] = pid
                        if p.poll() is None:
                            stat = self.root(record['name'] + '-late-stat', ['cat', f'/proc/{pid}/stat']).stdout.decode()
                            owned['startTime'] = stat[stat.rfind(')') + 2:].split()[19]
                if p.poll() is None and pid is not None:
                    current = self.root(record['name'] + '-stop-cmdline', ['cat', f'/proc/{pid}/cmdline']).stdout.decode().split('\0')
                    stat = self.root(record['name'] + '-stop-stat', ['cat', f'/proc/{pid}/stat']).stdout.decode()
                    assert current[0] == owned['binary'] and owned['pcap'] in current
                    assert stat[stat.rfind(')') + 2:].split()[19] == owned['startTime']
                    self.root(record['name'] + '-sigint', ['kill', '-INT', str(pid)])
                try:
                    record['exit'] = p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    record['forced'] = True
                    if pid is not None:
                        self.root(record['name'] + '-sigterm', ['kill', '-TERM', str(pid)], required=False)
                    record['exit'] = p.wait(timeout=3)
                for reader in owned['readers']:
                    reader.join(timeout=1)
                record['readersEnded'] = all(not v.is_alive() for v in owned['readers'])
                if record['readersEnded']:
                    p.stdout.close()
                    p.stderr.close()
                    record['localPipesClosed'] = True
                immediate = self.root(record['name'] + '-absence-immediate', ['test', '!', '-e', f'/proc/{pid}'], required=False)
                time.sleep(0.1)
                late = self.root(record['name'] + '-absence-late', ['test', '!', '-e', f'/proc/{pid}'], required=False)
                record.update(ownedProcessAbsentImmediate=immediate.returncode == 0,
                              ownedProcessAbsentLate=late.returncode == 0)
                raw = self.root(record['name'] + '-read-pcap', ['cat', owned['pcap']], timeout=10)
                target = self.output / (record['name'] + '.pcap')
                target.write_bytes(raw.stdout)
                record.update(pcapBytes=len(raw.stdout), pcapSha256=hashlib.sha256(raw.stdout).hexdigest())
                text = ''.join(v['text'] for v in record['lines'])
                for key, pattern in [('packetsCaptured', r'(\d+) packets captured'),
                                     ('packetsReceivedByFilter', r'(\d+) packets received by filter'),
                                     ('kernelDrops', r'(\d+) packets dropped by kernel')]:
                    match = re.search(pattern, text)
                    record[key] = int(match.group(1)) if match else None
                record['limitReached'] = record.get('packetsCaptured') == record['limitPackets']
                record['cleanupSucceeded'] = (record['exit'] == 0 and not record['forced']
                    and record['readersEnded'] and record['ownedProcessAbsentImmediate']
                    and record['ownedProcessAbsentLate'] and record.get('kernelDrops') == 0
                    and not record['limitReached'])
            except Exception as error:
                failures.append(type(error).__name__)
                record['cleanupSucceeded'] = False
        if self.made and all(v.get('ownedProcessAbsentLate') for v in self.records):
            self.root('remove-owned-capture-directory', ['rm', '-rf', self.remote])
            absent = self.root('owned-capture-directory-absence', ['test', '!', '-e', self.remote])
            directory_absent = absent.returncode == 0
        else:
            directory_absent = False
        value = {'role': self.role, 'records': self.records, 'failures': failures,
                 'ownedDirectoryAbsent': directory_absent,
                 'cleanupSucceeded': directory_absent and not failures
                    and all(v.get('cleanupSucceeded') for v in self.records)}
        (self.output / 'private-capture-receipt.json').write_text(json.dumps(value, indent=2) + '\n')
        return value


class CaptureGroup:
    def __init__(self, output, facility, native_profile):
        self.output, self.facility, self.profile = Path(output), facility, native_profile
        self.devices = {}

    def start_device(self, role, device):
        result, record = device.shell('dumpsys', 'connectivity')
        from android_pair_route_receipt_current import _agent_facts
        facts = _agent_facts(result)
        assert facts['activeDefaultObserved'] and facts['defaultAgentObserved']
        assert facts['defaultNotVpnCapability'] and facts['currentVpnAgentCount']==0
        assert facts['defaultTransport'] in ['CELLULAR','WIFI'] and len(facts['interfaces'])==1
        interface=facts['interfaces'][0]
        self.output.mkdir(mode=0o700,parents=True,exist_ok=True)
        safe={k:v for k,v in facts.items() if k not in ['selectedAgent','interfaces']}
        (self.output/(role+'-network-facts-before-sdk.json')).write_text(json.dumps(safe,indent=2)+'\n')
        assert re.fullmatch(r'[a-zA-Z0-9_.:-]{1,32}', interface)
        cap = DeviceCapture(role, device, self.output / role, interface)
        self.devices[role] = cap
        port = urlsplit(self.facility['native_url' if role == 'native' else 'wss_url']).port
        cap.start('pre-sdk-udp-and-mqtt', f'udp or (tcp and port {port})')

    def observe(self, actor, page):
        # No monitor, SDK operation, socket or periodic poll is needed. Both
        # bounded default-interface UDP captures were ready before SDK startup.
        return None

    def snapshot(self):
        values = {}
        for role, cap in self.devices.items():
            owned = cap.processes[0]
            assert owned['process'].poll() is None
            current = cap.root('snapshot-cmdline', ['cat', f"/proc/{owned['pid']}/cmdline"]).stdout.decode().split('\0')
            assert current[0] == owned['binary'] and owned['pcap'] in current
            stat = cap.root('snapshot-stat', ['cat', f"/proc/{owned['pid']}/stat"]).stdout.decode()
            assert stat[stat.rfind(')') + 2:].split()[19] == owned['startTime']
            result = cap.root('live-snapshot', ['cat', owned['pcap']], timeout=10)
            path = cap.output / 'live-sdk-pair-snapshot.pcap'
            assert not path.exists()
            path.write_bytes(result.stdout)
            values[role] = {'path': str(path), 'sha256': hashlib.sha256(result.stdout).hexdigest()}
        return values

    def close(self):
        values = {role: cap.close() for role, cap in self.devices.items()}
        value = {'devices': values, 'observerFailures': [], 'observerJoined': True,
                 'initialCaptureScope': 'bounded UDP and configured MQTT TLS on each observed default NOT_VPN interface before SDK; validation status recorded separately; ownership is established only for exact SDK-selected tuples afterward',
                 'firstPeerCheckCoveredByExpandedFilterProven': False,
                 'allCapturedPacketsOwnedBySdkProven': False,
                 'noGlobalNetworkOrTrustSettingsChanged': True,
                 'cleanupSucceeded': all(v.get('cleanupSucceeded') for v in values.values())}
        self.output.mkdir(mode=0o700, parents=True, exist_ok=True)
        (self.output / 'private-group-receipt.json').write_text(json.dumps(value, indent=2) + '\n')
        return value
