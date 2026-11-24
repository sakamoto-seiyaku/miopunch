#!/usr/bin/env python3
"""Exercise candidate preparation on an admitted CLI member QUIC.

Run like check_member_stream_paths.py in a disposable user/network namespace.
"""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import re
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

from check_member_stream_paths import (
    BACKUP, DIRECT, LINKS, allow_direct, command, direct_link, identity,
    paired_status, paths, prepare_relay, process, read_marker, send,
    nft_counters, setup_nodes, stream_number, topology, trace_positions, wait_direct,
)

FLOW = r"""
import socket, sys
host, port = sys.argv[1].rsplit(':', 1)
with socket.create_connection((host, int(port)), timeout=8) as client, client.makefile('rb') as reply:
    for line in sys.stdin.buffer:
        client.sendall(line)
        assert reply.readline() == line, line
        sys.stdout.buffer.write(line)
        sys.stdout.buffer.flush()
"""
THIRD = 'veth-third'
TRACE = re.compile(r'STREAM_PATH (tx|rx) stable=Some\((\d+)\) stream=(\d+) path=(\d+) offset=(\d+) len=(\d+)')


class Echo(socketserver.StreamRequestHandler):
    def handle(self):
        for line in self.rfile:
            self.server.received.append(line.decode().strip())
            self.wfile.write(line)


