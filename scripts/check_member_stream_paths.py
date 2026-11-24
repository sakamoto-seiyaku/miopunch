#!/usr/bin/env python3
"""Exercise ordinary CLI business streams over two direct member paths.

Run inside a disposable user/network namespace. The CLI must be built with
nat-lab-test-hooks. Checks receiver bytes and actual STREAM frame paths.
"""
import argparse
from contextlib import ExitStack, contextmanager
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace

from check_forwarding import Node

DIRECT, BACKUP = 'veth-direct', 'veth-relay'
LINKS = {DIRECT: '10.92.0.', BACKUP: '10.93.0.'}


class ProbeNode(Node):
    # The native diagnostics can also write plaintext to stdout.
    def read(self):
        with (self.runner.output / f'{self.label}.stdout-debug').open('w') as diagnostic:
            try:
                for line in self.process.stdout:
                    if line.lstrip().startswith('{'):
                        self.queue.put(json.loads(line), timeout=2)
                    elif line.strip():
                        diagnostic.write(line)
            except Exception as error:
                self.reader_error = type(error).__name__
            finally:
                self.queue.put(None, timeout=2)

    def relay_address(self, links=None):
        if links is None:
            links = self.runner.initial_links
        prefixes = [LINKS[link] for link in links]
        address = dict(self.address)
        address['addrs'] = [addr for addr in address['addrs'] if 'Relay' in addr or
                            any(addr.get('Ip', '').startswith(prefix) for prefix in prefixes)]
        for prefix in prefixes:
            assert any(addr.get('Ip', '').startswith(prefix) for addr in address['addrs']), address
        return address


def send(node, attachment, command):
    node.call('terminal_send', attachment=attachment,
              message={'type': 'bytes', 'data': list(command.encode())})


def read_marker(node, attachment, marker):
    data = bytearray()
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = node.call('terminal_read', attachment=attachment, timeout_ms=35)
        assert result['attached'], result
        message = result.get('message', {})
        if message.get('type') == 'bytes':
            data.extend(message['data'])
            if marker.encode() in data:
                return time.monotonic()
    raise RuntimeError(f'No terminal response for {marker!r}')


def record(output, event, **info):
    with (output / 'events.jsonl').open('a') as log:
        log.write(json.dumps(dict(event=event, t_ns=time.monotonic_ns(), **info)) + '\n')

def command(*args, namespace=None):
    prefix = [] if namespace is None else ['nsenter', '-t', str(namespace), '-n', '--']
    result = subprocess.run([*prefix, *args], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError((prefix, args, result.returncode, result.stderr))
    return result.stdout

@contextmanager
def process(argv, **opts):
    with subprocess.Popen(argv, **opts) as proc:
        try:
            yield proc
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=4)

def topology(phone):
    current = os.readlink('/proc/self/ns/net')
    deadline = time.monotonic() + 3
    while os.readlink(f'/proc/{phone}/ns/net') == current:
        assert time.monotonic() < deadline, 'Phone namespace startup'
        time.sleep(.010)
    for ns in (None, phone):
        command('ip', 'link', 'set', 'lo', 'up', namespace=ns)
    for link, prefix in LINKS.items():
        command('ip', 'link', 'add', link, 'type', 'veth', 'peer', 'name', 'peer-veth')
        command('ip', 'link', 'set', 'peer-veth', 'netns', str(phone))
        command('ip', 'link', 'set', 'peer-veth', 'name', link, namespace=phone)
        for ns, address in ((None, f'{prefix}1/30'), (phone, f'{prefix}2/30')):
            command('sysctl', '-q', '-w', f'net.ipv6.conf.{link}.disable_ipv6=1', namespace=ns)
            command('ip', 'addr', 'add', address, 'dev', link, namespace=ns)
            command('ip', 'link', 'set', link, 'up', namespace=ns)
    for ns in (None, phone):
        command('nft', 'add', 'table', 'inet', 'warm', namespace=ns)
        command('nft', 'add', 'chain', 'inet', 'warm', 'out', '{', 'type', 'filter',
                'hook', 'output', 'priority', '0', ';', '}', namespace=ns)

def counters(phone):
    return {
        side: {row['ifname']: row['stats64'] for row in json.loads(command(
            'ip', '-s', '-j', 'link', 'show', namespace=ns)) if row['ifname'] in (DIRECT, BACKUP)}
        for side, ns in (('provider', None), ('phone', phone))
    }

