#!/usr/bin/env python3
"""MIOPUNCH-41: isolated NAT-PMP wire service backed by actual kernel DNAT."""
import argparse
import json
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import tempfile
import shutil
import time


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def gateway(directory):
    """Controlled RFC 6886 server; responses follow installed forwarding rules."""
    directory = Path(directory)
    log = (directory / 'gateway.jsonl').open('a', buffering=1)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(('10.41.0.1', 5351))
    active = {}
    start = time.monotonic()
    last_mode = "normal"
    log.write(json.dumps({'gateway_started': True, 'pid': os.getpid(), 'epoch': 0}) + '\n')

    def rule(action, ip, internal, protocol, external):
        run('iptables', '-t', 'nat', action, 'PREROUTING', '-i', 'wan0', '-p', protocol, '--dport', str(external), '-j', 'DNAT', '--to-destination', f'{ip}:{internal}')
        run('iptables', action, 'FORWARD', '-i', 'wan0', '-o', 'lan0', '-p', protocol, '-d', ip, '--dport', str(internal), '-j', 'ACCEPT')

    def remove(key):
        if key in active:
            external, _ = active.pop(key)
            rule('-D', *key, external)

    while not (directory / 'stop-gateway').exists():
        mode = (directory / 'gateway-mode').read_text().strip() if (directory / 'gateway-mode').exists() else 'normal'
        if mode != last_mode:
            if mode in ('fail', 'restart'):
                for key in list(active):
                    remove(key)
                log.write(json.dumps({'administrative_delete': True, 'mode': mode}) + '\n')
            if mode == 'restart':
                start = time.monotonic()
            last_mode = mode
        for key, (_, expiry) in list(active.items()):
            if time.monotonic() >= expiry:
                remove(key)
        if not select.select([sock], [], [], .05)[0]:
            continue
        packet, source = sock.recvfrom(2048)
        mode = (directory / 'gateway-mode').read_text().strip() if (directory / 'gateway-mode').exists() else 'normal'
        epoch = int(time.monotonic() - start)
        event = {'source': list(source), 'packet_hex': packet.hex(), 'mode': mode}
        if len(packet) < 2 or packet[0] != 0:
            event['rejected_protocol'] = True
        elif packet[1] == 0:
            if mode in ('allocation-timeout', 'allocation-cancel', 'renewal-address-failure'):
                (directory / 'address-seen').write_text('')
                event['address_reply_dropped'] = True
            else:
                sock.sendto(struct.pack('!BBHI4s', 0, 128, 0, epoch, socket.inet_aton('100.64.41.1')), source)
                event['public_address'] = '100.64.41.1'
        elif packet[1] in (1, 2) and len(packet) == 12:
            _, _, _, internal, preferred, lifetime = struct.unpack('!BBHHHI', packet)
            key = (source[0], internal, 'udp' if packet[1] == 1 else 'tcp')
            if lifetime == 0:
                remove(key)
                external, granted, result = 0, 0, 0
                event['released'] = list(key)
                if mode == 'renewal-address-failure':
                    (directory / 'mapping-cleaned').write_text('')
            elif mode == 'fail':
                # Actual forwarding is removed; a failed renewal cannot extend it.
                remove(key)
                external, granted, result = 0, 0, 3
            else:
                external = 42041 if mode in ('changed', 'restart') else 41041
                granted, result = (30 if mode.startswith("allocation-") or packet[1] == 1 else 6), 0
                if key in active and active[key][0] != external:
                    remove(key)
                if key not in active:
                    rule('-A', *key, external)
                active[key] = external, time.monotonic() + granted
            sock.sendto(struct.pack('!BBHIHHI', 0, 128 + packet[1], result, epoch, internal, external, granted), source)
            event.update(protocol=key[2], internal_port=internal, preferred_port=preferred, requested_lifetime=lifetime, external_port=external, granted_lifetime=granted, result=result)
        else:
            event['unsupported'] = True
        event['active'] = [{'owner': list(k), 'external_port': v[0]} for k, v in active.items()]
        log.write(json.dumps(event) + '\n')
    for key in list(active):
        remove(key)


