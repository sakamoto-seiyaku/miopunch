#!/usr/bin/env bash
# MIOPUNCH-27: run the Rust path probe through two real Linux kernel NATs.
set -euo pipefail

if [[ $# -ne 4 || ${EUID} -ne 0 ]]; then
  echo "usage (as root): udp_nat_lab.sh positive|no_prime|path_id_retry|blocked|wrong|stale|control_cut PROBE_BIN QAD_BIN EVIDENCE_DIR" >&2
  exit 2
fi
case_name=$1
probe=$2
qad=$3
evidence=$4
case "$case_name" in positive|no_prime|path_id_retry|blocked|wrong|stale|control_cut) ;; *) exit 2 ;; esac
mkdir -p "$evidence"
evidence=$(realpath "$evidence")
runtime=$(mktemp -d /tmp/miopunch27.XXXXXX)
prefix="m27$$"
wan="$prefix-wan"
nat_a="$prefix-na"
nat_b="$prefix-nb"
node_a="$prefix-a"
node_b="$prefix-b"
names=("$wan" "$nat_a" "$nat_b" "$node_a" "$node_b")
jobs=()

cleanup() {
  for job in "${jobs[@]}"; do kill "$job" 2>/dev/null || true; done
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
  ip netns exec "$nat" iptables -A FORWARD -i lan0 -o wan0 -j ACCEPT
  ip netns exec "$nat" iptables -A FORWARD -i wan0 -o lan0 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ip netns exec "$nat" iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
done

# Endpoint-independent mapping, address-dependent filtering: the source port
# is preserved across destinations, but each remote IP must be contacted before
# its inbound packets pass. A QAD probe therefore cannot open the peer filter.
cone_mapping() {
  local nat=$1 private=$2 public=$3 port=$4
  ip netns exec "$nat" iptables -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "$port" \
    -m recent --name mio27allow --set --rdest
  ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$port" \
    -j SNAT --to-source "$public:$port"
  ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp --dport "$port" \
    -j DNAT --to-destination "$private:$port"
  ip netns exec "$nat" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp --dport "$port" \
    -m recent --name mio27allow --rcheck --seconds 60 --rsource -j ACCEPT
}
cone_mapping "$nat_a" 10.27.1.2 100.64.27.2 42027
cone_mapping "$nat_b" 10.27.2.2 100.64.27.6 43027
ip netns exec "$wan" sysctl -qw net.ipv4.ip_forward=1
for node in "$node_a" "$node_b"; do
  mkdir -p "/etc/netns/$node"
  printf '127.0.0.1 localhost\n100.64.27.1 qad.miopunch.test\n' > "/etc/netns/$node/hosts"
done

if [[ $case_name == blocked ]]; then
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.2 -d 100.64.27.6 -j DROP
  ip netns exec "$wan" iptables -A FORWARD -p udp -s 100.64.27.6 -d 100.64.27.2 -j DROP
fi

printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
: > "$evidence/broker.log"
: > "$evidence/qad.log"
: > "$evidence/tcpdump.log"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" "$qad" serve "$runtime/cert.der" 100.64.27.1:46761 > "$evidence/qad.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump -U -n -i any -w "$evidence/wan.pcap" 'udp and (host 100.64.27.2 or host 100.64.27.6)' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do
  if grep -q QAD_READY "$evidence/qad.log" && grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"; then break; fi
  sleep 0.1
done
grep -q QAD_READY "$evidence/qad.log"
grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"
sleep 0.5

client_id=$($probe identity "$runtime/client.key")
server_id=$($probe identity "$runtime/server.key")
skip_prime=''
if [[ $case_name == no_prime ]]; then skip_prime=1; fi
ip netns exec "$node_b" env MIOPUNCH_NAT_BIND_PORT=43027 MIOPUNCH_NAT_SKIP_PRIME="$skip_prime" timeout 35 "$probe" server "$runtime/server.key" "$runtime/cert.der" 100.64.27.1:1883 qad.miopunch.test 46761 "$client_id" > "$evidence/server.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 150); do
  route=$(sed -n 's/^READY id=.* route=//p' "$evidence/server.log" | head -1)
  if [[ -n $route ]]; then break; fi
  sleep 0.1
done
test -n "${route:-}"

client_args=(client "$runtime/client.key" "$runtime/cert.der" 100.64.27.1:1883 qad.miopunch.test 46761 "$server_id" "$route")
if [[ $case_name == wrong ]]; then client_args+=(100.64.27.6:9); fi
fault=''
if [[ $case_name == stale || $case_name == control_cut || $case_name == path_id_retry ]]; then fault=$case_name; fi
if [[ $case_name == control_cut ]]; then
  ip netns exec "$node_a" env MIOPUNCH_NAT_BIND_PORT=42027 MIOPUNCH_NAT_FAULT="$fault" timeout 35 "$probe" "${client_args[@]}" > "$evidence/client.log" 2>&1 &
  client_pid=$!
  for _ in $(seq 1 100); do
    if grep -q CONTROL_READY "$evidence/client.log"; then break; fi
    sleep 0.1
  done
  grep -q CONTROL_READY "$evidence/client.log"
  kill "${jobs[0]}"
  wait "$client_pid"
else
  ip netns exec "$node_a" env MIOPUNCH_NAT_BIND_PORT=42027 MIOPUNCH_NAT_FAULT="$fault" timeout 35 "$probe" "${client_args[@]}" > "$evidence/client.log" 2>&1
fi
wait "${jobs[-1]}"
sleep 0.3

case "$case_name" in
  positive|path_id_retry)
    grep -q DATA_OK "$evidence/client.log"
    grep -q DATA_OK "$evidence/server.log" ;;
  no_prime|blocked|wrong)
    grep -q PATH_UNAVAILABLE "$evidence/client.log"
    grep -q PATH_UNAVAILABLE "$evidence/server.log" ;;
  stale)
    grep -q STALE_CANDIDATE "$evidence/client.log"
    grep -q CONTROL_UNAVAILABLE "$evidence/server.log" ;;
  control_cut)
    grep -q CONTROL_UNAVAILABLE "$evidence/client.log"
    grep -q CONTROL_UNAVAILABLE "$evidence/server.log" ;;
esac
! grep -q DATA_OK "$evidence/client.log" || [[ $case_name == positive || $case_name == path_id_retry ]]
! grep -q DATA_OK "$evidence/server.log" || [[ $case_name == positive || $case_name == path_id_retry ]]
if [[ $case_name == path_id_retry ]]; then
  grep -Eq 'PATH_OPEN path_id_retries=[1-9]' "$evidence/client.log"
fi
if [[ $case_name == no_prime ]]; then
  grep -q PRIME_SKIPPED "$evidence/server.log"
fi

ip netns exec "$nat_a" iptables -t nat -nvL POSTROUTING > "$evidence/nat-a.txt"
ip netns exec "$nat_b" iptables -t nat -nvL POSTROUTING > "$evidence/nat-b.txt"
ip netns exec "$nat_a" iptables -nvL FORWARD > "$evidence/nat-a-forward.txt"
ip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/nat-b-forward.txt"
ip netns exec "$wan" iptables -nvL FORWARD > "$evidence/wan-forward.txt"
if [[ $case_name == no_prime ]]; then
  grep -Eq 'policy DROP [1-9]' "$evidence/nat-b-forward.txt"
fi
echo "PASS $case_name evidence=$evidence"
