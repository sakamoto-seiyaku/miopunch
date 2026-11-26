#!/bin/bash
# Run via unshare -Urnm, using a pinned lib test executable built with nat-lab-test-hooks.
set -euo pipefail
binary=$1
out=$2
mkdir -p "$out"
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mkdir -p /run/netns
ip link set lo up
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for name in m197-a m197-b; do ip netns del "$name" 2>/dev/null || true; done
}
trap cleanup EXIT
test_name=native_client::dual_stack_tests::ipv6_qnt_crosses_both_unsolicited_inbound_filters_on_the_original_member
ip link add bridge197 type bridge
ip addr add 10.197.0.254/24 dev bridge197
ip link set bridge197 up
for side in a b; do
  ip netns add m197-$side
  ip link add m197-${side}0 type veth peer name m197-${side}1
  ip link set m197-${side}0 master bridge197
  ip link set m197-${side}0 up
  ip link set m197-${side}1 netns m197-$side
  ip -n m197-$side link set m197-${side}1 name lan0
  ip -n m197-$side link set lo up
  ip -n m197-$side link set lan0 up
  if [[ "$side" == a ]]; then number=1; else number=2; fi
  ip -n m197-$side addr add 10.197.0.$number/24 dev lan0
  ip -n m197-$side -6 addr add fd00:197::$number/64 dev lan0 nodad
  ip netns exec m197-$side iptables -A OUTPUT -d 10.197.0.0/24 -p udp -j DROP
  ip netns exec m197-$side ip6tables -A INPUT -p udp -d fd00:197::/64 -j DROP
  ip netns exec m197-$side ip6tables -A INPUT -p udp -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ip netns exec m197-$side ip6tables -A INPUT -p udp -j DROP
  ip netns exec m197-$side ip6tables -I OUTPUT -p udp -j DROP
 done
env MIO197_ROOT="$out" MIO197_ROLE=relay "$binary" --exact "$test_name" --ignored --nocapture >"$out/relay.log" 2>&1 &
pids+=("$!")
for attempt in {1..100}; do [[ -f "$out/relay.port" ]] && break; sleep 0.05; done
env MIO197_ROOT="$out" MIO197_ROLE=prepare "$binary" --exact "$test_name" --ignored --nocapture >"$out/prepare.log" 2>&1
for pair in a:provider b:consumer; do
  side=${pair%:*}
  role=${pair#*:}
  ip netns exec m197-$side env MIO197_ROOT="$out" MIO197_ROLE="$role" "$binary" --exact "$test_name" --ignored --nocapture >"$out/$role.log" 2>&1 &
  pids+=("$!")
done
for attempt in {1..400}; do
  if [[ -f "$out/provider.relay" && -f "$out/consumer.relay" ]]; then break; fi
  sleep 0.05
done
[[ -f "$out/provider.relay" && -f "$out/consumer.relay" ]]
for side in a b; do
  if [[ "$side" == a ]]; then number=1; else number=2; fi
  ip -n m197-$side -6 addr add fd00:197:1::$number/64 dev lan0 nodad
  ip netns exec m197-$side ip6tables -D OUTPUT -p udp -j DROP
done
touch "$out/release"
status=0
wait "${pids[1]}" || status=$?
wait "${pids[2]}" || status=$?
for side in a b; do ip netns exec m197-$side ip6tables-save -c >"$out/$side-filter.txt"; done
rg DUAL_STACK_FILTERED_DATA "$out/provider.log" "$out/consumer.log"
exit "$status"
