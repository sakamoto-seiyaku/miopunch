#!/usr/bin/env bash
# MIOPUNCH-58: bilateral APDM+APDF on independent Linux kernel gateways.
set -euo pipefail

if [[ $# -ne 4 || ${EUID} -ne 0 ]]; then
  echo "usage (as root): nat4_pair_lab.sh CASE PROBE_BIN QAD_BIN EVIDENCE_DIR" >&2
  exit 2
fi
case_name=$1
probe=$2
qad=$3
evidence=$4
case "$case_name" in positive|tuning|blocked|wrong|stale|expire_mapping|late_control|cancel|old_round|unknown_mapping|jump_b|reversal|out_of_window|wide_claim|nonmember|revoked|no_secret|wrong_secret) ;; *) exit 2 ;; esac
strategy=legacy
topology=up2-up2
dialer=b
case "$strategy" in legacy) ;; *) exit 2 ;; esac
case "$topology" in *-*) ;; *) exit 2 ;; esac
case "$dialer" in a|b) ;; *) exit 2 ;; esac
mkdir -p "$evidence"
evidence=$(realpath "$evidence")
runtime=$(mktemp -d /tmp/miopunch58.XXXXXX)
prefix="m58$$"
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
    ip netns exec "$nat_a" iptables -t raw -nvL PREROUTING > "$evidence/nat-a-raw.txt" || true
    ip netns exec "$nat_a" iptables -t nat -nvL POSTROUTING > "$evidence/nat-a.txt" || true
    ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/nat-a-forward.txt" || true
    ip netns exec "$nat_a" conntrack -L -p udp > "$evidence/nat-a-conntrack.txt" 2>&1 || true
  fi
  if ip netns list | grep -q "^$nat_b "; then
    ip netns exec "$nat_b" iptables -t raw -nvL PREROUTING > "$evidence/nat-b-raw.txt" || true
    ip netns exec "$nat_b" iptables -t nat -nvL POSTROUTING > "$evidence/nat-b.txt" || true
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
    eim) base=42027; step=0 ;;
  esac
}
a_kind=${topology%-*}
b_kind=${topology#*-}
case "$a_kind" in up1|down1|up2|down2|eim) ;; *) exit 2 ;; esac
case "$b_kind" in up1|down1|up2|down2|eim) ;; *) exit 2 ;; esac
mapping_parameters "$a_kind"; a_base=$base; a_step=$step
mapping_parameters "$b_kind"; b_base=$((base + 1000)); b_step=$step
if [[ $b_kind == eim ]]; then b_base=43027; fi
a_predict=$((a_base + 4 * a_step))
b_predict=$((b_base + 4 * b_step))
a_alternate=$((a_base + 6 * a_step))
b_alternate=$((b_base + 6 * b_step))
a_peer=$a_predict
b_peer=$b_predict
map_side() {
  local nat=$1 private=$2 source=$3 public=$4 base=$5 step=$6 peer_ip=$7 peer_target=$8 peer_mapped=$9 peer_alternate=${10}
  for index in 0 1 2 3; do
    local observer_port=$((46761 + index))
    local mapped_port=$((base + index * step))
    if [[ $case_name == unknown_mapping && $nat == "$nat_b" && $index -eq 3 ]]; then
      mapped_port=$((base + step))
    fi
    ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" \
      -d 100.64.27.1 --dport "$observer_port" -j SNAT --to-source "$public:$mapped_port"
  done
  # The fallback catches baseline's stale QAD target and every other peer
  # destination port. It uses an explicitly different mapping on regular
  # gateways; MASQUERADE never decides a peer mapping by accident.
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" \
    -d "$peer_ip" -j SNAT --to-source "$public:$peer_alternate"
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$source" \
    -d "$peer_ip" --dport "$peer_target" -j SNAT --to-source "$public:$peer_mapped"
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$public" -p udp -j ACCEPT
  # Route even a wrong-source peer packet toward the LAN filter, so the
  # FORWARD counter independently proves APDF rather than gateway INPUT drop.
  ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp --dport "$peer_mapped" \
    -j DNAT --to-destination "$private:$source"
  ip netns exec "$nat" iptables -P INPUT DROP
}
map_side "$nat_a" 10.27.1.2 42027 100.64.27.2 "$a_base" "$a_step" 100.64.27.6 "$b_predict" "$a_peer" "$a_alternate"
map_side "$nat_b" 10.27.2.2 43027 100.64.27.6 "$b_base" "$b_step" 100.64.27.2 "$a_predict" "$b_peer" "$b_alternate"
ip netns exec "$wan" sysctl -qw net.ipv4.ip_forward=1
for node in "$node_a" "$node_b"; do
  mkdir -p "/etc/netns/$node"
  printf '127.0.0.1 localhost\n100.64.27.1 qad.miopunch.test qad2.miopunch.test qad3.miopunch.test qad4.miopunch.test\n' > "/etc/netns/$node/hosts"
