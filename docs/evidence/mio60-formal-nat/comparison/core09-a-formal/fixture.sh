#!/usr/bin/env bash
# MIOPUNCH-59: irregular APDM+APDF on independent Linux kernel gateways.
set -euo pipefail

if [[ $# -ne 4 || ${EUID} -ne 0 ]]; then
  echo "usage (as root): nat4_irregular_lab.sh CASE PROBE_BIN QAD_BIN EVIDENCE_DIR" >&2
  exit 2
fi
case_name=$1
probe=$2
qad=$3
evidence=$4
case "$case_name" in preflight|positive|tuning|blocked|wrong|stale|expire_mapping|late_control|cancel|old_round|old_receipt|network_after_plan|revoke_after_plan|unknown_mapping|jump_b|reversal|out_of_window|wide_claim|nonmember|revoked|no_secret|wrong_secret|wrong_identity|budget_exhausted) ;; *) exit 2 ;; esac
strategy=formal
seed=mio60-frozen-core09-a
allocator=/repo/scripts/legacy_nat_multi_socket_allocator.py
allocator_jobs=()
topology=wide-up1
dialer=a
case "$strategy" in formal|baseline) ;; *) exit 2 ;; esac
case "$topology" in *-*) ;; *) exit 2 ;; esac
case "$dialer" in a|b) ;; *) exit 2 ;; esac
mkdir -p "$evidence"
evidence=$(realpath "$evidence")
runtime=$(mktemp -d /tmp/miopunch60.XXXXXX)
prefix="m60$$"
wan="$prefix-wan"
nat_a="$prefix-na"
nat_b="$prefix-nb"
node_a="$prefix-a"
node_b="$prefix-b"
names=("$wan" "$nat_a" "$nat_b" "$node_a" "$node_b")
jobs=()