def prepare_relay(output, stack, relay_binary):
    errors = stack.enter_context((output / 'relay.stderr').open('w'))
    relay = stack.enter_context(process([str(relay_binary), 'relay', '0'], stdout=subprocess.PIPE,
                                        stderr=errors, text=True))
    ready = relay.stdout.readline().strip()
    assert re.fullmatch(r'RELAY 127\.0\.0\.1:\d+', ready), ready
    local = ready.split()[1]
    port = local.rsplit(':', 1)[1]
    stack.enter_context(process(['socat', f'TCP-LISTEN:{port},bind=10.93.0.1,fork,reuseaddr',
                                f'TCP:{local}'], stderr=errors))
    return f'http://10.93.0.1:{port}/'

def setup_nodes(output, state, phone_pid, relay_url, cli, initial_link):
    config = state / 'resources.json'
    values = {
        'defaults': {'revision': 2, 'resources': []},
        'replacement': [{'url': relay_url, 'uses': ['DataRelay'], 'limitation': 'owned warm lab relay'}],
        'disabled': [], 'disabled_uses': ['Observation', 'Lookup', 'Signaling'],
        'lan_discovery': 'Disabled', 'port_mapping': 'Disabled', 'tcp_direct_disabled': True,
    }
    config.write_text(json.dumps(values))
    wrapper = state / 'phone-cli'
    wrapper.write_text(f'#!/bin/sh\nexec nsenter -t {phone_pid} -n -- {shlex.quote(str(cli))} "$@"\n')
    wrapper.chmod(0o700)
    nodes = []
    shared = dict(private=state, output=output, native_configuration=config, nodes=nodes, sequence=0,
                  initial_links=(initial_link,), record=lambda *args: None)
    pr = SimpleNamespace(binary=cli, **shared)
    ph = SimpleNamespace(binary=wrapper, **shared)
    try:
        provider = ProbeNode(pr, 'provider')
        invitation = state / 'invite'
        provider.call('issue_precode', path=str(invitation), seconds=600, max_uses=1)
        phone = ProbeNode(ph, 'phone', 'join', invitation)
        phone.call('request', path=str(invitation))
        provider.call('service_sync', address=phone.relay_address())
        directory = state / 'tmux'
        assert provider.call('terminal_enable', directory=str(directory))['enabled']
        phone.call('service_sync', address=provider.relay_address())
        workspace = phone.call('terminal_create', address=provider.relay_address())['workspace']
        attachment = phone.call('terminal_attach', address=provider.relay_address(), workspace=workspace)['id']
        send(phone, attachment, "stty -echo; MIO_WARM_STATE=original; printf '%s\\n' $$ > shell-before; printf 'STATE_'; printf 'READY\\n'\n")
        read_marker(phone, attachment, 'STATE_READY')
        record(output, 'attached', attachment=attachment, workspace=workspace, relay=relay_url)
        return nodes, phone, provider, attachment, workspace, directory
    except BaseException:
        for node in reversed(nodes):
            node.close()
        raise

def operation(node):
    values = node.call('terminal_status')['operations']
    return values[0] if values else None

def paired_status(output, phone, provider, label):
    responses = {}
    for node in (phone, provider):
        before = time.monotonic_ns()
        value = operation(node)
        after = time.monotonic_ns()
        record(output, 'sample', side=node.label, request_ns=before,
               response_ns=after, label=label, operation=value)
        responses[node.label] = value
    return responses

def paths(summary):
    return summary['transport']['paths']

def identity(operation):
    transport = operation['transport']
    return [operation['id'], operation['channel'], transport['connection'], transport['stream']]

FIELDS = ('tx_bytes', 'rx_bytes', 'tx_stream_frames', 'rx_stream_frames')


def allow_direct(ns, case):
    for namespace in (None, ns):
        command('nft', 'flush', 'chain', 'inet', 'warm', 'out', namespace=namespace)
        for link in LINKS:
            rule = ['nft', 'add', 'rule', 'inet', 'warm', 'out', 'oifname', link,
                    'meta', 'l4proto', 'udp', 'counter']
            if case not in ('both', 'slow') and link != case:
                rule.append('drop')
            command(*rule, namespace=namespace)