done

printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
: > "$evidence/broker.log"
: > "$evidence/qad.log"
: > "$evidence/tcpdump.log"
for side in a b; do
  node_var="node_$side"
  ip netns exec "${!node_var}" tcpdump --immediate-mode -U -n -i eth0 -w "$evidence/lan-$side.pcap" udp > "$evidence/tcpdump-lan-$side.log" 2>&1 &
  jobs+=("$!")
done
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" env MIOPUNCH_QAD_EXTRA_PORTS=46762,46763,46764 "$qad" serve "$runtime/cert.der" 100.64.27.1:46761 > "$evidence/qad.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump --immediate-mode -U -n -i any -w "$evidence/wan.pcap" 'udp and (host 100.64.27.2 or host 100.64.27.6)' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
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
# Prove address-and-port-dependent mapping with two distinct destination
# ports on the same peer IP and the same local UDP source. The synthetic
# alternate rule is explicit; an unmatched MASQUERADE result is not evidence.
send_udp "$node_a" 10.27.1.2 42027 "100.64.27.6:$b_predict" A-MAIN-MAP
send_udp "$node_a" 10.27.1.2 42027 "100.64.27.6:$((b_predict + 1))" A-ALT-MAP
send_udp "$node_b" 10.27.2.2 43027 "100.64.27.2:$a_predict" B-MAIN-MAP
send_udp "$node_b" 10.27.2.2 43027 "100.64.27.2:$((a_predict + 1))" B-ALT-MAP
ip netns exec "$nat_a" conntrack -L -p udp > "$evidence/map-a-same-ip.txt" 2>&1
ip netns exec "$nat_b" conntrack -L -p udp > "$evidence/map-b-same-ip.txt" 2>&1
grep -F "src=10.27.1.2 dst=100.64.27.6 sport=42027 dport=$b_predict" "$evidence/map-a-same-ip.txt" | grep -F "dport=$a_peer" >/dev/null
grep -F "src=10.27.1.2 dst=100.64.27.6 sport=42027 dport=$((b_predict + 1))" "$evidence/map-a-same-ip.txt" | grep -F "dport=$a_alternate" >/dev/null
grep -F "src=10.27.2.2 dst=100.64.27.2 sport=43027 dport=$a_predict" "$evidence/map-b-same-ip.txt" | grep -F "dport=$b_peer" >/dev/null
grep -F "src=10.27.2.2 dst=100.64.27.2 sport=43027 dport=$((a_predict + 1))" "$evidence/map-b-same-ip.txt" | grep -F "dport=$b_alternate" >/dev/null
if [[ $a_step != 0 ]]; then [[ $a_peer != "$a_alternate" ]]; fi
if [[ $b_step != 0 ]]; then [[ $b_peer != "$b_alternate" ]]; fi
reset_classifier
send_udp "$node_b" 10.27.2.2 43027 "100.64.27.2:$a_predict" B-OPEN-A
sleep 0.1
ip netns exec "$nat_b" iptables -Z FORWARD
send_udp "$nat_a" 100.64.27.2 "$((a_peer + 1))" "100.64.27.6:$b_peer" B-WRONG-PORT
sleep 0.1
ip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/filter-b-wrong.txt"
assert_dropped "$evidence/filter-b-wrong.txt"
ip netns exec "$nat_b" iptables -Z FORWARD
send_udp "$wan" 100.64.27.1 "$a_peer" "100.64.27.6:$b_peer" B-WRONG-IP
sleep 0.1
ip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/filter-b-wrong-ip.txt"
assert_dropped "$evidence/filter-b-wrong-ip.txt"
send_udp "$nat_a" 100.64.27.2 "$a_peer" "100.64.27.6:$b_peer" B-RIGHT-PORT
sleep 0.1
ip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/filter-b-right.txt"
assert_forwarded "$evidence/filter-b-right.txt"
reset_classifier
send_udp "$node_a" 10.27.1.2 42027 "100.64.27.6:$b_predict" A-OPEN-B
sleep 0.1
ip netns exec "$nat_a" iptables -Z FORWARD
send_udp "$nat_b" 100.64.27.6 "$((b_peer + 1))" "100.64.27.2:$a_peer" A-WRONG-PORT
sleep 0.1
ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/filter-a-wrong.txt"
assert_dropped "$evidence/filter-a-wrong.txt"
ip netns exec "$nat_a" iptables -Z FORWARD
send_udp "$wan" 100.64.27.1 "$b_peer" "100.64.27.2:$a_peer" A-WRONG-IP
sleep 0.1
ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/filter-a-wrong-ip.txt"
assert_dropped "$evidence/filter-a-wrong-ip.txt"
send_udp "$nat_b" 100.64.27.6 "$b_peer" "100.64.27.2:$a_peer" A-RIGHT-PORT
sleep 0.1
ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/filter-a-right.txt"
assert_forwarded "$evidence/filter-a-right.txt"
reset_classifier
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
  inject_mapping_change "$nat_a" 10.27.1.2 42027 100.64.27.2 100.64.27.6 "$b_predict" "$((a_base + 5 * a_step))"
