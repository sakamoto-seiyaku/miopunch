#!/usr/bin/env python3
"""MIOPUNCH-61 kernel fixture: all physical sources, full tuple NAT/APDF."""
import argparse
import os
import pathlib
import subprocess
import tempfile
from legacy_nat_lab import replace_once


PREFLIGHT = r'''
# Independently prove mapping for two physical source ports and two remote
# ports. The application never sees these disposable conntrack entries.
mapped_port() {
  local nat=$1 private=$2 source=$3 peer=$4 destination=$5
  ip netns exec "$nat" conntrack -L -p udp 2>/dev/null | python3 -c 'import sys,re
private,source,peer,destination=sys.argv[1:]
for line in sys.stdin:
 if f"src={private} dst={peer} sport={source} dport={destination} " in line:
  pairs=re.findall(r"sport=(\d+) dport=(\d+)",line);print(pairs[1][1]);break' "$private" "$source" "$peer" "$destination"
}
for side in a b; do
  nat=$nat_a; node=$node_a; private=10.27.1.2; source=42027; public=100.64.27.2; peer=100.64.27.6; other=$nat_b; kind=$a_kind
  if [[ $side == b ]]; then nat=$nat_b; node=$node_b; private=10.27.2.2; source=43027; public=100.64.27.6; peer=100.64.27.2; other=$nat_a; kind=$b_kind; fi
  extra=$((source + 100))
  send_udp "$node" "$private" "$source" "$peer:50000" "PREFLIGHT-$side-primary-port1"
  send_udp "$node" "$private" "$source" "$peer:50001" "PREFLIGHT-$side-primary-port2"
  send_udp "$node" "$private" "$extra" "$peer:50000" "PREFLIGHT-$side-extra-port1"
  send_udp "$node" "$private" "$source" "100.64.27.1:46761" "PREFLIGHT-$side-other-ip"
  sleep 0.1
  first=$(mapped_port "$nat" "$private" "$source" "$peer" 50000)
  second=$(mapped_port "$nat" "$private" "$source" "$peer" 50001)
  physical=$(mapped_port "$nat" "$private" "$extra" "$peer" 50000)
  [[ -n $first && -n $second && -n $physical && $first != "$physical" ]]
  if [[ $kind == eim ]]; then [[ $first == "$second" ]]; else [[ $first != "$second" ]]; fi
  ip netns exec "$nat" conntrack -L -p udp > "$evidence/preflight-$side-conntrack.txt" 2>&1
  printf 'primary=%s alternate_destination=%s extra_source=%s primary_private=%s extra_private=%s\n' "$first" "$second" "$physical" "$source" "$extra" > "$evidence/preflight-$side-mapping.txt"
  for binding in "$first:$source" "$physical:$extra"; do
    mapped=${binding%:*}; local_port=${binding#*:}
    for rejection in wrong wrong-ip right; do
      ip netns exec "$nat" iptables -Z FORWARD
      case "$rejection" in
        wrong) send_udp "$other" "$peer" 50002 "$public:$mapped" "PREFLIGHT-$side-wrong-port-$local_port" ;;
        wrong-ip) send_udp "$wan" 100.64.27.1 50000 "$public:$mapped" "PREFLIGHT-$side-wrong-ip-$local_port" ;;
        right) send_udp "$other" "$peer" 50000 "$public:$mapped" "PREFLIGHT-$side-right-$local_port" ;;
      esac
      sleep 0.1
      ip netns exec "$nat" iptables -nvL FORWARD > "$evidence/filter-$side-$local_port-$rejection.txt"
      if [[ $rejection == right ]]; then assert_forwarded "$evidence/filter-$side-$local_port-$rejection.txt"; else assert_dropped "$evidence/filter-$side-$local_port-$rejection.txt"; fi
    done
  done
done
reset_classifier
for job in "${allocator_jobs[@]}"; do kill -USR1 "$job"; done
for side in a b; do
  for _ in $(seq 1 50); do grep -q ALLOCATOR_RESET "$evidence/allocator-$side.log" && break; sleep 0.1; done
  grep -q ALLOCATOR_RESET "$evidence/allocator-$side.log"
done
'''