def nft_counters(ns):
    return {side: json.loads(command('nft', '-j', 'list', 'chain', 'inet', 'warm', 'out', namespace=pid))
            for side, pid in (('provider', None), ('phone', ns))}


def sample(output, phone, provider, label, check_forward=False):
    terminal = paired_status(output, phone, provider, label)
    forwards = {node.label: node.call('forward_status')['flows'] for node in (phone, provider)} if check_forward else {}
    return {'terminal': terminal, 'forward': forwards}


def selected_ip(operation):
    return next((path['remote']['Ip'] for path in paths(operation)
                 if path['selected'] and 'Ip' in path['remote']), None)


def wait_direct(output, phone, provider, case):
    expected = LINKS[case]
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        status = sample(output, phone, provider, 'qualify')['terminal']
        if all(s and (selected_ip(s) or '').startswith(expected) for s in status.values()):
            return status
        time.sleep(.02)
    raise RuntimeError(('direct path did not qualify', status))


def wait_direct_pair(output, phone, provider, case):
    deadline = time.monotonic() + 8
    while True:
        statuses = paired_status(output, phone, provider, 'both_ready')
        rtts = {
            side: {direct_link(p): p['rtt_us'] for p in paths(op)
                   if direct_link(p) and p['last_ack_age_ms'] is not None
                   and p['last_ack_age_ms'] < 500}
            for side, op in statuses.items()
        }
        if all(set(values) == set(LINKS) for values in rtts.values()):
            gaps = [values[BACKUP] - values[DIRECT] for values in rtts.values()]
            if (case == 'both' and all(abs(gap) < 5000 for gap in gaps)
                    or case == 'slow' and all(gap >= 40000 for gap in gaps)):
                # Let each endpoint's next selection sample see both paths.
                time.sleep(1.05)
                return rtts
        assert time.monotonic() < deadline, (statuses, rtts)
        time.sleep(.02)


def echo_target(server, sink):
    client, _ = server.accept()
    with client:
        client.settimeout(10)
        while data := client.recv(65536):
            sink[0] += len(data)
            client.sendall(data)


SENDER = '''
import socket, sys, time
address, ready, stop = sys.argv[1:]
host, port = address.rsplit(':', 1)
with socket.create_connection((host, int(port)), timeout=8) as connection:
    transferred = 0
    with connection.makefile('rb') as replies:
        for i in range(70):
            data = bytes([i]) * 32768
            connection.sendall(data)
            response = replies.read(len(data))
            assert response == data, (i, len(response), len(data))
            transferred += len(data)
            if i == 0:
                open(ready, 'w').close()
            time.sleep(.05)
    print(transferred, flush=True)
    deadline = time.monotonic() + 8
    while not __import__('os').path.exists(stop):
        assert time.monotonic() < deadline
        time.sleep(.02)
'''


def per_path(before, after):
    result = {}
    for node in ('phone', 'provider'):
        left = {p['id']: p for p in paths(before[node])}
        right = {p['id']: p for p in paths(after[node])}
        result[node] = []
        for path_id, path in right.items():
            previous = left.get(path_id, {})
            result[node].append({'id': path_id, 'remote': path['remote'], 'local': path['local'],
                                 'selected': path['selected'], 'ack_age_ms': path['last_ack_age_ms'],
                                 'delta': {k: path[k] - previous.get(k, 0) for k in FIELDS}})
        result[node].extend({'id': pid, 'remote': p['remote'], 'retired': True}
                            for pid, p in left.items() if pid not in right)
    return result


def assert_shared(states):
    for node in ('phone', 'provider'):
        terminal = states['terminal'][node]['transport']
        assert len(states['forward'][node]) == 1, states['forward'][node]
        forward = states['forward'][node][0]['transport']
        assert terminal['connection'] == forward['connection'], (terminal, forward)
        assert terminal['stream'] != forward['stream'], (terminal, forward)


def trace_positions(output):
    return {side: (output / f'{side}.stderr').stat().st_size for side in ('phone', 'provider')}


def direct_link(path):
    address = path['remote'].get('Ip', '')
    return next((link for link, prefix in LINKS.items() if address.startswith(prefix)), None)