cleanup() {
  for job in "${jobs[@]}"; do kill -INT "$job" 2>/dev/null || true; done
  for job in "${jobs[@]}"; do wait "$job" 2>/dev/null || true; done
  if ip netns list | grep -q "^$nat_a "; then
    ip netns exec "$nat_a" nft list ruleset > "$evidence/nft-a.txt" || true
    ip netns exec "$nat_a" iptables -t raw -nvL > "$evidence/nat-a-raw.txt" || true
    ip netns exec "$nat_a" iptables -t nat -nvL > "$evidence/nat-a.txt" || true
    ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/nat-a-forward.txt" || true
    ip netns exec "$nat_a" conntrack -L -p udp > "$evidence/nat-a-conntrack.txt" 2>&1 || true
  fi
  if ip netns list | grep -q "^$nat_b "; then
    ip netns exec "$nat_b" nft list ruleset > "$evidence/nft-b.txt" || true
    ip netns exec "$nat_b" iptables -t raw -nvL > "$evidence/nat-b-raw.txt" || true
    ip netns exec "$nat_b" iptables -t nat -nvL > "$evidence/nat-b.txt" || true
    ip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/nat-b-forward.txt" || true
    ip netns exec "$nat_b" conntrack -L -p udp > "$evidence/nat-b-conntrack.txt" 2>&1 || true
  fi
  if ip netns list | grep -q "^$wan "; then
    ip netns exec "$wan" iptables -nvL FORWARD > "$evidence/wan-forward.txt" || true
  fi
  for snapshot in "$evidence"/*.txt; do
    [[ -f $snapshot ]] || continue
    sed -i 's/[[:space:]]*$//' "$snapshot"
  done
  for ns in "${names[@]}"; do
    if ip netns list | grep -q "^$ns "; then
      ip netns pids "$ns" | xargs -r kill 2>/dev/null || true
      ip netns del "$ns" 2>/dev/null || true
    fi
    rm -rf "/etc/netns/$ns"
  done
  rm -rf "$runtime"
}
trap cleanup EXIT

for ns in "${names[@]}"; do
  ip netns add "$ns"
  ip -n "$ns" link set lo up
done

link_pair() {
  local left=$1 left_if=$2 left_addr=$3 right=$4 right_if=$5 right_addr=$6
  local first="x${$}a" second="x${$}b"
  ip link add "$first" type veth peer name "$second"
  ip link set "$first" netns "$left"
  ip link set "$second" netns "$right"
  ip -n "$left" link set "$first" name "$left_if"
  ip -n "$right" link set "$second" name "$right_if"
  ip -n "$left" addr add "$left_addr" dev "$left_if"
  ip -n "$right" addr add "$right_addr" dev "$right_if"
  ip -n "$left" link set "$left_if" up
  ip -n "$right" link set "$right_if" up
}

link_pair "$wan" wan-a 100.64.27.1/30 "$nat_a" wan0 100.64.27.2/30
link_pair "$wan" wan-b 100.64.27.5/30 "$nat_b" wan0 100.64.27.6/30
link_pair "$nat_a" lan0 10.27.1.1/24 "$node_a" eth0 10.27.1.2/24
link_pair "$nat_b" lan0 10.27.2.1/24 "$node_b" eth0 10.27.2.2/24
if [[ $case_name == unknown_mapping ]]; then
  ip -n "$nat_b" addr add 100.64.27.10/32 dev wan0
  ip -n "$wan" route add 100.64.27.10/32 via 100.64.27.6
fi
ip -n "$wan" addr add 100.64.27.9/32 dev wan-a
ip -n "$node_a" route add default via 10.27.1.1
ip -n "$node_b" route add default via 10.27.2.1
ip -n "$nat_a" route add default via 100.64.27.1
ip -n "$nat_b" route add default via 100.64.27.5
for nat in "$nat_a" "$nat_b"; do
  ip netns exec "$nat" sysctl -qw net.ipv4.ip_forward=1
  ip netns exec "$nat" iptables -P FORWARD DROP
  ip netns exec "$nat" iptables -t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP
  ip netns exec "$nat" iptables -A FORWARD -i lan0 -o wan0 -j ACCEPT
  ip netns exec "$nat" iptables -A FORWARD -i wan0 -o lan0 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ip netns exec "$nat" iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
done

# Each gateway independently maps the same local UDP socket to four QAD
# destinations and the one peer destination. Only conntrack reverse tuples
# pass WAN->LAN, so filtering also depends on the peer's IP and UDP port.
mapping_parameters() {
  case "$1" in
    up1) base=42031; step=1 ;;
    down1) base=42045; step=-1 ;;
    up2) base=42031; step=2 ;;
    down2) base=42049; step=-2 ;;
    eim|eif|adf) base=42027; step=0 ;;
    random|wide) base=42031; step=0 ;;
  esac
}
a_kind=${topology%-*}
b_kind=${topology#*-}
case "$a_kind" in up1|down1|up2|down2|eim|eif|adf|random|wide) ;; *) exit 2 ;; esac
case "$b_kind" in up1|down1|up2|down2|eim|eif|adf|random|wide) ;; *) exit 2 ;; esac
mapping_parameters "$a_kind"; a_base=$base; a_step=$step
mapping_parameters "$b_kind"; b_base=$((base + 1000)); b_step=$step
if [[ $b_kind == eim || $b_kind == eif || $b_kind == adf ]]; then b_base=43027; fi
allocation_ports() {
  python3 - "$allocator" "$1" "$2" "$3" "$seed:$4" <<'ALLOC'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("allocator", "/repo/scripts/nat4_irregular_allocator.py")
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
print(*m.allocation("eim" if sys.argv[2] in ("eif", "adf") else sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5])[:6])
ALLOC
}
read -r -a a_ports <<< "$(allocation_ports "$a_kind" "$a_base" "$a_step" a)"
read -r -a b_ports <<< "$(allocation_ports "$b_kind" "$b_base" "$b_step" b)"
a_predict=${a_ports[4]}; b_predict=${b_ports[4]}
a_peer=$a_predict; b_peer=$b_predict
a_alternate=${a_ports[5]}; b_alternate=${b_ports[5]}
map_side() {
  local nat=$1 private=$2 source=$3 public=$4 kind=$5 base=$6 step=$7 peer_ip=$8 side=$9
  local ports
  read -r -a ports <<< "$(allocation_ports "$kind" "$base" "$step" "$side")"
  for index in 0 1 2 3; do
    local observer_port=$((46761 + index)) mapped_port=${ports[$index]}
    local mapped_ip=$public
    if [[ $case_name == unknown_mapping && $side == b && $index == 3 ]]; then mapped_ip=100.64.27.10; fi
    ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" \
      -d 100.64.27.1 --dport "$observer_port" -j SNAT --to-source "$mapped_ip:$mapped_port"
  done
  ip netns exec "$nat" iptables -t nat -N MIO59_PEER
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" -d "$peer_ip" -j MIO59_PEER
  ip netns exec "$nat" iptables -t mangle -A FORWARD -s "$private" -p udp -d "$peer_ip" -j NFQUEUE --queue-num 61
  ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp -d "$public" \
    -j DNAT --to-destination "$private:$source"

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
  ip netns exec "$nat" iptables -P INPUT DROP
  ip netns exec "$nat" python3 "$allocator" --kind "$([[ $kind == eif || $kind == adf ]] && echo eim || echo "$kind")" --base "$base" --step "$step" \
    --private "$private" --source "$source" --public "$public" --peer "$peer_ip" --seed "$seed:$side" \
    > "$evidence/allocator-$side.log" 2>&1 &
  allocator_jobs+=("$!"); jobs+=("$!")
}
map_side "$nat_a" 10.27.1.2 42027 100.64.27.2 "$a_kind" "$a_base" "$a_step" 100.64.27.6 a
map_side "$nat_b" 10.27.2.2 43027 100.64.27.6 "$b_kind" "$b_base" "$b_step" 100.64.27.2 b
for side in a b; do
  for _ in $(seq 1 50); do grep -q ALLOCATOR_READY "$evidence/allocator-$side.log" && break; sleep 0.1; done
  grep -q ALLOCATOR_READY "$evidence/allocator-$side.log"
done
ip netns exec "$wan" sysctl -qw net.ipv4.ip_forward=1
for node in "$node_a" "$node_b"; do
  mkdir -p "/etc/netns/$node"
  printf '127.0.0.1 localhost\n100.64.27.1 qad.miopunch.test qad2.miopunch.test qad3.miopunch.test qad4.miopunch.test\n' > "/etc/netns/$node/hosts"
done

printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
: > "$evidence/broker.log"
: > "$evidence/qad.log"
: > "$evidence/tcpdump.log"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" env MIOPUNCH_QAD_EXTRA_PORTS=46762,46763,46764 "$qad" serve "$runtime/cert.der" 100.64.27.1:46761 > "$evidence/qad.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump --immediate-mode -B 8192 -s 4096 -U -n -i any -w "$evidence/wan.pcap" 'udp and (host 100.64.27.2 or host 100.64.27.6 or host 100.64.27.10)' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for side in a b; do
  node=$node_a
  [[ $side == a ]] || node=$node_b
  ip netns exec "$node" tcpdump --immediate-mode -B 8192 -s 4096 -U -n -i any -w "$evidence/lan-$side.pcap" udp > "$evidence/tcpdump-lan-$side.log" 2>&1 &
  jobs+=("$!")
done
for _ in $(seq 1 100); do
  if grep -q QAD_READY "$evidence/qad.log" && grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"; then break; fi
  sleep 0.1
done
grep -q QAD_READY "$evidence/qad.log"
grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"
sleep 0.5

# Prove the filtering dimension with real packets, then erase the disposable
# conntrack entries so the classifier itself cannot open the product path.
send_udp() {
  local ns=$1 bind_ip=$2 bind_port=$3 target=$4 payload=$5
  ip netns exec "$ns" python3 -c 'import socket,sys; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.bind((sys.argv[1],int(sys.argv[2]))); host,port=sys.argv[3].rsplit(":",1); s.sendto(sys.argv[4].encode(),(host,int(port))); s.close()' \
    "$bind_ip" "$bind_port" "$target" "$payload"
}
reset_classifier() {

  for nat in "$nat_a" "$nat_b"; do
    ip netns exec "$nat" sh -c 'for table in /proc/net/xt_recent/m60adf*; do [ ! -f "$table" ] || echo / > "$table"; done'
  done
  ip netns exec "$nat_a" conntrack -F >/dev/null
  ip netns exec "$nat_b" conntrack -F >/dev/null
  ip netns exec "$nat_a" iptables -Z FORWARD
  ip netns exec "$nat_b" iptables -Z FORWARD
}
assert_forwarded() {
  local file=$1
  awk '$1 ~ /^[0-9]+$/ && $1 > 0 && $3 == "ACCEPT" && $6 == "wan0" && $7 == "lan0" {ok=1} END {exit !ok}' "$file"
}
assert_dropped() {
  local file=$1
  grep -Eq 'policy DROP [1-9][0-9]* packets' "$file"
}

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
  if [[ $kind == eim || $kind == eif || $kind == adf ]]; then [[ $first == "$second" ]]; else [[ $first != "$second" ]]; fi
  ip netns exec "$nat" conntrack -L -p udp > "$evidence/preflight-$side-conntrack.txt" 2>&1
  printf 'primary=%s alternate_destination=%s extra_source=%s primary_private=%s extra_private=%s\n' "$first" "$second" "$physical" "$source" "$extra" > "$evidence/preflight-$side-mapping.txt"
  for binding in "$first:$source" "$physical:$extra"; do
    mapped=${binding%:*}; local_port=${binding#*:}
    for rejection in wrong wrong-ip right; do
      ip netns exec "$nat" iptables -Z FORWARD
      case "$rejection" in
        wrong) send_udp "$other" "$peer" 50002 "$public:$mapped" "PREFLIGHT-$side-wrong-port-$local_port" ;;
        wrong-ip) send_udp "$wan" 100.64.27.9 50000 "$public:$mapped" "PREFLIGHT-$side-wrong-ip-$local_port" ;;
        right) send_udp "$other" "$peer" 50000 "$public:$mapped" "PREFLIGHT-$side-right-$local_port" ;;
      esac
      sleep 0.1
      ip netns exec "$nat" iptables -nvL FORWARD > "$evidence/filter-$side-$local_port-$rejection.txt"
      if [[ $rejection == right || $kind == eif || ( $kind == adf && $rejection == wrong ) ]]; then assert_forwarded "$evidence/filter-$side-$local_port-$rejection.txt"; else assert_dropped "$evidence/filter-$side-$local_port-$rejection.txt"; fi
    done
  done
done
reset_classifier
for job in "${allocator_jobs[@]}"; do kill -USR1 "$job"; done
for side in a b; do
  for _ in $(seq 1 50); do grep -q ALLOCATOR_RESET "$evidence/allocator-$side.log" && break; sleep 0.1; done
  grep -q ALLOCATOR_RESET "$evidence/allocator-$side.log"
done
inject_mapping_change() {
  local nat=$1 private=$2 source=$3 public=$4 peer_ip=$5 target=$6 mapped=$7
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" \
    -d "$peer_ip" --dport "$target" -j SNAT --to-source "$public:$mapped"
}
# Mutate only the application's next peer mapping, after the independent
# mapping/filter classifier has completed and its conntrack state is erased.
if [[ $case_name == reversal ]]; then
  inject_mapping_change "$nat_a" 10.27.1.2 42027 100.64.27.2 100.64.27.6 "$b_predict" "$((a_base + 2 * a_step))"
elif [[ $case_name == out_of_window ]]; then
  inject_mapping_change "$nat_a" 10.27.1.2 42027 100.64.27.2 100.64.27.6 "$b_predict" "$((a_base + 100))"
elif [[ $case_name == jump_b ]]; then
  inject_mapping_change "$nat_b" 10.27.2.2 43027 100.64.27.6 100.64.27.2 "$a_predict" "$((b_base + 7 * b_step))"
fi
if [[ $case_name == blocked ]]; then
  # Fault injection begins only after the NAT classifier, so the classifier
  # still proves APDF while the application sees a genuinely blocked WAN.
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.2 -d 100.64.27.6 -j DROP
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.6 -d 100.64.27.2 -j DROP
fi

if [[ $case_name == preflight ]]; then echo "PASS physical mapping/filter preflight"; exit 0; fi
key_dir=${MIOPUNCH_NAT_KEY_DIR:-$runtime}
mkdir -p "$key_dir"
client_key="$key_dir/client.key"
server_key="$key_dir/server.key"
client_id=$($probe identity "$client_key")
server_id=$($probe identity "$server_key")
expected_client_id=$client_id
if [[ $case_name == wrong_identity ]]; then
  expected_client_id=$($probe identity "$runtime/other.key")
fi
if [[ ! -f $key_dir/secret.bin ]]; then
  dd if=/dev/urandom of="$key_dir/secret.bin" bs=32 count=1 status=none
fi
client_secret="$key_dir/secret.bin"
server_secret="$key_dir/secret.bin"
client_auth=''
server_auth=''
if [[ $case_name == no_secret ]]; then client_secret='-'; fi
if [[ $case_name == wrong_secret ]]; then
  dd if=/dev/urandom of="$runtime/wrong-secret.bin" bs=32 count=1 status=none
  client_secret="$runtime/wrong-secret.bin"
fi
if [[ $case_name == nonmember ]]; then client_auth=nonmember; server_auth=nonmember; fi
if [[ $case_name == revoked ]]; then server_auth=revoked; fi
server_node=$node_b
server_port=43027
client_node=$node_a
client_port=42027
wrong_addr=100.64.27.6:9
if [[ $dialer == b ]]; then
  server_node=$node_a
  server_port=42027
  client_node=$node_b
  client_port=43027
  wrong_addr=100.64.27.2:9
fi
observers=''
if [[ $strategy == formal ]]; then
  observers='qad.miopunch.test:46761,qad2.miopunch.test:46762,qad3.miopunch.test:46763,qad4.miopunch.test:46764'
fi
strategy_env=(MIOPUNCH_NAT_LAB_FAILURE_LIVENESS=1)
: > "$evidence/server.log"
: > "$evidence/client.log"
ip netns exec "$server_node" env MIOPUNCH_NAT_BIND_PORT="$server_port" \
  MIOPUNCH_NAT_SECRET_FILE="$server_secret" MIOPUNCH_NAT_AUTH_FAULT="$server_auth" \
  MIOPUNCH_NAT_FAULT="$([[ $case_name == budget_exhausted ]] && echo budget_exhausted || true)" \
  "${strategy_env[@]}" MIOPUNCH_NAT_OBSERVERS="$observers" python3 "/repo/scripts/nat_connection_process.py" "$evidence/server-cpu.json" -- "$probe" server \
  "$server_key" "$runtime/cert.der" 100.64.27.1:1883 qad.miopunch.test 46761 \
  "$expected_client_id" > "$evidence/server.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 150); do
  route=$(sed -n 's/^READY id=.* route=//p' "$evidence/server.log" | head -1)
  if [[ -n $route ]]; then break; fi
  sleep 0.1
done
test -n "${route:-}"

client_args=(client "$client_key" "$runtime/cert.der" 100.64.27.1:1883 qad.miopunch.test 46761 "$server_id" "$route")
if [[ $case_name == wrong ]]; then client_args+=("$wrong_addr"); fi
fault=''
case "$case_name" in stale|expire_mapping|late_control|cancel|old_round|old_receipt|wide_claim|budget_exhausted|network_after_plan|revoke_after_plan) fault=$case_name ;; esac
set +e
ip netns exec "$client_node" env MIOPUNCH_NAT_BIND_PORT="$client_port" \
  MIOPUNCH_NAT_SECRET_FILE="$client_secret" MIOPUNCH_NAT_AUTH_FAULT="$client_auth" \
  "${strategy_env[@]}" MIOPUNCH_NAT_FAULT="$fault" \
  MIOPUNCH_NAT_OBSERVERS="$observers" \
  MIOPUNCH_NAT_REPLAY_ROUND="${MIOPUNCH_NAT_REPLAY_ROUND:-00000000000000000000000000000000}" python3 "/repo/scripts/nat_connection_process.py" "$evidence/client-cpu.json" -- \
  "$probe" "${client_args[@]}" > "$evidence/client.log" 2>&1
client_rc=$?
wait "${jobs[-1]}"
server_rc=$?
set -e
printf 'case=%s strategy=%s topology=%s dialer=%s client_rc=%s server_rc=%s seed=%s\n' \
  "$case_name" "$strategy" "$topology" "$dialer" "$client_rc" "$server_rc" "$seed" > "$evidence/run.txt"
[[ $client_rc != 124 && $server_rc != 124 ]]
sleep 0.3


if [[ $case_name == positive || $case_name == blocked ]]; then
  [[ $client_rc == 0 && $server_rc == 0 ]]
  for side in client server; do
    grep -Eq '^(LEGACY_SELECTED|NAT_DIRECT_SUCCESS) ' "$evidence/$side.log"
    if grep -q DATA_OK "$evidence/$side.log"; then
      grep -Eq '^(LEGACY_BUSINESS_BINDING|NAT_DIRECT_SUCCESS) ' "$evidence/$side.log"
    else
      grep -q '^PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed' "$evidence/$side.log"
      grep -Eq '^(CONTROL_ALIVE_AFTER_UDP_FAILURE|EARLY_FAILURE_CONTROL_ALIVE) stable_id=' "$evidence/$side.log"
    fi
  done
else
  ! grep -q DATA_OK "$evidence/client.log"
  ! grep -q DATA_OK "$evidence/server.log"
fi
echo "PASS formal case=$case_name topology=$topology dialer=$dialer seed=$seed"