elif [[ $case_name == jump_b ]]; then
  inject_mapping_change "$nat_b" 10.27.2.2 43027 100.64.27.6 100.64.27.2 "$a_predict" "$((b_base + 7 * b_step))"
fi
if [[ $case_name == blocked ]]; then
  # Fault injection begins only after the NAT classifier, so the classifier
  # still proves APDF while the application sees a genuinely blocked WAN.
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.2 -d 100.64.27.6 -j DROP
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.6 -d 100.64.27.2 -j DROP
fi

key_dir=${MIOPUNCH_NAT_KEY_DIR:-$runtime}
mkdir -p "$key_dir"
client_key="$key_dir/client.key"
server_key="$key_dir/server.key"
client_id=$($probe identity "$client_key")
server_id=$($probe identity "$server_key")
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
if [[ $strategy == legacy ]]; then
  observers='qad.miopunch.test:46761,qad2.miopunch.test:46762,qad3.miopunch.test:46763,qad4.miopunch.test:46764'
fi
strategy_env=(MIOPUNCH_NAT_STRATEGY=legacy MIOPUNCH_LEGACY_PREVIOUS_FAILURES=18)
ip netns exec "$server_node" env MIOPUNCH_NAT_BIND_PORT="$server_port" \
  MIOPUNCH_NAT_SECRET_FILE="$server_secret" MIOPUNCH_NAT_AUTH_FAULT="$server_auth" \
  "${strategy_env[@]}" MIOPUNCH_NAT_OBSERVERS="$observers" timeout 40 "$probe" server \
  "$server_key" "$runtime/cert.der" 100.64.27.1:1883 qad.miopunch.test 46761 \
  "$client_id" > "$evidence/server.log" 2>&1 &
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
if [[ $case_name == stale || $case_name == expire_mapping || $case_name == late_control || $case_name == cancel || $case_name == old_round || $case_name == wide_claim ]]; then fault=$case_name; fi
set +e
ip netns exec "$client_node" env MIOPUNCH_NAT_BIND_PORT="$client_port" \
  MIOPUNCH_NAT_SECRET_FILE="$client_secret" MIOPUNCH_NAT_AUTH_FAULT="$client_auth" \
  "${strategy_env[@]}" MIOPUNCH_NAT_FAULT="$fault" \
  MIOPUNCH_NAT_OBSERVERS="$observers" \
  MIOPUNCH_NAT_REPLAY_ROUND="${MIOPUNCH_NAT_REPLAY_ROUND:-00000000000000000000000000000000}" timeout 40 \
  "$probe" "${client_args[@]}" > "$evidence/client.log" 2>&1
client_rc=$?
wait "${jobs[-1]}"
server_rc=$?
set -e
printf 'case=%s strategy=%s topology=%s dialer=%s client_rc=%s server_rc=%s\n' \
  "$case_name" "$strategy" "$topology" "$dialer" "$client_rc" "$server_rc" > "$evidence/run.txt"
[[ $client_rc != 124 && $server_rc != 124 ]]
sleep 0.3


if [[ $case_name == positive ]]; then
  [[ $client_rc == 0 && $server_rc == 0 ]]
  for side in client server; do
    grep -q '^LEGACY_SELECTED ' "$evidence/$side.log"
    if grep -q DATA_OK "$evidence/$side.log"; then
      grep -q CompleteExecution "$evidence/$side.log"
    else
      grep -q '^PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed' "$evidence/$side.log"
      grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/$side.log"
    fi
  done
else
  if [[ $case_name == nonmember ]]; then
    [[ $client_rc == 0 && $server_rc == 0 ]]
    grep -Eq '^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=[0-9]+ probes=0$' "$evidence/client.log"
    grep -Eq '^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=[0-9]+ probes=0$' "$evidence/server.log"
  elif [[ $case_name == revoked || $case_name == no_secret || $case_name == wrong_secret ]]; then
    [[ $client_rc == 1 && $server_rc == 1 ]]
    grep -Fxq 'Error: Connection was rejected locally' "$evidence/client.log"
    grep -Fxq 'Error: Custom { kind: Other, error: Connection was rejected locally }' "$evidence/server.log"
  else
    [[ $client_rc == 0 && $server_rc == 0 ]]
    grep -q PATH_UNAVAILABLE "$evidence/client.log"
    grep -q PATH_UNAVAILABLE "$evidence/server.log"
    grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/client.log"
    grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/server.log"
  fi
  ! grep -q DATA_OK "$evidence/client.log"
  ! grep -q DATA_OK "$evidence/server.log"
fi
echo "PASS scheduler case=$case_name topology=$topology dialer=$dialer history=18"
