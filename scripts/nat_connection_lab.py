#!/usr/bin/env python3
"""Formal no-override connection entry in the retained physical NAT fixture."""
import argparse
import os
import pathlib
import subprocess
import tempfile
from legacy_nat_multi_socket_lab import fixture
from legacy_nat_lab import replace_once


def make_fixture(repo, topology, dialer, seed, strategy="formal"):
    # Reuse the physical tuple/allocator seam, extending the EIM filter
    # dimension to the old NAT1(EIF) and NAT2(ADF) profiles.
    source = fixture(repo, topology, dialer, 0, seed)
    source = source.replace('eim) base=42027; step=0', 'eim|eif|adf) base=42027; step=0')
    source = source.replace('up1|down1|up2|down2|eim|random|wide)', 'up1|down1|up2|down2|eim|eif|adf|random|wide)')
    source = source.replace('if [[ $b_kind == eim ]];', 'if [[ $b_kind == eim || $b_kind == eif || $b_kind == adf ]];')
    source = source.replace('m.allocation(sys.argv[2],', 'm.allocation("eim" if sys.argv[2] in ("eif", "adf") else sys.argv[2],')
    source = source.replace('--kind "$kind" --base', '--kind "$([[ $kind == eif || $kind == adf ]] && echo eim || echo "$kind")" --base')
    source = source.replace('if [[ $kind == eim ]];', 'if [[ $kind == eim || $kind == eif || $kind == adf ]];')
    source = replace_once(source, 'ip -n "$node_a" route add default via 10.27.1.1',
                          'ip -n "$wan" addr add 100.64.27.9/32 dev wan-a\nip -n "$node_a" route add default via 10.27.1.1')
    source = replace_once(source, 'wrong-ip) send_udp "$wan" 100.64.27.1 50000', 'wrong-ip) send_udp "$wan" 100.64.27.9 50000')
    source = replace_once(source,
        'if [[ $rejection == right ]]; then assert_forwarded "$evidence/filter-$side-$local_port-$rejection.txt"; else assert_dropped "$evidence/filter-$side-$local_port-$rejection.txt"; fi',
        'if [[ $rejection == right || $kind == eif || ( $kind == adf && $rejection == wrong ) ]]; then assert_forwarded "$evidence/filter-$side-$local_port-$rejection.txt"; else assert_dropped "$evidence/filter-$side-$local_port-$rejection.txt"; fi')
    source = replace_once(source, '  ip netns exec "$nat" iptables -P INPUT DROP', r'''
  # The independent filter preflight originates exact public source tuples at
  # the gateway. Do not let an EIF-created reverse entry/MASQUERADE rewrite
  # these probes. Node-forwarded product packets never enter this OUTPUT rule.
  ip netns exec "$nat" iptables -t raw -A OUTPUT -s "$public" -p udp -j NOTRACK
  if [[ $kind == eim || $kind == eif || $kind == adf ]]; then
    # EIM keeps each physical source port. Preserve unmatched cone destination
    # ports instead of merging extra sockets into the primary socket.
    ip netns exec "$nat" iptables -t nat -D PREROUTING -i wan0 -p udp -d "$public" -j DNAT --to-destination "$private:$source"
    ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp -d "$public" -j DNAT --to-destination "$private"
  fi
  if [[ $kind == eif ]]; then
    ip netns exec "$nat" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp -j ACCEPT
  elif [[ $kind == adf ]]; then
    for filter_port in "$source" "$((source + 100))"; do
      # NFQUEUE is a terminal verdict for this chain. Record the per-source
      # contacted IP before queueing, rather than after reinjection.
      ip netns exec "$nat" iptables -t mangle -I FORWARD 1 -i lan0 -o wan0 -p udp --sport "$filter_port" -m recent --name "m60adf$filter_port" --set --rdest
      ip netns exec "$nat" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp --dport "$filter_port" -m recent --name "m60adf$filter_port" --rcheck --seconds 120 --rsource -j ACCEPT
    done
  fi
  ip netns exec "$nat" iptables -P INPUT DROP''')
    source = replace_once(source, '  ip netns exec "$nat_a" conntrack -F >/dev/null', r'''
  for nat in "$nat_a" "$nat_b"; do
    ip netns exec "$nat" sh -c 'for table in /proc/net/xt_recent/m60adf*; do [ ! -f "$table" ] || echo / > "$table"; done'
  done
  ip netns exec "$nat_a" conntrack -F >/dev/null''')
    source = source.replace("miopunch61", "miopunch60").replace('prefix="m61$$"', 'prefix="m60$$"')
    source = replace_once(source, "strategy=legacy", "strategy=" + strategy)
    source = replace_once(source, 'case "$strategy" in legacy)', 'case "$strategy" in formal|baseline)')
    source = replace_once(source,
                          'strategy_env=(MIOPUNCH_NAT_STRATEGY=legacy MIOPUNCH_LEGACY_PREVIOUS_FAILURES=0 MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)',
                          'strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)' if strategy == "formal" else
                          'strategy_env=(MIOPUNCH_NAT_STRATEGY=baseline MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)')
    source = source.replace('if [[ $strategy == legacy ]]; then', 'if [[ $strategy == formal ]]; then')
    wrapper = str(repo / "scripts/nat_connection_process.py")
    source = replace_once(source, 'timeout 65 "$probe" server',
                          f'python3 "{wrapper}" "$evidence/server-cpu.json" -- "$probe" server')
    source = replace_once(source, 'MIOPUNCH_NAT_REPLAY_ROUND="${MIOPUNCH_NAT_REPLAY_ROUND:-00000000000000000000000000000000}" timeout 65',
                          f'MIOPUNCH_NAT_REPLAY_ROUND="${{MIOPUNCH_NAT_REPLAY_ROUND:-00000000000000000000000000000000}}" python3 "{wrapper}" "$evidence/client-cpu.json" --')
    source = source.replace("  [[ $client_rc == 0 && $server_rc == 0 ]]\n  for side", "  [[ $client_rc == 0 && $server_rc == 0 ]]\n  for side")
    source = source.replace('    grep -q \'^LEGACY_SELECTED \' "$evidence/$side.log"',
                            '    grep -Eq \'^(LEGACY_SELECTED|NAT_DIRECT_SUCCESS) \' "$evidence/$side.log"')
    source = source.replace('      grep -q \'^LEGACY_BUSINESS_BINDING \' "$evidence/$side.log"',
                            '      grep -Eq \'^(LEGACY_BUSINESS_BINDING|NAT_DIRECT_SUCCESS) \' "$evidence/$side.log"')
    source = source.replace("PASS multisocket", "PASS formal")
    if strategy == "baseline":
        start = source.index('if [[ $case_name == positive || $case_name == blocked ]]; then')
        source = source[:start] + r'''
[[ $client_rc == 0 && $server_rc == 0 ]]
for side in client server; do
  grep -Eq '^(DATA_OK|PATH_UNAVAILABLE) ' "$evidence/$side.log"
done
echo "PASS baseline case=$case_name topology=$topology dialer=$dialer seed=$seed"
'''
    return source


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("case")
    parser.add_argument("topology")
    parser.add_argument("dialer", choices=["a", "b"])
    parser.add_argument("seed")
    parser.add_argument("runtime", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--strategy", choices=["formal", "baseline"], default="formal")
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--no-observers", action="store_true", help="functional resource-absence fixture only")
    args = parser.parse_args()
    if os.geteuid() != 0 or args.output.exists():
        parser.error("require disposable privileged fixture and new output path")
    if not all(c.isalnum() or c in "-_" for c in args.seed):
        parser.error("invalid seed identifier")
    repo = pathlib.Path(__file__).resolve().parents[1]
    source = make_fixture(repo, args.topology, args.dialer, args.seed, args.strategy)
    if args.no_observers:
        if args.strategy != "formal" or args.rounds != 1:
            parser.error("no-observers is one formal functional round")
        source = source.replace('strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)',
                                'strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1 MIOPUNCH_NAT_LAB_NO_OBSERVERS=1)')
    if args.rounds != 1:
        if not 1 <= args.rounds <= 32 or args.strategy != "formal":
            parser.error("invalid bounded functional round count")
        source = source.replace('strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)',
                                f'strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1 MIOPUNCH_NAT_LAB_ROUNDS={args.rounds})')
        source = source.replace('"$evidence/server-cpu.json"', f'--limit {args.rounds * 60 + 15} "$evidence/server-cpu.json"')
        source = source.replace('"$evidence/client-cpu.json"', f'--limit {args.rounds * 60 + 15} "$evidence/client-cpu.json"')
    args.output.mkdir(parents=True)
    (args.output / "fixture.sh").write_text(source)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".sh", prefix="mio60-") as script:
        script.write(source)
        script.flush()
        result = subprocess.run(["bash", script.name, args.case,
                                 str(args.runtime / "udp_nat_probe"),
                                 str(args.runtime / "udp_observation_probe"), str(args.output)])
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
