#!/usr/bin/env python3
"""Fresh, local public-CLI reproduction of the two service-sync directions."""
import hashlib
import json
import socket
import sys
import time
from pathlib import Path

ROOT = Path('/home/js/Git/miopunch')
sys.path.insert(0, str(ROOT / 'scripts'))
from check_web_service import OwnedNode  # noqa: E402

PIN = ROOT / '.artifacts/runs/m119-route-feedback-20261028/linux-sdk-current'
EXPECTED = 'b63a7f6280417399df02731ce0c2980edb20f24099244107e8c1b38be47a8c46'
OUTPUT = Path('/tmp/m119-route-feedback-20261028/local-sync-feedback-five-address-good-first')


def local(address, blackholes=None):
    ipv4 = [entry['Ip'] for entry in address['addrs'] if 'Ip' in entry and entry['Ip'].count(':') == 1]
    assert ipv4, 'no actual IPv4 bind port'
    port = ipv4[0].rsplit(':', 1)[1]
    if blackholes is None:
        return {'id': address['id'], 'addrs': [{'Ip': f'127.0.0.1:{port}'}]}
    assert len(blackholes) == 4
    bad = [{'Ip': f'127.0.0.1:{sock.getsockname()[1]}'} for sock in blackholes]
    good = {'Ip': f'127.0.0.2:{port}'}
    return {'id': address['id'], 'addrs': [good, *bad]}


class Runner:
    def __init__(self):
        OUTPUT.mkdir(parents=True, exist_ok=False)
        self.output = OUTPUT
        self.private = OUTPUT / 'private'
        self.private.mkdir()
        self.binary = PIN
        self.nodes = []
        self.sequence = 0
        self.native_configuration = self.private / 'resources.json'
        self.native_configuration.write_text(json.dumps({
            'defaults': {'revision': 2, 'resources': []}, 'replacement': [],
            'disabled': [], 'disabled_uses': ['Observation', 'Lookup', 'Signaling'],
            'lan_discovery': 'Disabled', 'port_mapping': 'Disabled'}))

    def record(self, label, operation, value):
        # Public metadata only. Invitation and recovery-card contents stay private.
        result = value.get('result') if value.get('ok') else None
        row = {'node': label, 'operation': operation, 'ok': value.get('ok'),
               'stage': result.get('stage') if isinstance(result, dict) else None,
               'saved_locally': result.get('saved_locally') if isinstance(result, dict) else None,
               'monotonic': time.monotonic()}
        with (OUTPUT / 'events.jsonl').open('a') as stream:
            stream.write(json.dumps(row) + '\n')


def main():
    assert hashlib.sha256(PIN.read_bytes()).hexdigest() == EXPECTED
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as receiver, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
        receiver.bind(('0.0.0.0', 0))
        receiver.settimeout(2)
        sender.sendto(b'owned-loopback-precondition', ('127.0.0.2', receiver.getsockname()[1]))
        assert receiver.recvfrom(128)[0] == b'owned-loopback-precondition'
    runner = Runner()
    first, blackholes, reverse_elapsed = None, [], None
    try:
        provider = OwnedNode(runner, 'provider')
        invitation = runner.private / 'invite'
        provider.call('invite', path=str(invitation))
        consumer = OwnedNode(runner, 'consumer', 'join', invitation)
        assert consumer.call('request', path=str(invitation))['stage'] == 'AwaitingApproval'
        assert consumer.call('request', path=str(invitation))['stage'] == 'AwaitingApproval'
        requests = provider.call('requests')['requests']
        assert len(requests) == 1 and requests[0]['applicant'] == consumer.identity
        assert provider.call('approve', **requests[0])['stage'] == 'Approved'
        assert consumer.call('request', path=str(invitation))['stage'] == 'Complete'
        assert provider.call('service_sync', address=local(consumer.address))['saved_locally']
        for _ in range(4):
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(('127.0.0.1', 0))
            blackholes.append(sock)
        started = time.monotonic()
        assert consumer.call('service_sync', address=local(provider.address, blackholes))['saved_locally']
        reverse_elapsed = time.monotonic() - started
    except BaseException as exc:
        first = {'type': type(exc).__name__, 'message': str(exc)[:400]}
    for sock in blackholes: sock.close()
    cleanup = []
    for node in reversed(runner.nodes):
        try:
            cleanup.append(node.close())
        except BaseException as exc:
            cleanup.append({'node': getattr(node, 'label', None), 'failure': type(exc).__name__})
    result = {'first_failure': first, 'cleanup': cleanup, 'pin_sha256': EXPECTED,
              'scope': 'fresh local two-node five-ordered-address direct-loopback CLI sequence',
              'blackhole_count': len(blackholes), 'reverse_sync_seconds': reverse_elapsed}
    (OUTPUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    assert first is None and all(item.get('exit') == 0 and not item.get('errors') for item in cleanup)


if __name__ == '__main__':
    main()
