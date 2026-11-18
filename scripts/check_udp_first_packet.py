#!/usr/bin/env python3
"""MIOPUNCH-139 isolated kernel-NAT ordering and handoff experiment.

Run inside the existing privileged NAT lab with explicitly pinned probe tools.
The receiver's complete LAN capture, rather than its scheduler log, establishes
whether it sent any peer UDP before the first authenticated traversal request.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import struct
import subprocess

from legacy_nat_lab import replace_once


def fixture(repo, dialer, receiver_filter, delay_ms, gate_delay=False):
    source = (repo / "scripts/nat3_lab.sh").read_text()
    source = replace_once(source, 'strategy=${MIOPUNCH_NAT_STRATEGY:-baseline}', 'strategy=legacy')
    source = replace_once(source, 'dialer=${MIOPUNCH_NAT_DIALER:-a}', f'dialer={dialer}')
    source = replace_once(source, 'baseline|coordinated)', 'baseline|coordinated|legacy)')
    source = source.replace('kill -INT', 'kill -TERM')
    source = source.replace('tcpdump -U -n -i any', 'tcpdump --immediate-mode -U -n -i any')
    # Keep classifier traffic out of the experiment capture.
    source = source.replace('-w "$evidence/wan.pcap"', '-w "$evidence/preflight.pcap"')
    anchor = 'if [[ $case_name == blocked ]]; then\n'
    setup = '''
# The QAD server remains available to supply its certificate. Only its UDP
# observations are blackholed, so both groups actually take QAD -> STUN.
ip netns exec "$wan" iptables -A INPUT -p udp --dport 46761 -j DROP
ip netns exec "$wan" python3 /repo/scripts/stun_udp_observer.py --bind 100.64.27.1:3478 --bind 100.64.27.1:3479 > "$evidence/stun.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 50); do
  if [[ $(grep -c STUN_UDP_READY "$evidence/stun.log") == 2 ]]; then break; fi
  sleep 0.1
done
[[ $(grep -c STUN_UDP_READY "$evidence/stun.log") == 2 ]]
ip netns exec "$wan" tcpdump --immediate-mode -U -n -i any -w "$evidence/wan.pcap" udp > "$evidence/wan-capture.log" 2>&1 &
jobs+=("$!")
for side in a b; do
  node_var="node_$side"
  ip netns exec "${!node_var}" tcpdump --immediate-mode -U -n -i eth0 -w "$evidence/lan-$side.pcap" udp > "$evidence/lan-$side.log" 2>&1 &
  jobs+=("$!")
done
'''
    # coordinate_server owns selector c=Sender; the QUIC client is Receiver.
    if receiver_filter == "cone":
        nat, private, port = ("nat_a", "10.27.1.2", 42027) if dialer == "a" else ("nat_b", "10.27.2.2", 43027)
        setup += f'ip netns exec "${nat}" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp -d {private} --dport {port} -j ACCEPT\n'
    if delay_ms and gate_delay:
        setup += '''
export MIOPUNCH_NAT_LAB_PHASE_GATE="$runtime/phase.gate"
(
  for _ in $(seq 1 600); do
    if grep -q NAT_LAB_PHASE_GATE_READY "$evidence/server.log" 2>/dev/null && grep -q NAT_LAB_PHASE_GATE_READY "$evidence/client.log" 2>/dev/null; then
'''
        for interface in ("wan-a", "wan-b"):
            setup += f'      ip netns exec "$wan" tc qdisc add dev {interface} root netem delay {delay_ms}ms\n'
        setup += '''      date +%s.%N > "$evidence/delay-installed-at.txt"
      touch "$runtime/phase.gate"
      exit 0
    fi
    sleep 0.05
  done
  echo 'both authenticated setup gates were not reached' >&2
  exit 1
) > "$evidence/delay-injection.log" 2>&1 &
jobs+=("$!")
'''
    elif delay_ms:
        for interface in ("wan-a", "wan-b"):
            setup += f'ip netns exec "$wan" tc qdisc add dev {interface} root netem delay {delay_ms}ms\n'
    source = replace_once(source, anchor, setup + '\n' + anchor)
    # Driver evaluates both successes and honest traversal misses. The retained
    # fixture's nonzero/timeout exits still fail the experiment itself.
    source = source[:source.index('case "$case_name" in\n  positive)')]
    source += '\n[[ $client_rc == 0 && $server_rc == 0 ]]\n'
    return source


def packets(path):
    """Read Ethernet or SLL2 IPv4 UDP captures without another dependency."""
    with path.open('rb') as handle:
        head = handle.read(24)
        endian = '<' if head[:4] == b'\xd4\xc3\xb2\xa1' else '>'
        link = struct.unpack(endian + 'I', head[20:24])[0]
        offset, protocol = {1: (14, 12), 276: (20, 0)}[link]
        while record := handle.read(16):
            sec, micro, length, _ = struct.unpack(endian + 'IIII', record)
            frame = handle.read(length)
            if frame[protocol:protocol + 2] != b'\x08\x00':
                continue
            ip = frame[offset:]
            if len(ip) < 28 or ip[9] != 17:
                continue
            ihl = (ip[0] & 15) * 4
            sport, dport, size = struct.unpack('!HHH', ip[ihl:ihl + 6])
            payload = ip[ihl + 8:ihl + size]
            raw = payload.startswith(b'\0MIOPUNCH63') and len(payload) >= 76
            yield dict(at=sec + micro / 1e6, src=str(ipaddress.IPv4Address(ip[12:16])),
                       dst=str(ipaddress.IPv4Address(ip[16:20])), sport=sport, dport=dport,
                       raw=raw, round=payload[11:27].hex() if raw else None,
                       response=bool(payload[27]) if raw else None, bytes=len(payload))


def assess(output, args):
    logs = {side: (output / f'{side}.log').read_text() for side in ('client', 'server')}
    for side, log in logs.items():
        assert 'LEGACY_OBSERVATION_FALLBACK' in log, f'{side}: QAD did not fail'
        assert len(re.findall(r'^LEGACY_SAMPLE ', log, re.M)) == 2, f'{side}: wrong STUN source count'
        assert 'LEGACY_SELECTED mode=0 index=0 ' in log, f'{side}: changed role/candidates'
    rounds = [re.search(r'LEGACY_SELECTED .*?round=(\w+)', logs[side])[1] for side in logs]
    assert rounds[0] == rounds[1], 'different traversal rounds'
    receiver = args.dialer
    private, peer = ('10.27.1.2', '100.64.27.6') if receiver == 'a' else ('10.27.2.2', '100.64.27.2')
    lan = list(packets(output / f'lan-{receiver}.pcap'))
    inbound = [p for p in lan if p['src'] == peer and p['dst'] == private and p['raw'] and p['round'] == rounds[0] and not p['response']]
    outbound = [p for p in lan if p['src'] == private and p['dst'] == peer]
    first = min((p['at'] for p in inbound), default=None)
    before = [p for p in outbound if first is None or p['at'] < first]
    if args.wait:
        assert not before, 'receiver sent UDP (including native) before the first request'
    business = {side: 'DATA_OK ' in log for side, log in logs.items()}
    for side, log in logs.items():
        assert business[side] or 'EARLY_FAILURE_CONTROL_ALIVE ' in log, f'{side}: failure lost original control'
    wan = list(packets(output / 'wan.pcap'))
    replies = [p for p in wan if p['round'] == rounds[0] and p['response']]
    return dict(round=rounds[0], receiver=receiver, receiverFilter=args.receiver_filter,
                receiverWait=args.wait, earlyValidation=args.early, linkDelayMs=args.delay_ms,
                delayAfterSetup=args.gate_delay,
                protectedControlRttMs={side: int(m[1]) if (m := re.search(r'NAT_LAB_CONTROL_RTT .*maximum_ms=(\d+)', log)) else None for side, log in logs.items()},
                phaseMs=args.phase_ms, firstRequestReceived=first is not None,
                peerUdpBeforeFirstRequest=len(before), nativeUdpBeforeFirstRequest=sum(not p['raw'] for p in before),
                rawRequestReceived='LEGACY_RECEIPT ' in logs['client'],
                rawResponseReceived=bool(re.search(r'LEGACY_RECEIPT .*response=true', logs['server'])),
                replyPublicSources=sorted({f"{p['src']}:{p['sport']}" for p in replies}),
                failureControlPreserved={side: 'EARLY_FAILURE_CONTROL_ALIVE ' in log for side, log in logs.items()}, business=business,
                handoffMs={side: int(m[1]) if (m := re.search(r'DATA_OK .*?path_ms=(\d+)', log)) else None for side, log in logs.items()})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--qad', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--dialer', choices=['a', 'b'], default='a')
    parser.add_argument('--receiver-filter', choices=['apdf', 'cone'], default='apdf')
    parser.add_argument('--wait', action='store_true')
    parser.add_argument('--early', action='store_true')
    parser.add_argument('--delay-ms', type=int, default=0)
    parser.add_argument('--phase-ms', type=int, default=5000)
    parser.add_argument('--gate-delay', action='store_true', help='inject delay only after both authenticated setups; measure protected control RTT before raw phase')
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('run inside the disposable privileged NAT lab')
    # Stop before touching namespaces when frozen artifact inputs are absent.
    for path in (args.probe, args.qad):
        if not path.is_file() or not os.access(path, os.X_OK):
            parser.error(f'missing executable: {path}')
    args.output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[1]
    script = args.output / 'fixture.sh'
    script.write_text(fixture(repo, args.dialer, args.receiver_filter, args.delay_ms, args.gate_delay))
    env = dict(os.environ, MIOPUNCH_NAT_LAB_STUN='stun://100.64.27.1:3478/,stun://100.64.27.1:3479/',
               MIOPUNCH_NAT_LAB_RECEIVER_WAIT=str(int(args.wait)), MIOPUNCH_NAT_LAB_EARLY_VALIDATE=str(int(args.early)),
               MIOPUNCH_NAT_LAB_PHASE_MS=str(args.phase_ms), MIOPUNCH_NAT_LAB_FAILURE_LIVENESS='1')
    result = subprocess.run(['bash', str(script), 'positive', str(args.probe), str(args.qad), str(args.output)],
                            env=env, capture_output=True, text=True)
    (args.output / 'runner.log').write_text(result.stdout + result.stderr)
    if result.returncode:
        raise SystemExit(f'fixture failed exit={result.returncode}; see {args.output}/runner.log')
    report = assess(args.output, args)
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
