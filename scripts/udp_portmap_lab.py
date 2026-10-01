#!/usr/bin/env python3
"""MIOPUNCH-39: isolated NAT-PMP wire service backed by actual kernel DNAT."""
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
    sock.bind(('10.39.0.1', 5351))
    active = {}
    start = time.monotonic()
    last_mode = "normal"
    log.write(json.dumps({'gateway_started': True, 'pid': os.getpid(), 'epoch': 0}) + '\n')

    def rule(action, ip, internal, external):
        run('iptables', '-t', 'nat', action, 'PREROUTING', '-i', 'wan0', '-p', 'udp', '--dport', str(external), '-j', 'DNAT', '--to-destination', f'{ip}:{internal}')
        run('iptables', action, 'FORWARD', '-i', 'wan0', '-o', 'lan0', '-p', 'udp', '-d', ip, '--dport', str(internal), '-j', 'ACCEPT')

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
                sock.sendto(struct.pack('!BBHI4s', 0, 128, 0, epoch, socket.inet_aton('100.64.39.1')), source)
                event['public_address'] = '100.64.39.1'
        elif packet[1] == 1 and len(packet) == 12:
            _, _, _, internal, preferred, lifetime = struct.unpack('!BBHHHI', packet)
            key = (source[0], internal)
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
                external = 42039 if mode in ('changed', 'restart') else 41039
                granted, result = (30 if mode.startswith("allocation-") else 6), 0
                if key in active and active[key][0] != external:
                    remove(key)
                if key not in active:
                    rule('-A', *key, external)
                active[key] = external, time.monotonic() + granted
            sock.sendto(struct.pack('!BBHIHHI', 0, 129, result, epoch, internal, external, granted), source)
            event.update(internal_port=internal, preferred_port=preferred, requested_lifetime=lifetime, external_port=external, granted_lifetime=granted, result=result)
        else:
            event['unsupported'] = True
        event['active'] = [{'owner': list(k), 'external_port': v[0]} for k, v in active.items()]
        log.write(json.dumps(event) + '\n')
    for key in list(active):
        remove(key)