def run(output, case, cli, relay):
    output.mkdir()
    os.environ.setdefault('TOKIO_WORKER_THREADS', '2')
    dual = case in ('both', 'slow')
    initial_link = DIRECT if dual else case
    with tempfile.TemporaryDirectory(prefix='mio-healthy-direct-state-', dir='/tmp') as private, ExitStack() as stack:
        ns = stack.enter_context(process(['unshare', '-n', 'sleep', '1800']))
        topology(ns.pid)
        allow_direct(ns.pid, 'neither')
        for namespace in (None, ns.pid):
            command('tc', 'qdisc', 'add', 'dev', DIRECT, 'root', 'netem', 'delay', '4ms', namespace=namespace)
            command('tc', 'qdisc', 'add', 'dev', BACKUP, 'root', 'netem', 'delay',
                    '45ms' if case == 'slow' else '4ms', namespace=namespace)
        url = prepare_relay(output, stack, relay)
        nodes, phone, provider, attachment, workspace, directory = setup_nodes(
            output, Path(private), ns.pid, url, cli, initial_link)
        try:
            bootstrap = paired_status(output, phone, provider, 'bootstrap')
            assert all('Relay' in op['transport']['initial_data_path'] for op in bootstrap.values())
            allow_direct(ns.pid, initial_link)
            wait_direct(output, phone, provider, initial_link)
            trace_start = trace_positions(output)
            send(phone, attachment, "printf 'BEFORE_'; printf 'FORWARD\\n'\n")
            read_marker(phone, attachment, 'BEFORE_FORWARD')
            qualified_rtts_us = {}
            if dual:
                allow_direct(ns.pid, case)
                # Supply the B hint through the ordinary CLI on the live
                # member connection after A has delivered terminal data.
                for node, peer in ((phone, provider), (provider, phone)):
                    assert node.call('ping', address=peer.relay_address(LINKS))['reply'] == 'PONG'
                qualified_rtts_us = wait_direct_pair(output, phone, provider, case)
            server = stack.enter_context(socket.socket())
            server.bind(('127.0.0.1', 0))
            server.listen()
            server.settimeout(10)
            sink_bytes = [0]
            sink = threading.Thread(target=echo_target, args=(server, sink_bytes), daemon=True)
            sink.start()
            target = '127.0.0.1:' + str(server.getsockname()[1])
            provider.call('forward_enable', protocol='tcp', targets=[target])
            forward = phone.call('forward_start', address=provider.relay_address(), protocol='tcp',
                                 target=target, bind='127.0.0.1:0')
            ready = Path(private) / 'forward-ready'
            stop = Path(private) / 'forward-stop'
            net_before = counters(ns.pid)
            rules_before = nft_counters(ns.pid)
            trace_before_forward = trace_positions(output)
            with process(['nsenter', '-t', str(ns.pid), '-n', '--', sys.executable, '-c', SENDER,
                              forward['listen'], str(ready), str(stop)], stdout=subprocess.PIPE, text=True) as sender:
                deadline = time.monotonic() + 8
                while not ready.exists():
                    assert sender.poll() is None and time.monotonic() < deadline
                    time.sleep(.01)
                start = sample(output, phone, provider, 'business_start', True)
                assert_shared(start)
                floor = time.monotonic()
                delays = []
                for idx in range(26):
                    planned = floor + idx * .1
                    time.sleep(max(0, planned - time.monotonic()))
                    marker = f'DIR_{idx:03d}'
                    start_send = time.monotonic()
                    send(phone, attachment, f"printf '%s\\n' {marker} >> observed; printf 'DIR_'; printf '{idx:03d}\\n'\n")
                    delays.append(round(1e3 * (read_marker(phone, attachment, marker) - start_send), 3))
                    sample(output, phone, provider, 'business')
                transferred = int(sender.stdout.readline())
                end = sample(output, phone, provider, 'business_end', True)
                assert_shared(end)
                trace_end = trace_positions(output)
                stop.touch()
                assert sender.wait(timeout=4) == 0
            rules_after = nft_counters(ns.pid)
            net_after = counters(ns.pid)
            sink.join(timeout=3)
            assert not sink.is_alive()
            assert transferred == sink_bytes[0] == 70*32768, (transferred, sink_bytes)
            effects = (directory / workspace / 'observed').read_text().splitlines()
            assert effects == [f'DIR_{idx:03d}' for idx in range(26)], effects
            orig = {side: identity(op) for side, op in start['terminal'].items()}
            final = {side: identity(op) for side, op in end['terminal'].items()}
            bootstrap_identity = {side: identity(op) for side, op in bootstrap.items()}
            assert bootstrap_identity == orig == final, (bootstrap_identity, orig, final)
            report = dict(case=case, candidates={node.label: node.relay_address() for node in (phone, provider)},
                          transferred_bytes_each_direction=transferred, interactive_round_trips=len(delays),
                          interactive_ms=delays, bootstrap=bootstrap, start=start, end=end,
                          qualified_rtts_us=qualified_rtts_us,
                          path_deltas=per_path(start['terminal'], end['terminal']),
                          link_deltas={'before': net_before, 'after': net_after},
                          rules_before=rules_before, rules_after=rules_after)
            (output / 'result.json').write_text(json.dumps(report, indent=2)+'\n')
            print(json.dumps(dict(case=case, selected={side:selected_ip(op) for side,op in end['terminal'].items()},
                                  transferred=transferred, echo=len(delays), path_deltas=report['path_deltas'])), flush=True)
            phone.call('forward_stop', task=forward['id'])
            assert not phone.call('terminal_detach', attachment=attachment)['failed']
            phone.call('terminal_end', address=provider.relay_address(), workspace=workspace)
        finally:
            for node in reversed(nodes):
                node.close()


    verify_paths(output, case, start, end, trace_start, trace_before_forward, trace_end)