def lab(binary, directory, case="positive"):
    evidence = Path(directory).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="mio41-private-"))
    prefix = f'm41-{os.getpid()}'
    names = {role: f'{prefix}-{role}' for role in ['gateway', 'node', 'remote']}
    jobs = []
    files = []
    passed = False

    def ns(role, *args, **kwargs):
        return run('ip', 'netns', 'exec', names[role], *args, **kwargs)

    def spawn(role, command, filename):
        out = (evidence / filename).open('w')
        files.append(out)
        proc = subprocess.Popen(['ip', 'netns', 'exec', names[role], *command], stdout=out, stderr=subprocess.STDOUT, text=True)
        jobs.append(proc)
        return proc

    def worker(role, namespace):
        command = ['env', f'MIO41_ROLE={role}', f'MIO41_DIRECTORY={directory}', f'MIO41_CASE={case}', binary, '--exact', 'gateway_node_worker', '--nocapture']
        return spawn(namespace, command, f'{role}.log')

    def wait_file(name, process, seconds=12):
        deadline = time.monotonic() + seconds
        while not (directory / name).exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f'{name} did not appear; worker status={process.poll()}')
            time.sleep(.05)

    try:
        for name in names.values():
            run('ip', 'netns', 'add', name)
            run('ip', '-n', name, 'link', 'set', 'lo', 'up')
        for left, li, la, right, ri, ra in [('gateway', 'lan0', '10.41.0.1/24', 'node', 'eth0', '10.41.0.2/24'), ('gateway', 'wan0', '100.64.41.1/24', 'remote', 'eth0', '100.64.41.2/24')]:
            run('ip', 'link', 'add', 'm41-left', 'type', 'veth', 'peer', 'name', 'm41-right')
            for link, role, interface, address in [('m41-left', left, li, la), ('m41-right', right, ri, ra)]:
                run('ip', 'link', 'set', link, 'netns', names[role])
                run('ip', '-n', names[role], 'link', 'set', link, 'name', interface)
                run('ip', '-n', names[role], 'addr', 'add', address, 'dev', interface)
                run('ip', '-n', names[role], 'link', 'set', interface, 'up')
        run('ip', '-n', names['node'], 'route', 'add', 'default', 'via', '10.41.0.1')
        ns('gateway', 'sysctl', '-qw', 'net.ipv4.ip_forward=1')
        ns('gateway', 'iptables', '-P', 'FORWARD', 'DROP')
        ns('gateway', 'iptables', '-A', 'FORWARD', '-i', 'lan0', '-o', 'wan0', '-j', 'ACCEPT')
        ns('gateway', 'iptables', '-A', 'FORWARD', '-m', 'conntrack', '--ctstate', 'ESTABLISHED,RELATED', '-j', 'ACCEPT')
        ns('gateway', 'iptables', '-t', 'nat', '-A', 'POSTROUTING', '-o', 'wan0', '-j', 'MASQUERADE')
        # A foreign owner's permanent rule is never touched by the node.
        ns('gateway', 'iptables', '-t', 'nat', '-A', 'PREROUTING', '-i', 'wan0', '-p', 'tcp', '--dport', '49041', '-j', 'DNAT', '--to-destination', '10.41.0.3:49041')
        ns('gateway', 'iptables', '-t', 'nat', '-A', 'PREROUTING', '-i', 'wan0', '-p', 'udp', '--dport', '49041', '-j', 'DNAT', '--to-destination', '10.41.0.3:49041')
        prepared = worker('prepare', 'remote')
        if prepared.wait(timeout=20) != 0:
            raise RuntimeError('prepare failed')
        if case.startswith('allocation-') or case == 'initial-failure':
            (directory / 'gateway-mode').write_text('fail' if case == 'initial-failure' else case)
        gateway_process = spawn('gateway', ['python3', __file__, 'gateway', str(directory)], 'gateway.log')
        spawn('gateway', ['tcpdump', '--immediate-mode', '-U', '-ni', 'any', '-w', str(evidence / 'gateway.pcap'), 'tcp or udp'], 'tcpdump.log')
        time.sleep(.15)
        if case.startswith('allocation-'):
            allocation = worker(case, 'node')
            if allocation.wait(timeout=8) != 0:
                raise RuntimeError('allocation failure/cancellation worker failed')
            time.sleep(.15)
            with (evidence / 'final-rules.txt').open('w') as out:
                ns('gateway', 'iptables-save', stdout=out)
            events = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
            assert any(event.get('granted_lifetime') == 30 for event in events), 'successful allocation must precede lost address reply'
            assert any(event.get('address_reply_dropped') for event in events)
            assert any(event.get('protocol') == 'tcp' and event.get('requested_lifetime') == 0 and event.get('released') for event in events), 'acknowledged allocation must be released on failure or cancellation before publication'
            rules = (evidence / 'final-rules.txt').read_text()
            assert '--dport 49041' in rules and '--dport 41041' not in rules, 'owned allocation must be removed, foreign owner retained'
            passed = True
            print(f'PASS {case} acknowledged allocation released evidence={evidence}')
            return
        if case in ('disabled', 'disabled-advanced', 'tcp-disabled-advanced', 'tcp-disabled'):
            disabled = worker(case, 'node')
            if disabled.wait(timeout=8) != 0:
                raise RuntimeError('disabled mapping worker failed')
            time.sleep(.1)
            events = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
            if case == 'tcp-disabled':
                assert any(event.get('protocol') == 'udp' and event.get('granted_lifetime') for event in events), 'UDP mapping consent remains independently enabled'
                assert not any(event.get('protocol') == 'tcp' for event in events), 'TCP disable must suppress TCP gateway requests'
            else:
                assert all(event.get('gateway_started') for event in events), 'disabled node must send no mapping protocol requests'
            passed = True
            print(f'PASS {case} ' + ('TCP disabled; UDP independently mapped' if case == 'tcp-disabled' else 'no gateway requests') + f' evidence={evidence}')
            return
        if case == 'close-isolation':
            sentinel = spawn('node', ['python3', __file__, 'udp-sentinel', str(directory)], 'udp-sentinel.log')
            wait_file('udp-mapped', sentinel)
        server = worker('serve', 'node')
        wait_file('mapped', server)
        client = worker('connect', 'gateway' if case == 'initial-failure' else 'remote')
        lifecycle = case in ('expired', 'changed', 'restart', 'renewed', 'renewal-address-failure')
        if lifecycle:
            wait_file('data-ready', server)
        elif client.wait(timeout=30) != 0:
            raise RuntimeError('authorized remote worker failed')
        if case == 'renewal-address-failure':
            wait_file('data-ready', server)
            (directory / 'gateway-mode').write_text(case)
            wait_file('withdrawn', server)
            probe = worker('unreachable', 'remote')
            if probe.wait(timeout=8) != 0:
                raise RuntimeError('cleaned renewal candidate allowed a fresh connection')
        if case == 'expired':
            wait_file('data-ready', server)
            (directory / 'gateway-mode').write_text('fail')
            wait_file('withdrawn', server)
            probe = worker('unreachable', 'remote')
            if probe.wait(timeout=8) != 0:
                raise RuntimeError('new connection incorrectly survived deletion')
            alternate = worker('fallback', 'gateway')
            if alternate.wait(timeout=8) != 0:
                raise RuntimeError('mapping failure blocked another allowed direct path')
        if case in ('changed', 'restart', 'renewed'):
            wait_file('data-ready', server)
            if case == 'restart':
                # Restart the real service and remove its volatile forwarding
                # state, while preserving the foreign owner's permanent rule.
                gateway_process.terminate()
                gateway_process.wait(timeout=2)
                previous = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
                allocation = next(event for event in previous if event.get('granted_lifetime') == 6)
                internal = allocation['internal_port']
                ns('gateway', 'iptables', '-t', 'nat', '-D', 'PREROUTING', '-i', 'wan0', '-p', 'tcp', '--dport', '41041', '-j', 'DNAT', '--to-destination', f'10.41.0.2:{internal}')
                ns('gateway', 'iptables', '-D', 'FORWARD', '-i', 'wan0', '-o', 'lan0', '-p', 'tcp', '-d', '10.41.0.2', '--dport', str(internal), '-j', 'ACCEPT')
                (directory / 'gateway-mode').write_text('restart')
                gateway_process = spawn('gateway', ['python3', __file__, 'gateway', str(directory)], 'gateway-restarted.log')
            elif case != 'renewed':
                (directory / 'gateway-mode').write_text(case)
            wait_file('renewed' if case == 'renewed' else 'changed', server)
            if case in ('changed', 'restart'):
                stale = worker('unreachable', 'remote')
                if stale.wait(timeout=8) != 0:
                    raise RuntimeError('old mapped port admitted a fresh connection after replacement')
            reconnect = worker('reconnect', 'remote')
            if reconnect.wait(timeout=8) != 0:
                raise RuntimeError('fresh connection to renewed/replaced candidate failed')
        if client.wait(timeout=20) != 0:
            raise RuntimeError('authorized remote worker failed')
        if server.wait(timeout=15) != 0:
            raise RuntimeError('mapped node worker failed')
        time.sleep(.15)
        if case == 'close-isolation':
            probe = spawn('remote', ['python3', __file__, 'udp-probe', str(directory)], 'udp-probe.log')
            if probe.wait(timeout=3) != 0:
                raise RuntimeError('TCP close deleted independent same-port UDP mapping')
        with (evidence / 'final-rules.txt').open('w') as out:
            ns('gateway', 'iptables-save', stdout=out)
        events = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
        if case == 'initial-failure':
            assert any(event.get('protocol') == 'tcp' and event.get('result') == 3 for event in events), 'real TCP allocation failure required'
            assert not any(event.get('protocol') == 'tcp' and event.get('granted_lifetime') for event in events)
        else:
            assert any(event.get('protocol') == 'tcp' and event.get('external_port') == 41041 and event.get('result') == 0 for event in events), 'real gateway allocation required'
        assert not any(event.get('rejected_protocol') or event.get('unsupported') for event in events), 'disabled PCP/UPnP must not be attempted'
        rules = (evidence / 'final-rules.txt').read_text()
        assert '--dport 49041' in rules and not any('-p tcp' in line and ('--dport 41041' in line or '--dport 42041' in line) for line in rules.splitlines()), 'close must clear only owned mapping and preserve foreign owner'
        if case not in ('expired', 'initial-failure'):
            assert any(event.get('protocol') == 'tcp' and event.get('requested_lifetime') == 0 and event.get('released') for event in events), 'graceful close must release its own mapping'
        assert 'DATA_OK' in (evidence / 'serve.log').read_text() and 'DATA_OK' in (evidence / 'connect.log').read_text()
        if case == 'restart':
            assert len({event['pid'] for event in events if event.get('gateway_started')}) == 2, 'actual gateway service must restart'
        if case == 'renewed':
            assert sum(event.get('granted_lifetime') == 6 for event in events) >= 3, 'two renewals beyond original lease are required'
        if case == 'expired':
            assert 'CANDIDATE_WITHDRAWN' in (evidence / 'serve.log').read_text()
            assert 'NEW_CONNECTION_UNAVAILABLE' in (evidence / 'unreachable.log').read_text()
        if lifecycle:
            assert 'EXISTING_CONNECTION_CONTINUES' in (evidence / 'serve.log').read_text() and 'EXISTING_CONNECTION_CONTINUES' in (evidence / 'connect.log').read_text(), 'existing end-to-end session outcome must be independently verified'
        passed = True
        print(f'PASS {case} ' + ('allocation rejected; allowed LAN authorization/stream' if case == 'initial-failure' else 'actual NAT-PMP mapped authorization/stream') + f' evidence={evidence}')
    finally:
        for proc in reversed(jobs):
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
        if passed:
            capture = subprocess.run(['tcpdump', '-nn', '-r', str(evidence / 'gateway.pcap'), 'udp port 1900 or (udp dst port 5351 and udp[8] != 0)'], check=False, text=True, capture_output=True)
            (evidence / 'disabled-protocol-packets.txt').write_text(capture.stdout)
        gateway_log = directory / 'gateway.jsonl'
        if gateway_log.exists():
            shutil.copyfile(gateway_log, evidence / 'gateway.jsonl')
        shutil.rmtree(directory)
        for out in files:
            out.close()
        for name in reversed(list(names.values())):
            subprocess.run(['ip', 'netns', 'del', name], check=False)
        if passed:
            assert capture.returncode == 0 and not capture.stdout.strip(), 'PCP and UPnP must have no packets'



