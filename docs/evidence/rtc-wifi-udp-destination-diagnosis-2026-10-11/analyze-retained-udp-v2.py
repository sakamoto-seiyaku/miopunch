import collections
import hashlib
import ipaddress
import json
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
OLD = Path('/tmp/mio82-op13-returned-runtime-root-20261008')
NEW = Path('/tmp/mio84-op13-returned-current-20261011')

def packets(path):
    data = path.read_bytes()
    order = {b'\xd4\xc3\xb2\xa1': '<', b'\xa1\xb2\xc3\xd4': '>'}[data[:4]]
    link = struct.unpack(order + 'I', data[20:24])[0]
    assert link in (1, 113, 276)
    offset = 24
    while offset < len(data):
        sec, usec, size, wire = struct.unpack(order + 'IIII', data[offset:offset + 16])
        offset += 16
        frame = data[offset:offset + size]
        offset += size
        assert len(frame) == size and size <= wire
        start = {1: 14, 113: 16, 276: 20}[link]
        ether = int.from_bytes(frame[:2] if link == 276 else frame[start - 2:start], 'big')
        if ether != 0x800:
            continue
        ip = frame[start:]
        if len(ip) < 20 or ip[0] >> 4 != 4 or ip[9] != 17:
            continue
        ihl = (ip[0] & 15) * 4
        if int.from_bytes(ip[6:8], 'big') & 0x3fff:
            continue
        udp = ip[ihl:]
        if len(udp) < 8:
            continue
        sport, dport, length, _ = struct.unpack('!HHHH', udp[:8])
        if length > len(udp):
            continue
        yield sec + usec / 1e6, ip[12:16], sport, ip[16:20], dport, udp[8:length]

rows = []
for label, root in [('previous-pass', OLD), ('current-replica-fail', NEW / 'mqtt-rtc-whole-story-first'),
                    ('current-member-fail', NEW / 'mqtt-member-only-current-v1')]:
    profile_root = OLD if label == 'previous-pass' else NEW
    profile = json.loads((profile_root / 'private-native-profile.json').read_text())
    observer = {(ipaddress.ip_address(url[5:].rsplit(':', 1)[0]).packed,int(url[5:].rsplit(':',1)[1])) for url in profile['stun_servers']}
    for role in ['native', 'browser']:
        path = root / 'runtime/private-pcap' / role / 'pre-sdk-udp-and-mqtt.pcap'
        counts = collections.Counter()
        requests, responses, peer_requests = set(), set(), set()
        pairs = collections.defaultdict(collections.Counter)
        for stamp, source, sport, target, dport, body in packets(path):
            counts['udpDatagrams'] += 1
            if len(body) >= 20 and body[4:8] == b'\x21\x12\xa4\x42':
                kind = int.from_bytes(body[:2], 'big')
                to_observer = (target, dport) in observer
                from_observer = (source, sport) in observer
                category = 'observer' if to_observer or from_observer else 'peer'
                if kind == 1:
                    counts[category + 'BindingRequests'] += 1
                    (requests if category == 'observer' else peer_requests).add(body[8:20])
                elif kind == 0x101:
                    counts[category + 'BindingSuccessResponses'] += 1
                    if category == 'observer':
                        responses.add(body[8:20])
                elif kind == 0x111:
                    counts[category + 'BindingErrorResponses'] += 1
                pairs[(source, sport, target, dport)][category] += 1
            elif len(body) >= 13 and body[0] in (20, 21, 22, 23) and body[1:3] in (b'\xfe\xff', b'\xfe\xfd'):
                counts['dtlsRecords'] += 1
                counts[{20: 'dtlsChangeCipherSpec', 21: 'dtlsAlert', 22: 'dtlsHandshake', 23: 'dtlsApplication'}[body[0]]] += 1
            else:
                counts['otherUdp'] += 1
        row = {'case': label, 'role': role, 'captureSha256': hashlib.sha256(path.read_bytes()).hexdigest(),
               'counts': dict(counts), 'distinctObserverRequests': len(requests),
               'observerRequestsWithCapturedSuccess': len(requests & responses),
               'distinctPeerRequests': len(peer_requests), 'udpIpv4OnlyAnalysis': True,
               'noEndpointOrPacketPayloadPublished': True}
        rows.append(row)
(HERE / 'retained-udp-comparison-v2.json').write_text(json.dumps(rows, indent=2) + '\n')
print(json.dumps(rows, indent=2))