def fixture(repo, topology, dialer, history, seed):
    source = (repo / "scripts/nat4_irregular_lab.sh").read_text()
    source = replace_once(source, "strategy=${MIOPUNCH_NAT_STRATEGY:-auto}", "strategy=legacy")
    source = replace_once(source, "seed=${MIOPUNCH_NAT_TEST_SEED:-mio59-default}", "seed=" + seed)
    source = replace_once(source, "allocator=$(dirname \"$0\")/nat4_irregular_allocator.py", "allocator=" + str(repo / "scripts/legacy_nat_multi_socket_allocator.py"))
    source = replace_once(source, "topology=${MIOPUNCH_NAT_TOPOLOGY:-up1-up1}", "topology=" + topology)
    source = replace_once(source, "dialer=${MIOPUNCH_NAT_DIALER:-a}", "dialer=" + dialer)
    source = replace_once(source, "baseline|auto)", "legacy)")
    source = source.replace("/tmp/miopunch59.", "/tmp/miopunch61.").replace('prefix="m59$$"', 'prefix="m61$$"')
    source = replace_once(source, 'spec = importlib.util.spec_from_file_location("allocator", sys.argv[1])', 'spec = importlib.util.spec_from_file_location("allocator", "' + str(repo / "scripts/nat4_irregular_allocator.py") + '")')
    # Every outbound physical source is queued; no fixed primary sport match.
    source = replace_once(source, '-p udp --sport "$source" -d "$peer_ip" -j NFQUEUE --queue-num 59', '-p udp -d "$peer_ip" -j NFQUEUE --queue-num 61')
    source = replace_once(source, '  ip netns exec "$nat" iptables -P FORWARD DROP', '  ip netns exec "$nat" iptables -P FORWARD DROP\n  ip netns exec "$nat" iptables -t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP')
    start = source.index("# Prove address-and-port-dependent mapping with two distinct destination")
    end = source.index("inject_mapping_change()", start)
    source = source[:start] + PREFLIGHT + source[end:]
    source = replace_once(source, 'key_dir=${MIOPUNCH_NAT_KEY_DIR:-$runtime}', 'if [[ $case_name == preflight ]]; then echo "PASS physical mapping/filter preflight"; exit 0; fi\nkey_dir=${MIOPUNCH_NAT_KEY_DIR:-$runtime}')
    source = replace_once(source, 'positive|tuning|blocked|wrong|', 'preflight|positive|tuning|blocked|wrong|')
    source = source.replace("if [[ $strategy == auto ]]; then", "if [[ $strategy == legacy ]]; then")
    source = replace_once(source, 'strategy_env=()\nif [[ $strategy == baseline ]]; then strategy_env=(MIOPUNCH_NAT_STRATEGY=baseline); fi', f'strategy_env=(MIOPUNCH_NAT_STRATEGY=legacy MIOPUNCH_LEGACY_PREVIOUS_FAILURES={history} MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)')
    source = source.replace("timeout 40", "timeout 65")
    source = replace_once(source, 'ip netns exec "$server_node" env MIOPUNCH_NAT_BIND_PORT=', ': > "$evidence/server.log"\n: > "$evidence/client.log"\nip netns exec "$server_node" env MIOPUNCH_NAT_BIND_PORT=')
    # Keep existing native authorization/budget/network-fault entry points.
    end = source.index('case "$case_name" in\n  tuning)')
    source = source[:end] + r'''
if [[ $case_name == positive || $case_name == blocked ]]; then
  [[ $client_rc == 0 && $server_rc == 0 ]]
  for side in client server; do
    grep -q '^LEGACY_SELECTED ' "$evidence/$side.log"
    if grep -q DATA_OK "$evidence/$side.log"; then
      grep -q '^LEGACY_BUSINESS_BINDING ' "$evidence/$side.log"
    else
      grep -q '^PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed' "$evidence/$side.log"
      grep -Eq '^(CONTROL_ALIVE_AFTER_UDP_FAILURE|EARLY_FAILURE_CONTROL_ALIVE) stable_id=' "$evidence/$side.log"
    fi
  done
else
  ! grep -q DATA_OK "$evidence/client.log"
  ! grep -q DATA_OK "$evidence/server.log"
fi
echo "PASS multisocket case=$case_name topology=$topology dialer=$dialer seed=$seed"
'''
    # Cleanup retains exact kernel rules, including dynamic SNAT and TTL loss.
    source = source.replace('    ip netns exec "$nat_a" iptables -t nat -nvL', '    ip netns exec "$nat_a" nft list ruleset > "$evidence/nft-a.txt" || true\n    ip netns exec "$nat_a" iptables -t raw -nvL > "$evidence/nat-a-raw.txt" || true\n    ip netns exec "$nat_a" iptables -t nat -nvL', 1)
    source = source.replace('    ip netns exec "$nat_b" iptables -t nat -nvL', '    ip netns exec "$nat_b" nft list ruleset > "$evidence/nft-b.txt" || true\n    ip netns exec "$nat_b" iptables -t raw -nvL > "$evidence/nat-b-raw.txt" || true\n    ip netns exec "$nat_b" iptables -t nat -nvL', 1)
    return source


def main():
    p = argparse.ArgumentParser()
    p.add_argument("case")
    p.add_argument("topology")
    p.add_argument("dialer", choices=["a", "b"])
    p.add_argument("history", type=int)
    p.add_argument("seed")
    p.add_argument("probe", type=pathlib.Path)
    p.add_argument("qad", type=pathlib.Path)
    p.add_argument("evidence", type=pathlib.Path)
    a = p.parse_args()
    if os.geteuid() != 0:
        p.error("run in disposable privileged NAT lab")
    if not all(c.isalnum() or c in "-_" for c in a.seed):
        p.error("seed must be a shell-safe frozen identifier")
    repo = pathlib.Path(__file__).resolve().parents[1]
    source = fixture(repo, a.topology, a.dialer, a.history, a.seed)
    a.evidence.mkdir(parents=True, exist_ok=True)
    (a.evidence / "fixture.sh").write_text(source)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".sh", prefix="miopunch61-") as script:
        script.write(source)
        script.flush()
        result = subprocess.run(["bash", script.name, a.case, str(a.probe), str(a.qad), str(a.evidence)])
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