def run(output, cli, relay, case, forward_from):
    output.mkdir()
    os.environ['MIOPUNCH_TRACE_STREAM_PATHS'] = '1'
    os.environ.setdefault('TOKIO_WORKER_THREADS', '2')
    with tempfile.TemporaryDirectory(prefix='mio179-cli-') as private, ExitStack() as stack:
        ns = stack.enter_context(process(['unshare', '-n', 'sleep', '1800']))
        LINKS[THIRD] = '10.94.0.'
        topology(ns.pid)
        allow_direct(ns.pid, 'neither')
        for namespace in (None, ns.pid):
            for link in LINKS:
                command('tc', 'qdisc', 'add', 'dev', link, 'root', 'netem', 'delay', '4ms',
                        namespace=namespace)
        url = prepare_relay(output, stack, relay)
        nodes, phone, provider, attachment, workspace, directory = setup_nodes(
            output, Path(private), ns.pid, url, cli, DIRECT)
        try:
            allow_direct(ns.pid, DIRECT)
            start = wait_direct(output, phone, provider, DIRECT)
            original = {side: identity(op) for side, op in start.items()}
            source, destination = (phone, provider) if forward_from == 'phone' else (provider, phone)
            client_namespace = ['nsenter', '-t', str(ns.pid), '-n', '--'] if source is phone else []
            target_host = '127.0.0.1' if destination is provider else '10.92.0.1'
            server = stack.enter_context(socketserver.ThreadingTCPServer((target_host, 0), Echo))
            server.received = []
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            stack.callback(server.shutdown)
            target = f'{target_host}:{server.server_address[1]}'
            destination.call('forward_enable', protocol='tcp', targets=[target])
            forward = source.call('forward_start', address=destination.relay_address(tuple(LINKS)),
                                  protocol='tcp', target=target, bind='127.0.0.1:0')
            results = []
            expected_payloads = []
            live = {}

            def trace(transport, offsets, link, label):
                observed = {}
                for side in ('phone', 'provider'):
                    row = transport[side]
                    number, stable = stream_number(row['stream']), row['connection']
                    assert stable == original[side][2]
                    raw = (output / f'{side}.stderr').read_bytes()[offsets[side]:].decode(errors='replace')
                    remote_link = {int(path['id']): direct_link(path) for path in row['paths']}
                    for direction in ('tx', 'rx'):
                        seen = {remote_link.get(int(m[4])) for m in TRACE.finditer(raw)
                                if m[1] == direction and int(m[2]) == stable and int(m[3]) == number
                                and int(m[6]) > 0}
                        assert seen == {link}, (side, direction, label, number, seen, link)
                    observed[side] = {'connection': stable, 'stream': number, 'link': link,
                                      'initiator_quic_role': row['stream'].split()[0]}
                results.append(dict(label=label, paths=observed))

            def flow_ids():
                return {node.label: {f['transport']['stream']: f['transport']
                                     for f in node.call('forward_status')['flows']}
                        for node in (phone, provider)}

            def open_flows(labels, link):
                before = flow_ids()
                offset = trace_positions(output)
                children = {}
                for label in labels:
                    children[label] = stack.enter_context(process(
                        [*client_namespace, sys.executable, '-c', FLOW, forward['listen']],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True))
                tokens = {label: f'{label}-first' for label in labels}
                sent = len(server.received)
                for label, child in children.items():
                    child.stdin.write(tokens[label] + '\n')
                    child.stdin.flush()
                for label, child in children.items():
                    assert child.stdout.readline() == tokens[label] + '\n'
                received = server.received[sent:]
                assert sorted(received) == sorted(tokens.values()), received
                expected_payloads.extend(received)

                after = flow_ids()
                streams = {side: sorted(after[side].keys() - before[side].keys(), key=stream_number)
                           for side in before}
                assert len(streams['phone']) == len(labels)
                assert streams['phone'] == streams['provider'], streams
                # All simultaneous flows have the same expected path and close together.
                for label, phone_stream in zip(labels, streams['phone']):
                    transport = {side: after[side][phone_stream] for side in after}
                    trace(transport, offset, link, label)
                    live[label] = (children[label], transport)

            def new_flow(label, link):
                open_flows((label,), link)

            def send_again(label, link):
                child, transport = live[label]
                offset = trace_positions(output)
                token = f'{label}-again'
                expected_payloads.append(token)
                child.stdin.write(token + '\n')
                child.stdin.flush()
                assert child.stdout.readline() == token + '\n'
                trace(transport, offset, link, token)

            def close_flows(*labels):
                removed = {}
                for label in labels:
                    child, transports = live.pop(label)
                    child.stdin.close()
                    removed[label] = transports
                deadline = time.monotonic() + 6
                while True:
                    active = flow_ids()
                    if all(row['stream'] not in active[side]
                           for transports in removed.values() for side, row in transports.items()):
                        return
                    assert time.monotonic() < deadline, (labels, active)
                    time.sleep(.02)

            def terminal_round_trip(label):
                offset = trace_positions(output)
                marker = 'OCC_' + label
                send(phone, attachment, f"printf '%s\\n' {marker} >> occupancy; printf 'OCC_'; printf '{label}\\n'\n")
                read_marker(phone, attachment, marker)
                operations = paired_status(output, phone, provider, label)
                assert {side: identity(op) for side, op in operations.items()} == original
                trace({side: op['transport'] for side, op in operations.items()}, offset, DIRECT, label)
                return marker

            def snapshot(label):
                counts = {}
                for side, payload in nft_counters(ns.pid).items():
                    counts[side] = {}
                    for entry in payload['nftables']:
                        if rule := entry.get('rule'):
                            link = rule['expr'][0]['match']['right']
                            counts[side][link] = rule['expr'][2]['counter']['packets']
                if label:
                    (output / (label + '.json')).write_text(json.dumps(counts, indent=2))
                return counts

            terminal = [terminal_round_trip('before')]
            if case == 'success':
                # Queue B's datagrams before they can complete the handshake.
                # C stays gated until the business-before-B observation.
                for namespace in (None, ns.pid):
                    command('tc', 'qdisc', 'change', 'dev', BACKUP, 'root',
                            'netem', 'delay', '1200ms', namespace=namespace)
                    command('nft', 'flush', 'chain', 'inet', 'warm', 'out', namespace=namespace)
                    for link in LINKS:
                        rule = ['nft', 'add', 'rule', 'inet', 'warm', 'out', 'oifname', link,
                                'meta', 'l4proto', 'udp', 'counter']
                        if link == THIRD:
                            rule.append('drop')
                        command(*rule, namespace=namespace)
            else:
                allow_direct(ns.pid, DIRECT)

            new_flow('while_B_unverified', DIRECT)
            open_flows(('concurrent_1', 'concurrent_2'), DIRECT)
            deadline = time.monotonic() + 3
            while not any(rows[BACKUP] for rows in snapshot(None).values()):
                assert time.monotonic() < deadline, snapshot('during_gate')
                time.sleep(.02)
            before_b = snapshot('during_gate')
            for side, op in paired_status(output, phone, provider, 'business-before-B').items():
                assert all(path['last_ack_age_ms'] is None
                           for path in paths(op) if direct_link(path) == BACKUP), (side, op)
            assert all(not row[THIRD] for row in before_b.values()), before_b
            close_flows('concurrent_1', 'concurrent_2')
            if case == 'success':
                allow_direct(ns.pid, 'both')
                for namespace in (None, ns.pid):
                    command('tc', 'qdisc', 'change', 'dev', BACKUP, 'root',
                            'netem', 'delay', '4ms', namespace=namespace)
                deadline = time.monotonic() + 9
                while True:
                    state = paired_status(output, phone, provider, 'qualify')
                    rtts = {side: {direct_link(path): path['rtt_us'] for path in paths(op)
                                   if direct_link(path) and path['last_ack_age_ms'] is not None
                                   and path['last_ack_age_ms'] < 500}
                            for side, op in state.items()}
                    if all(set(row) == {DIRECT, BACKUP} and max(row.values()) - min(row.values()) < 5000
                           for row in rtts.values()):
                        break
                    assert time.monotonic() < deadline, rtts
                    time.sleep(.02)
                time.sleep(1.05)
                new_flow('after_B', BACKUP)
                new_flow('another_after_B', BACKUP)
                assert all(not row[THIRD] for row in snapshot('two_ready').values())
                close_flows('after_B', 'another_after_B')
            else:
                time.sleep(5.25)
                new_flow('after_B_failure', DIRECT)
                assert all(not row[THIRD] for row in snapshot('failed').values())
                close_flows('after_B_failure')
            send_again('while_B_unverified', DIRECT)
            terminal.append(terminal_round_trip('after'))
            close_flows('while_B_unverified')
            received_terminal = (directory / workspace / 'occupancy').read_text().splitlines()
            assert received_terminal == terminal
            assert not phone.call('terminal_detach', attachment=attachment)['failed']
            phone.call('terminal_end', address=provider.relay_address(), workspace=workspace)
            deadline = time.monotonic() + 5
            while any(paired_status(output, phone, provider, 'release').values()):
                assert time.monotonic() < deadline
                time.sleep(.02)
            time.sleep(1.1)
            idle_start = snapshot('idle_start')
            time.sleep(6)
            idle_end = snapshot('idle_end')
            for side in idle_end:
                assert idle_end[side][DIRECT] > idle_start[side][DIRECT], (idle_start, idle_end)
                assert idle_end[side][BACKUP] == idle_start[side][BACKUP], (idle_start, idle_end)
                assert idle_end[side][THIRD] == idle_start[side][THIRD], (idle_start, idle_end)
            new_flow('reopened_from_standby', DIRECT)
            close_flows('reopened_from_standby')
            assert server.received == expected_payloads, server.received
            source.call('forward_stop', task=forward['id'])
            report = dict(flows=results, independent_tcp_received=server.received,
                          independent_terminal_received=received_terminal,
                          forward_from=forward_from, case=case)
            (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps(report))
        finally:
            for node in reversed(nodes):
                node.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--relay', type=Path, required=True)
    parser.add_argument('--case', required=True, choices=('success', 'failure'))
    parser.add_argument('--forward-from', choices=('phone', 'provider'), default='phone')
    opts = parser.parse_args()
    run(opts.output, opts.cli.resolve(), opts.relay.resolve(), opts.case, opts.forward_from)
