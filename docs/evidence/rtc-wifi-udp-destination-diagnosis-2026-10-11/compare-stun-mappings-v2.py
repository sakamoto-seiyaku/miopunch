import collections
import hashlib
import importlib.util
import json
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('udp_capture_parse', HERE / 'analyze-retained-udp-v2.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

def attrs(body):
    offset = 20
    values = {}
    while offset + 4 <= len(body):
        kind, size = struct.unpack('!HH', body[offset:offset + 4])
        offset += 4
        value = body[offset:offset + size]
        if len(value) != size:
            break
        values[kind] = value
        offset += (size + 3) // 4 * 4
    return values

def mapped(body):
    value = attrs(body).get(0x20)
    if value is None or len(value) != 8 or value[1] != 1:
        return None
    return bytes(a ^ b for a, b in zip(value[4:8], body[4:8])), int.from_bytes(value[2:4], 'big') ^ 0x2112

results = []
for name, root in [('previous-pass', module.OLD),
                   ('current-replica-fail', module.NEW / 'mqtt-rtc-whole-story-first'),
                   ('current-member-fail', module.NEW / 'mqtt-member-only-current-v1')]:
    profiles = module.OLD if name == 'previous-pass' else module.NEW
    profile = json.loads((profiles / 'private-native-profile.json').read_text())
    import ipaddress
    servers = {(ipaddress.ip_address(url[5:].rsplit(':', 1)[0]).packed, int(url.rsplit(':', 1)[1])) for url in profile['stun_servers']}
    packets = {role: list(module.packets(root / 'runtime/private-pcap' / role / 'pre-sdk-udp-and-mqtt.pcap')) for role in ['native', 'browser']}
    native_mappings = set()
    for stamp, source, sport, target, dport, body in packets['native']:
        if (source, sport) in servers and body[:2] == b'\x01\x01':
            value = mapped(body)
            if value:
                native_mappings.add(value)
    peer_requests = []
    server_responses = {}
    browser_mappings = set()
    for stamp, source, sport, target, dport, body in packets['browser']:
        if len(body) < 20 or body[4:8] != b'\x21\x12\xa4\x42':
            continue
        if body[:2] == b'\x00\x01' and 6 in attrs(body):
            peer_requests.append((target, dport, body[8:20], attrs(body)))
        if body[:2] == b'\x01\x01' and (source, sport) in servers:
            server_responses[body[8:20]] = attrs(body)
            value = mapped(body)
            if value:
                browser_mappings.add(value)
    targets = {(a, p) for a, p, _, _ in peer_requests}
    groups=collections.defaultdict(list)
    for address,port,transaction,attributes in peer_requests:groups[(address,port)].append(transaction)
    comparisons=[{'targetIsConfiguredObserver':target in servers,'targetIsNativeMappedTuple':target in native_mappings,'checks':len(transactions),'answersFromConfiguredObserver':sum(t in server_responses for t in transactions)} for target,transactions in groups.items()]
    row = {'peerTargetGroups':comparisons,'case': name, 'nativeMappedTupleCount': len(native_mappings),
           'nativeMappedTupleIsObserver': bool(native_mappings & servers),
           'browserPeerCheckTargetTupleCount': len(targets),
           'browserChecksNativeMappedTuple': bool(targets & native_mappings),
           'browserIceChecksAnsweredByConfiguredStunServer': sum(t in server_responses for _, _, t, _ in peer_requests),
           'browserStunResponsesWithMessageIntegrity': sum(8 in v for v in server_responses.values()),
           'browserPeerRequestsWithMessageIntegrity': sum(8 in a for _, _, _, a in peer_requests),
           'browserMappingCount': len(browser_mappings),
           'noAddressesPortsCredentialsOrPayloadPublished': True}
    results.append(row)
(HERE / 'stun-mapping-correlation-v2.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results, indent=2))
