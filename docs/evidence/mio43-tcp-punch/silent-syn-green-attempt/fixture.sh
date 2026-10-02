#!/usr/bin/env bash
# MIOPUNCH-27: run the Rust path probe through two real Linux kernel NATs.
set -euo pipefail

if [[ $# -ne 4 || ${EUID} -ne 0 ]]; then
  echo "usage (as root): udp_nat_lab.sh positive|blocked PROBE_BIN QAD_BIN EVIDENCE_DIR" >&2
  exit 2
fi
case_name=$1
probe=$2
qad=$3
evidence=$4
case "$case_name" in positive|blocked) ;; *) exit 2 ;; esac
mkdir -p "$evidence"
evidence=$(realpath "$evidence")
runtime=$(mktemp -d /tmp/miopunch43.XXXXXX)
prefix="m43$$"
wan="$prefix-wan"
nat_a="$prefix-na"
nat_b="$prefix-nb"
node_a="$prefix-a"
node_b="$prefix-b"
names=("$wan" "$nat_a" "$nat_b" "$node_a" "$node_b")
jobs=()

cleanup() {
  for job in "${jobs[@]}"; do kill "$job" 2>/dev/null || true; done
  for nat in "$nat_a" "$nat_b"; do
    ip netns exec "$nat" iptables-save -c > "$evidence/$nat-rules.txt" 2>/dev/null || true
    ip netns exec "$nat" conntrack -L > "$evidence/$nat-conntrack.txt" 2>&1 || true
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
  ip netns exec "$nat" iptables -A FORWARD -i lan0 -o wan0 -j ACCEPT
  ip netns exec "$nat" iptables -A FORWARD -i wan0 -o lan0 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ip netns exec "$nat" iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
done


# A supported NAT silently drops unsolicited WAN SYNs instead of letting its
# own TCP stack actively reset the private host's simultaneous-open attempt.
for nat in "$nat_a" "$nat_b"; do
  ip netns exec "$nat" iptables -A INPUT -i wan0 -p tcp --syn -j DROP
done
ip netns exec "$wan" sysctl -qw net.ipv4.ip_forward=1
if [[ $case_name == blocked ]]; then
  ip netns exec "$wan" iptables -A FORWARD -p tcp -s 100.64.27.2 -d 100.64.27.6 -j DROP
  ip netns exec "$wan" iptables -A FORWARD -p tcp -s 100.64.27.6 -d 100.64.27.2 -j DROP
fi
env MIO43_ROLE=prepare MIO43_DIRECTORY="$runtime/nodes" "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/prepare.log" 2>&1
printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" python3 /repo/scripts/tcp_stun_observer.py > "$evidence/observer.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump -U -n -i any -w "$evidence/wan.pcap" tcp > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do
  if grep -q STUN_TCP_READY "$evidence/observer.log" && grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"; then break; fi
  sleep 0.1
done
grep -q STUN_TCP_READY "$evidence/observer.log"
grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"
ip netns exec "$node_b" env MIO43_CASE="$case_name" MIO43_ROLE=serve MIO43_DIRECTORY="$runtime/nodes" timeout 30 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/server.log" 2>&1 &
server_pid=$!
jobs+=("$server_pid")
for _ in $(seq 1 100); do
  [[ -f "$runtime/nodes/server-route" ]] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -f "$runtime/nodes/server-route" ]]
ip netns exec "$node_a" env MIO43_CASE="$case_name" MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" timeout 30 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/client.log" 2>&1
wait "$server_pid"
grep -q DATA_OK "$evidence/client.log"
grep -q DATA_OK "$evidence/server.log"
echo "PASS formal TCP dual NAT case=$case_name"
