#!/usr/bin/env bash
# MIOPUNCH-27: run the Rust path probe through two real Linux kernel NATs.
set -euo pipefail

if [[ $# -ne 4 || ${EUID} -ne 0 ]]; then
  echo "usage (as root): udp_nat_lab.sh positive|blocked|simultaneous_cancel|endpoint_dependent|udp_fallback|simultaneous|mixed_tcp|wrong_target|private_bind|cancel_attempt|observer_failure|candidate_denied PROBE_BIN QAD_BIN EVIDENCE_DIR" >&2
  exit 2
fi
case_name=$1
probe=$2
qad=$3
evidence=$4
case "$case_name" in positive|blocked|simultaneous_cancel|endpoint_dependent|udp_fallback|simultaneous|mixed_tcp|wrong_target|private_bind|cancel_attempt|observer_failure|candidate_denied) ;; *) exit 2 ;; esac
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
if [[ $case_name == blocked || $case_name == simultaneous_cancel || $case_name == udp_fallback || $case_name == cancel_attempt ]]; then
  ip netns exec "$wan" iptables -A FORWARD -p tcp -s 100.64.27.2 -d 100.64.27.6 -j DROP
  ip netns exec "$wan" iptables -A FORWARD -p tcp -s 100.64.27.6 -d 100.64.27.2 -j DROP
fi
if [[ $case_name == observer_failure ]]; then
  ip netns exec "$nat_a" iptables -I FORWARD 1 -s 10.27.1.2 -d 100.64.27.1 -p tcp --dport 3478 -j DROP
fi
if [[ $case_name == endpoint_dependent ]]; then
  ip netns exec "$nat_a" iptables -t nat -I POSTROUTING 1 -o wan0 -s 10.27.1.2 -d 100.64.27.6 -p tcp --sport 42027 -j SNAT --to-source 100.64.27.2:42127
  ip netns exec "$nat_b" iptables -t nat -I POSTROUTING 1 -o wan0 -s 10.27.2.2 -d 100.64.27.2 -p tcp --sport 43027 -j SNAT --to-source 100.64.27.6:43127
fi
env MIO43_CASE="$case_name" MIO43_ROLE=prepare MIO43_DIRECTORY="$runtime/nodes" "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/prepare.log" 2>&1
printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" python3 /repo/scripts/tcp_stun_observer.py > "$evidence/observer.log" 2>&1 &
jobs+=("$!")
ip netns exec "$wan" tcpdump -U -n -i any -w "$evidence/wan.pcap" 'tcp or udp' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do
  if grep -q STUN_TCP_READY "$evidence/observer.log" && grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"; then break; fi
  sleep 0.1
done
grep -q STUN_TCP_READY "$evidence/observer.log"
grep -q 'Opening ipv4 listen socket' "$evidence/broker.log"
if [[ $case_name == udp_fallback || $case_name == mixed_tcp || $case_name == observer_failure || $case_name == candidate_denied ]]; then
  ip netns exec "$wan" "$qad" serve "$runtime/cert.der" 100.64.27.1:7842 > "$evidence/qad.log" 2>&1 &
  jobs+=("$!")
  for _ in $(seq 1 100); do [[ -f "$runtime/cert.der" ]] && break; sleep 0.01; done
  cp "$runtime/cert.der" "$runtime/nodes/cert.der"
  for node in "$node_a" "$node_b"; do
    mkdir -p "/etc/netns/$node"
    printf '127.0.0.1 localhost\n100.64.27.1 qad.miopunch.test\n' > "/etc/netns/$node/hosts"
  done
  for side in a b; do
    if [[ $side == a ]]; then nat="$nat_a"; private=10.27.1.2; public=100.64.27.2; port=42027;
    else nat="$nat_b"; private=10.27.2.2; public=100.64.27.6; port=43027; fi
    ip netns exec "$nat" iptables -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "$port" -m recent --name mio43allow --set --rdest
    ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$port" -j SNAT --to-source "$public:$port"
    ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp --dport "$port" -j DNAT --to-destination "$private:$port"
    ip netns exec "$nat" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp --dport "$port" -m recent --name mio43allow --rcheck --seconds 60 --rsource -j ACCEPT
  done