def udp_sentinel(directory):
    directory = Path(directory)
    data = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    data.bind(('0.0.0.0', 41041))
    control = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    control.settimeout(2)
    control.sendto(struct.pack('!BBHHHI', 0, 1, 0, 41041, 41041, 30), ('10.41.0.1', 5351))
    response, _ = control.recvfrom(32)
    version, opcode, result, _, private, public, lease = struct.unpack('!BBHIHHI', response)
    assert (version, opcode, result, private, public, lease) == (0, 129, 0, 41041, 41041, 30)
    (directory / 'udp-mapped').write_text('')
    while True:
        packet, peer = data.recvfrom(128)
        assert packet == b'independent UDP mapping'
        data.sendto(b'UDP survives TCP close', peer)
        print('UDP_DATA_OK same_internal_and_external_port=41041', flush=True)


def udp_probe(directory):
    data = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    data.settimeout(2)
    data.sendto(b'independent UDP mapping', ('100.64.41.1', 41041))
    assert data.recvfrom(128)[0] == b'UDP survives TCP close'
    print('UDP_SURVIVED_TCP_CLOSE same_port=41041', flush=True)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['gateway', 'lab', 'udp-sentinel', 'udp-probe'])
    parser.add_argument('paths', nargs='+')
    args = parser.parse_args()
    if args.mode == 'gateway':
        gateway(*args.paths)
    elif args.mode == 'udp-sentinel':
        udp_sentinel(*args.paths)
    elif args.mode == 'udp-probe':
        udp_probe(*args.paths)
    else:
        lab(*args.paths)