def lab(binary, directory, case="positive"):
    evidence = Path(directory).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="mio39-private-"))
    prefix = f'm39-{os.getpid()}'
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
        command = ['env', f'MIO39_ROLE={role}', f'MIO39_DIRECTORY={directory}', f'MIO39_CASE={case}', binary, '--exact', 'gateway_node_worker', '--nocapture']
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
        for left, li, la, right, ri, ra in [('gateway', 'lan0', '10.39.0.1/24', 'node', 'eth0', '10.39.0.2/24'), ('gateway', 'wan0', '100.64.39.1/24', 'remote', 'eth0', '100.64.39.2/24')]:
            run('ip', 'link', 'add', 'm39-left', 'type', 'veth', 'peer', 'name', 'm39-right')
            for link, role, interface, address in [('m39-left', left, li, la), ('m39-right', right, ri, ra)]:
                run('ip', 'link', 'set', link, 'netns', names[role])
                run('ip', '-n', names[role], 'link', 'set', link, 'name', interface)
                run('ip', '-n', names[role], 'addr', 'add', address, 'dev', interface)
                run('ip', '-n', names[role], 'link', 'set', interface, 'up')
        run('ip', '-n', names['node'], 'route', 'add', 'default', 'via', '10.39.0.1')
        ns('gateway', 'sysctl', '-qw', 'net.ipv4.ip_forward=1')
        ns('gateway', 'iptables', '-P', 'FORWARD', 'DROP')
        ns('gateway', 'iptables', '-A', 'FORWARD', '-i', 'lan0', '-o', 'wan0', '-j', 'ACCEPT')
        ns('gateway', 'iptables', '-A', 'FORWARD', '-m', 'conntrack', '--ctstate', 'ESTABLISHED,RELATED', '-j', 'ACCEPT')
        ns('gateway', 'iptables', '-t', 'nat', '-A', 'POSTROUTING', '-o', 'wan0', '-j', 'MASQUERADE')
        # A foreign owner's permanent rule is never touched by the node.
        ns('gateway', 'iptables', '-t', 'nat', '-A', 'PREROUTING', '-i', 'wan0', '-p', 'udp', '--dport', '49039', '-j', 'DNAT', '--to-destination', '10.39.0.3:49039')
        prepared = worker('prepare', 'remote')
        if prepared.wait(timeout=20) != 0:
            raise RuntimeError('prepare failed')
        if case.startswith('allocation-'):
            (directory / 'gateway-mode').write_text(case)
        gateway_process = spawn('gateway', ['python3', __file__, 'gateway', str(directory)], 'gateway.log')
        spawn('gateway', ['tcpdump', '--immediate-mode', '-U', '-ni', 'any', '-w', str(evidence / 'gateway.pcap'), 'udp'], 'tcpdump.log')
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
            assert any(event.get('requested_lifetime') == 0 and event.get('released') for event in events), 'acknowledged allocation must be released on failure or cancellation before publication'
            rules = (evidence / 'final-rules.txt').read_text()
            assert '--dport 49039' in rules and '--dport 41039' not in rules, 'owned allocation must be removed, foreign owner retained'
            passed = True
            print(f'PASS {case} acknowledged allocation released evidence={evidence}')
            return
        if case == 'disabled':
            disabled = worker('disabled', 'node')
            if disabled.wait(timeout=8) != 0:
                raise RuntimeError('disabled mapping worker failed')
            time.sleep(.1)
            events = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
            assert all(event.get('gateway_started') for event in events), 'disabled node must send no mapping protocol requests'
            passed = True
            print(f'PASS disabled no gateway requests evidence={evidence}')
            return
        server = worker('serve', 'node')
        wait_file('mapped', server)
        client = worker('connect', 'remote')
        if client.wait(timeout=30) != 0:
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
                ns('gateway', 'iptables', '-t', 'nat', '-D', 'PREROUTING', '-i', 'wan0', '-p', 'udp', '--dport', '41039', '-j', 'DNAT', '--to-destination', f'10.39.0.2:{internal}')
                ns('gateway', 'iptables', '-D', 'FORWARD', '-i', 'wan0', '-o', 'lan0', '-p', 'udp', '-d', '10.39.0.2', '--dport', str(internal), '-j', 'ACCEPT')
                (directory / 'gateway-mode').write_text('restart')
                gateway_process = spawn('gateway', ['python3', __file__, 'gateway', str(directory)], 'gateway-restarted.log')
            elif case != 'renewed':
                (directory / 'gateway-mode').write_text(case)
            wait_file('renewed' if case == 'renewed' else 'changed', server)
            reconnect = worker('reconnect', 'remote')
            if reconnect.wait(timeout=8) != 0:
                raise RuntimeError('fresh connection to renewed/replaced candidate failed')
        if server.wait(timeout=15) != 0:
            raise RuntimeError('mapped node worker failed')
        time.sleep(.15)
        with (evidence / 'final-rules.txt').open('w') as out:
            ns('gateway', 'iptables-save', stdout=out)
        events = [json.loads(line) for line in (directory / 'gateway.jsonl').read_text().splitlines()]
        assert any(event.get('external_port') == 41039 and event.get('result') == 0 for event in events), 'real gateway allocation required'
        assert not any(event.get('rejected_protocol') or event.get('unsupported') for event in events), 'disabled PCP/UPnP must not be attempted'
        rules = (evidence / 'final-rules.txt').read_text()
        assert '--dport 49039' in rules and '--dport 41039' not in rules and '--dport 42039' not in rules, 'close must clear only owned mapping and preserve foreign owner'
        if case != 'expired':
            assert any(event.get('requested_lifetime') == 0 and event.get('released') for event in events), 'graceful close must release its own mapping'
        assert 'DATA_OK' in (evidence / 'serve.log').read_text() and 'DATA_OK' in (evidence / 'connect.log').read_text()
        if case == 'restart':
            assert len({event['pid'] for event in events if event.get('gateway_started')}) == 2, 'actual gateway service must restart'
        if case == 'renewed':
            assert sum(event.get('granted_lifetime') == 6 for event in events) >= 3, 'two renewals beyond original lease are required'
        if case == 'expired':
            assert 'CANDIDATE_WITHDRAWN' in (evidence / 'serve.log').read_text()
            assert 'NEW_CONNECTION_UNAVAILABLE' in (evidence / 'unreachable.log').read_text()
        passed = True
        print(f'PASS {case} actual NAT-PMP mapped authorization/stream evidence={evidence}')
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


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['gateway', 'lab'])
    parser.add_argument('paths', nargs='+')
    args = parser.parse_args()
    if args.mode == 'gateway':
        gateway(*args.paths)
    else:
        lab(*args.paths)