fi
ip netns exec "$node_b" env MIO43_CASE="$case_name" MIO43_ROLE=serve MIO43_DIRECTORY="$runtime/nodes" timeout 30 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/server.log" 2>&1 &
server_pid=$!
jobs+=("$server_pid")
for _ in $(seq 1 100); do
  [[ -f "$runtime/nodes/server-route" ]] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -f "$runtime/nodes/server-route" ]]
if [[ $case_name == simultaneous_cancel || $case_name == cancel_attempt ]]; then
  ip netns exec "$node_a" env MIO43_CASE="$case_name" MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" timeout 30 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/client.log" 2>&1 &
  client_pid=$!
  jobs+=("$client_pid")
  for _ in $(seq 1 400); do
    if grep -q TCP_PUNCH_START "$evidence/client.log" && grep -q TCP_PUNCH_START "$evidence/server.log"; then break; fi
    sleep 0.01
  done
  grep -q TCP_PUNCH_START "$evidence/client.log"
  grep -q TCP_PUNCH_START "$evidence/server.log"
  touch "$runtime/nodes/cancel"
  if [[ $case_name == cancel_attempt ]]; then
    for _ in $(seq 1 400); do [[ -f "$runtime/nodes/cancel-done" ]] && break; sleep 0.01; done
    [[ -f "$runtime/nodes/cancel-done" ]]
    ip netns exec "$wan" iptables -D FORWARD -p tcp -s 100.64.27.2 -d 100.64.27.6 -j DROP
    ip netns exec "$wan" iptables -D FORWARD -p tcp -s 100.64.27.6 -d 100.64.27.2 -j DROP
    # The prior SYN conntrack can reject a new ISN even though the SDK socket
    # is closed. Model expiry of only these owned peer flows; do not touch the
    # broker/observer flows or pretend immediate gateway tuple reuse works.
    ip netns exec "$nat_a" conntrack -D -p tcp --orig-src 10.27.1.2 --orig-dst 100.64.27.6 --sport 42027 --dport 43027 > "$evidence/peer-flow-expiry-a.txt" 2>&1 || true
    ip netns exec "$nat_b" conntrack -D -p tcp --orig-src 10.27.2.2 --orig-dst 100.64.27.2 --sport 43027 --dport 42027 > "$evidence/peer-flow-expiry-b.txt" 2>&1 || true
    ip netns exec "$node_a" ss -tan > "$evidence/client-before-retry-sockets.txt"
    ip netns exec "$node_b" ss -tan > "$evidence/server-before-retry-sockets.txt"
    touch "$runtime/nodes/retry"
  fi
  wait "$client_pid"
else
ip netns exec "$node_a" env MIO43_CASE="$case_name" MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" timeout 30 "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/client.log" 2>&1
fi
wait "$server_pid"
if [[ $case_name == simultaneous_cancel ]]; then
  grep -q CANCEL_OK "$evidence/client.log"
  grep -q CANCEL_OK "$evidence/server.log"
  exit 0
fi
if [[ $case_name == blocked || $case_name == endpoint_dependent ]]; then
  grep -q UNSUPPORTED_BOUNDED "$evidence/client.log"
  grep -q UNSUPPORTED_BOUNDED "$evidence/server.log"
  exit 0
fi
if [[ $case_name == candidate_denied ]]; then
  grep -q TCP_OBSERVED "$evidence/client.log"
  grep -q TCP_OBSERVED "$evidence/server.log"
  ! grep -q TCP_PUNCH_START "$evidence/client.log"
  ! grep -q TCP_PUNCH_START "$evidence/server.log"
fi
if [[ $case_name == observer_failure ]]; then
  grep -q TCP_OBSERVATION_FAILED "$evidence/client.log"
  grep -q TCP_OBSERVED "$evidence/server.log"
  ! grep -q TCP_PUNCH_START "$evidence/client.log"
  ! grep -q TCP_PUNCH_START "$evidence/server.log"
fi
grep -q DATA_OK "$evidence/client.log"
grep -q DATA_OK "$evidence/server.log"
echo "PASS formal TCP dual NAT case=$case_name"
