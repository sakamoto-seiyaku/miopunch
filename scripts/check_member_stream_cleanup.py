#!/usr/bin/env python3
"""Verify path failure and service revocation on a split native member QUIC.

Run like check_member_stream_paths.py in a disposable user/network namespace.
"""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import socket
import tempfile
import time

from check_member_stream_paths import (
    BACKUP, DIRECT, LINKS, allow_direct, command, direct_link, identity,
    paired_status, prepare_relay, process, read_marker, record, selected_ip,
    send, setup_nodes, stream_number, topology, trace_positions, wait_direct,
    wait_direct_pair,
)
from check_member_stream_warmup import TRACE


def read_bytes(sock, expected):
    received = bytearray()
    while len(received) < len(expected):
        chunk = sock.recv(len(expected) - len(received))
        assert chunk, received
        received.extend(chunk)
    assert received == expected, (received, expected)


def run(output, cli, relay, case):
    output.mkdir()
    os.environ['MIOPUNCH_TRACE_STREAM_PATHS'] = '1'
    os.environ.setdefault('TOKIO_WORKER_THREADS', '2')
    with tempfile.TemporaryDirectory(prefix='mio-stream-cleanup-') as private, ExitStack() as stack:
        ns = stack.enter_context(process(['unshare', '-n', 'sleep', '1800']))
        topology(ns.pid)
        allow_direct(ns.pid, 'neither')
        for namespace in (None, ns.pid):
            for link in LINKS:
                command('tc', 'qdisc', 'add', 'dev', link, 'root',
                        'netem', 'delay', '4ms', namespace=namespace)
        url = prepare_relay(output, stack, relay)
        nodes, phone, provider, attachment, workspace, directory = setup_nodes(
            output, Path(private), ns.pid, url, cli, DIRECT)
        try:
            allow_direct(ns.pid, DIRECT)
            start = wait_direct(output, phone, provider, DIRECT)
            original = {side: identity(op) for side, op in start.items()}
            tcp_received, terminal_received = [], []

            def observe(label, offset, transports, link):
                observed = {}
                for side, transport in transports.items():
                    stable = transport['connection']
                    number = stream_number(transport['stream'])
                    assert stable == original[side][2], (label, side, stable, original)
                    remote = {int(path['id']): direct_link(path) for path in transport['paths']}
                    raw = (output / f'{side}.stderr').read_bytes()[offset[side]:].decode()
                    directions = {direction: set() for direction in ('tx', 'rx')}
                    for match in TRACE.finditer(raw):
                        if int(match[2]) == stable and int(match[3]) == number and int(match[6]):
                            directions[match[1]].add(remote.get(int(match[4])))
                    assert all(seen == {link} for seen in directions.values()), (label, side, directions, link)
                    observed[side] = dict(connection=stable, stream=number, link=link)
                record(output, 'stream_paths', label=label, endpoints=observed)

            def terminal_round_trip(label):
                marker = 'MIO_' + label
                offset = trace_positions(output)
                send(phone, attachment, f"printf '%s\\n' {marker} >> cleanup; printf 'MIO_'; printf '{label}\\n'\n")
                read_marker(phone, attachment, marker)
                ops = paired_status(output, phone, provider, label)
                assert {side: identity(op) for side, op in ops.items()} == original
                observe(label, offset, {side: op['transport'] for side, op in ops.items()}, DIRECT)
                terminal_received.append(marker)

            terminal_round_trip('initial_terminal')
            allow_direct(ns.pid, 'both')
            # Qualify B explicitly through the original CLI connection, with
            # only the initial terminal open.
            for node, peer in ((phone, provider), (provider, phone)):
                ping = node.call('ping', address=peer.relay_address(LINKS))
                assert ping['reply'] == 'PONG' and ping['transport']['connection'] == original[node.label][2]
            rtts = wait_direct_pair(output, phone, provider, 'both')
            preferred = paired_status(output, phone, provider, 'split_preferred')
            assert all((selected_ip(op) or '').startswith(LINKS[DIRECT]) for op in preferred.values())
            record(output, 'qualified_pair', rtt_us=rtts)

            # The provider namespace owns both independent sockets. The
            # serving CLI in the phone namespace reaches the target via A.
            listener = stack.enter_context(socket.socket())
            listener.bind(('10.92.0.1', 0))
            listener.listen()
            listener.settimeout(8)
            target = '%s:%s' % listener.getsockname()
            phone.call('forward_enable', protocol='tcp', targets=[target])
            forward = provider.call('forward_start', address=phone.relay_address(),
                                    protocol='tcp', target=target, bind='127.0.0.1:0')
            host, port = forward['listen'].rsplit(':', 1)
            listen = host, int(port)

            def flows():
                return {node.label: {row['id']: row['transport'] for row in node.call('forward_status')['flows']}
                        for node in (phone, provider)}

            def round_trip(client, receiver, label):
                # Consecutive distinct records make a duplicate or out-of-order
                # delivery visibly different at either independent reader.
                sent = (label.encode() + b'\0') * 256
                reply = (label.encode() + b'\1') * 128
                client.sendall(sent)
                read_bytes(receiver, sent)
                receiver.sendall(reply)
                read_bytes(client, reply)
                tcp_received.append(dict(label=label, request_bytes=len(sent), reply_bytes=len(reply)))

            def open_flow(label, link):
                before = flows()
                offset = trace_positions(output)
                client = stack.enter_context(socket.create_connection(listen, timeout=8))
                receiver, _ = listener.accept()
                stack.enter_context(receiver)
                receiver.settimeout(8)
                round_trip(client, receiver, label)
                after = flows()
                ids = {}
                for side in after:
                    ids[side], = after[side].keys() - before[side].keys()
                transports = {side: rows[ids[side]] for side, rows in after.items()}
                assert transports['phone']['stream'] == transports['provider']['stream'], transports
                observe(label, offset, transports, link)
                return ids, client, receiver

            def use_flow(flow, label, link):
                ids, client, receiver = flow
                before = flows()
                offset = trace_positions(output)
                round_trip(client, receiver, label)
                after = flows()
                streams = {side: rows[ids[side]] for side, rows in after.items()}
                assert all(row['stream'] == before[side][ids[side]]['stream'] for side, row in streams.items())
                observe(label, offset, streams, link)

            def close_flow(flow):
                ids, client, receiver = flow
                client.shutdown(socket.SHUT_WR)
                assert receiver.recv(1) == b''
                receiver.shutdown(socket.SHUT_WR)
                assert client.recv(1) == b''
                receiver.close()
                client.close()
                deadline = time.monotonic() + 6
                while any(ids[side] in rows for side, rows in flows().items()):
                    assert time.monotonic() < deadline, ids
                    time.sleep(.02)

            split = open_flow('initial_forward_B', BACKUP)
            if case == 'cut':
                allow_direct(ns.pid, DIRECT)
                record(output, 'cut', link=BACKUP)
                # Start the first post-cut transfer before waiting for the
                # existing stream to leave B.
                original_forward = {side: rows[split[0][side]] for side, rows in flows().items()}
                round_trip(*split[1:], 'during_cut')
                use_flow(split, 'old_forward_after_cut', DIRECT)
                assert all(rows[split[0][side]]['stream'] == original_forward[side]['stream']
                           for side, rows in flows().items())
                terminal_round_trip('terminal_after_cut')
                close_flow(open_flow('new_forward_after_cut', DIRECT))
                close_flow(split)
                close_flow(open_flow('new_forward_after_release', DIRECT))
            else:
                on_a = open_flow('initial_forward_A', DIRECT)
                close_flow(open_flow('new_forward_before_revoke', BACKUP))
                terminal_round_trip('terminal_before_revoke')
                policy = provider.call('service_rule', provider=provider.identity,
                                       service='terminal', rule={'members': False, 'guests': True, 'denied': []})
                assert policy['saved_locally']
                deadline = time.monotonic() + 6
                while any(paired_status(output, phone, provider, 'revoke_terminal').values()):
                    assert time.monotonic() < deadline
                    time.sleep(.02)
                while True:
                    ended = phone.call('terminal_read', attachment=attachment, timeout_ms=25)
                    if not ended['attached']:
                        break
                    assert time.monotonic() < deadline, ended
                record(output, 'terminal_attachment_ended', result=ended)
                # The provider still runs terminal service, and now rejects a
                # fresh request for the same workspace.
                phone.runner.record = lambda side, operation, reply: record(
                    output, 'terminal_attach_denied', reply=reply)
                phone.call('terminal_attach', ok=False, address=provider.relay_address(), workspace=workspace)
                phone.runner.record = lambda *args: None
                use_flow(split, 'old_forward_B_after_revoke', BACKUP)
                use_flow(on_a, 'old_forward_A_after_revoke', DIRECT)
                close_flow(open_flow('new_forward_after_revoke', DIRECT))
                close_flow(on_a)
                close_flow(split)

            assert (directory / workspace / 'cleanup').read_text().splitlines() == terminal_received
            listener.settimeout(.15)
            try:
                unexpected, _ = listener.accept()
            except TimeoutError:
                pass
            else:
                unexpected.close()
                raise AssertionError('unexpected additional TCP target connection')
            provider.call('forward_stop', task=forward['id'])
            result = dict(case=case, independent_tcp_received=tcp_received,
                          independent_terminal_received=terminal_received)
            (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps(result))
        finally:
            for node in reversed(nodes):
                node.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--relay', type=Path, required=True)
    parser.add_argument('--case', required=True, choices=('cut', 'revoke'))
    args = parser.parse_args()
    run(args.output, args.cli.resolve(), args.relay.resolve(), args.case)