def stream_number(display):
    initiator, direction, _, index = display.split()
    return 4 * int(index) + int(initiator == 'server') + 2 * int(direction == 'unidirectional')


def verify_paths(output, case, start, end, trace_start, trace_before_forward, trace_end):
    pattern = re.compile(r'STREAM_PATH (tx|rx) stable=Some\((\d+)\) stream=(\d+) path=(\d+) offset=(\d+) len=(\d+)')
    observed = {}
    for side in ('phone', 'provider'):
        stable = start['terminal'][side]['transport']['connection']
        raw = (output / f'{side}.stderr').read_bytes()
        text = raw[trace_start[side]:trace_end[side]].decode()
        before_forward = raw[trace_start[side]:trace_before_forward[side]].decode()
        records = [m.groups() for m in pattern.finditer(text)
                   if int(m[2]) == stable and int(m[6]) > 0]
        streams = [stream_number(start['terminal'][side]['transport']['stream']),
                   stream_number(start['forward'][side][0]['transport']['stream'])]
        known = {int(p['id']): p for p in paths(end['terminal'][side])}
        observed[side] = {}
        for direction in ('tx', 'rx'):
            assigned = []
            for stream in streams:
                frames = [r for r in records if r[0] == direction and int(r[2]) == stream]
                path_ids = {int(r[3]) for r in frames}
                assert len(path_ids) == 1, (side, direction, stream, path_ids)
                path_id, = path_ids
                link = direct_link(known[path_id])
                assert link, (side, direction, known[path_id])
                assigned.append(link)
                observed[side][f'{direction}-{stream}'] = {'path': path_id, 'frames': len(frames),
                    'bytes': sum(int(r[5]) for r in frames), 'link': link, 'remote': known[path_id]['remote']}
                if stream == streams[0]:
                    assert any(m[1] == direction and int(m[2]) == stable and int(m[3]) == stream
                               and int(m[4]) == path_id and int(m[6]) > 0
                               for m in pattern.finditer(before_forward)), (side, direction, stream, path_id)
            expected = ([DIRECT, BACKUP] if case == 'both' else
                        [DIRECT, DIRECT] if case == 'slow' else [case, case])
            assert assigned == expected, (side, direction, assigned, expected)
    for sender, receiver in (('phone', 'provider'), ('provider', 'phone')):
        for key, record in observed[sender].items():
            if key.startswith('tx-'):
                assert record['link'] == observed[receiver]['rx-' + key[3:]]['link'], (sender, receiver, key)
    (output / 'stream-paths.json').write_text(json.dumps(observed, indent=2) + '\n')
    print(json.dumps({'verified': case, 'streams': observed}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('--case', choices=['both', 'slow', DIRECT, BACKUP], default='both')
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--relay', type=Path, required=True)
    args = parser.parse_args()
    os.environ['MIOPUNCH_TRACE_STREAM_PATHS'] = '1'
    run(args.output, args.case, args.cli.resolve(), args.relay.resolve())
