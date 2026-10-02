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
case "$case_name" in positive|duplicate|data_cut|no_prime|path_id_retry|blocked|wrong|stale|control_cut) ;; *) exit 2 ;; esac
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

env MIO43_ROLE=prepare MIO43_DIRECTORY="$runtime/nodes" "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/prepare.log" 2>&1
printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
: > "$evidence/broker.log"
: > "$evidence/qad.log"
: > "$evidence/tcpdump.log"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" "$qad" serve "$runtime/cert.der" 100.64.27.1:7842 > "$evidence/qad.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump -U -n -i any -w "$evidence/wan.pcap" 'tcp and (host 100.64.27.2 or host 100.64.27.6)' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do
  if grep -q QAD_READY "$evidence/qad.log" && grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"; then break; fi
  sleep 0.1
done
grep -q QAD_READY "$evidence/qad.log"
grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"
sleep 0.5


cp "$runtime/cert.der" "$runtime/nodes/cert.der"
if [[ $case_name == duplicate || $case_name == data_cut ]]; then
  ip netns exec "$wan" env MIO43_CASE="$case_name" MIO43_ROLE=observe MIO43_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/observer.log" 2>&1 &
  jobs+=("$!")
  for _ in $(seq 1 100); do
    [[ -f "$runtime/nodes/observer-ready" ]] && break
    sleep 0.01
  done
  [[ -f "$runtime/nodes/observer-ready" ]]
fi
ip netns exec "$node_b" env MIO43_CASE="$case_name" MIO43_ROLE=serve MIO43_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/server.log" 2>&1 &
server_pid=$!
jobs+=("$server_pid")
for _ in $(seq 1 100); do
  [[ -f "$runtime/nodes/server-route" ]] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -f "$runtime/nodes/server-route" ]]
if [[ $case_name == control_cut || $case_name == data_cut ]]; then
  ip netns exec "$node_a" env MIO43_CASE="$case_name" MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/client.log" 2>&1 &
  client_pid=$!
  jobs+=("$client_pid")
  if [[ $case_name == control_cut ]]; then
    for _ in $(seq 1 100); do
      grep -q MQTT_BOOTSTRAP_START "$evidence/client.log" && break
      kill -0 "$client_pid" 2>/dev/null || break
      sleep 0.01
    done
    grep -q MQTT_BOOTSTRAP_START "$evidence/client.log"
    kill "${jobs[0]}"
  else
    for _ in $(seq 1 500); do
      [[ -f "$runtime/nodes/client-cut-ready" && -f "$runtime/nodes/server-cut-ready" ]] && break
      kill -0 "$client_pid" 2>/dev/null || break
      sleep 0.01
    done
    [[ -f "$runtime/nodes/client-cut-ready" && -f "$runtime/nodes/server-cut-ready" ]]
    ip netns exec "$wan" iptables -I FORWARD 1 -p udp -s 100.64.27.2 -d 100.64.27.6 -j DROP
    ip netns exec "$wan" iptables -I FORWARD 1 -p udp -s 100.64.27.6 -d 100.64.27.2 -j DROP
    date -u +'%s.%N' > "$evidence/data-cut-clock.txt"
    touch "$runtime/nodes/data-cut"
  fi
  wait "$client_pid"
else
  ip netns exec "$node_a" env MIO43_CASE="$case_name" MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/client.log" 2>&1
fi
wait "$server_pid"
if [[ $case_name == control_cut || $case_name == blocked ]]; then
  grep -q CONTROL_UNAVAILABLE "$evidence/client.log"
  grep -q CONTROL_UNAVAILABLE "$evidence/server.log"
  ! grep -q DATA_OK "$evidence/client.log"
  ! grep -q DATA_OK "$evidence/server.log"
else
  grep -q DATA_OK "$evidence/client.log"
  grep -q DATA_OK "$evidence/server.log"
  if [[ $case_name == duplicate ]]; then
    grep -q MQTT_DUPLICATE "$evidence/observer.log"
    grep -q DUPLICATE_BOUNDED "$evidence/client.log"
    grep -q DUPLICATE_BOUNDED "$evidence/server.log"
  elif [[ $case_name == data_cut ]]; then
    grep -q DATA_FAILED "$evidence/client.log"
    grep -q DATA_FAILED "$evidence/server.log"
    ip netns exec "$wan" iptables-save -c > "$evidence/data-cut-rules.txt"
  fi
fi
echo "PASS formal MQTT dual NAT case=$case_name"
