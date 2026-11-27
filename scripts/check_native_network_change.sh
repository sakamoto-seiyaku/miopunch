#!/bin/bash
# Run through unshare -Urnm with a pinned nat-lab-test-hooks lib executable.
set -euo pipefail
binary=$1
out=$2
moving=${3:-both}
transport=${4:-udp}
protocol=udp
if [[ $transport == tcp ]]; then protocol=tcp; fi
mkdir -p "$out"
if [[ -n ${MIOPUNCH_TRACE_PEER_IPV6:-} ]]; then
  exec 2>"$out/driver.log"
  PS4='+${EPOCHREALTIME} '
  set -x
fi
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mkdir -p /run/netns
ip link set lo up
sysctl -q -w net.ipv4.ip_forward=1 net.ipv6.conf.all.forwarding=1
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for side in a b; do ip netns del m205-$side 2>/dev/null || true; done
}
trap cleanup EXIT
wait_file() {
  for attempt in {1..700}; do
    [[ -f "$1" ]] && return
    for pid in "${pids[@]}"; do
      if ! kill -0 "$pid" 2>/dev/null; then
        echo "worker exited before signal $1" >&2
        return 1
      fi
    done
    sleep 0.05
  done
  echo "missing signal $1" >&2
  return 1
}
verify_routes() {
  local phase=$1 side number peer prefix fourth
  for side in a b; do
    if [[ $side == a ]]; then number=1; else number=2; fi
    peer=$((3-number))
    prefix=$phase
    if [[ $moving == provider && $side == a ]]; then prefix=0; fi
    fourth=0
    if [[ $phase == 2 && $prefix == 2 ]]; then fourth=1; fi
    ip netns exec m205-$side ip -6 route get 2001:db8:205:$prefix::$peer >"$out/$side-v6-route-$phase.txt"
    ip -6 route show table all >"$out/root-v6-routes-$phase.txt"
    ip -6 neigh show >"$out/root-v6-neighbors-$phase.txt"
    ip -n m205-$side -6 route show table all >"$out/$side-v6-routes-$phase.txt"
    ip -n m205-$side -6 neigh show >"$out/$side-v6-neighbors-$phase.txt"
    ip netns exec m205-$side ping -6 -c 1 -W 2 2001:db8:205:$prefix::$peer >"$out/$side-v6-ping-$phase.txt" || true
    ip -6 neigh show >"$out/root-v6-neighbors-after-$phase.txt"
    bridge fdb show br bridge205 >"$out/bridge-fdb-$phase.txt"
    ip netns exec m205-$side ip route get 10.205.$fourth.$peer >"$out/$side-v4-route-$phase.txt"
    ip netns exec m205-$side ping -c 1 -W 2 10.205.$fourth.$peer >"$out/$side-v4-ping-$phase.txt" || true
  done
}
ip link add bridge205 address 02:00:00:00:02:05 type bridge
ip addr add 10.205.0.254/24 dev bridge205
ip addr add 10.205.1.254/24 dev bridge205
for prefix in 0 1 2; do
  if [[ $prefix == 0 ]]; then address=2001:db8:205::fe; else address=2001:db8:205:$prefix::fe; fi
  ip -6 addr add "$address/64" dev bridge205 nodad
done
ip link set bridge205 up
for side in a b; do
  if [[ $side == a ]]; then number=1; else number=2; fi
  ip netns add m205-$side
  ip link add m205-${side}0 type veth peer name m205-${side}1
  ip link set m205-${side}0 master bridge205
  ip link set m205-${side}0 up
  ip link set m205-${side}1 netns m205-$side
  ip -n m205-$side link set m205-${side}1 name lan0
  ip -n m205-$side link set lo up
  ip -n m205-$side link set lan0 up
  ip -n m205-$side addr add 10.205.0.$number/24 dev lan0
  ip -n m205-$side -6 addr add 2001:db8:205::$number/64 dev lan0 nodad
  ip -n m205-$side route add default via 10.205.0.254
  ip -n m205-$side -6 route add default via 2001:db8:205::fe
  peer=$((3-number))
  ip netns exec m205-$side ip6tables -A INPUT -s 2001:db8:205::$peer -p "$protocol" -j DROP
done
for side in a b; do
  if [[ $moving == provider && $side == b ]]; then continue; fi
  if [[ $side == a ]]; then number=1; else number=2; fi
  ip link add m205-${side}2 type veth peer name m205-${side}3
  ip link set m205-${side}2 master bridge205
  ip link set m205-${side}2 up
  ip link set m205-${side}3 netns m205-$side
  ip -n m205-$side link set m205-${side}3 name lan1
  ip -n m205-$side addr add 10.205.1.$number/24 dev lan1
  ip -n m205-$side -6 addr add 2001:db8:205:2::$number/64 dev lan1 nodad
done
test_name=native_client::dual_stack_tests::actual_interface_changes_preserve_valid_work_and_the_original_forwarding_stream
env MIO205_ROOT="$out" MIO205_TRANSPORT="$transport" MIO205_ROLE=relay "$binary" --exact "$test_name" --ignored --nocapture >"$out/relay.log" 2>&1 &
pids+=("$!")
wait_file "$out/relay.port"
env MIO205_ROOT="$out" MIO205_TRANSPORT="$transport" MIO205_ROLE=prepare "$binary" --exact "$test_name" --ignored --nocapture >"$out/prepare.log" 2>&1
for pair in a:provider b:consumer; do
  side=${pair%:*}
  role=${pair#*:}
  ip netns exec m205-$side env MIO205_ROOT="$out" MIO205_TRANSPORT="$transport" MIO205_ROLE="$role" MIO205_MOVING="$moving" "$binary" --exact "$test_name" --ignored --nocapture >"$out/$role.log" 2>&1 &
  pids+=("$!")
done
for side in a b; do
  ip netns exec m205-$side ip -ts monitor link address route >"$out/$side-netlink.log" 2>&1 &
  pids+=("$!")
done
wait_file "$out/provider.observation-ready"
wait_file "$out/consumer.observation-ready"
if [[ $transport == udp ]]; then
  rm -f "$out/observed-10.205.0.1" "$out/observed-10.205.0.2"
  touch "$out/hold-v4"
fi
touch "$out/start-observer"
wait_file "$out/provider.ready"
wait_file "$out/consumer.ready"
for side in a b; do
  if [[ $side == a ]]; then number=1; else number=2; fi
  peer=$((3-number))
  ip netns exec m205-$side ip6tables -D INPUT -s 2001:db8:205::$peer -p "$protocol" -j DROP
  if [[ $moving == provider && $side == b ]]; then continue; fi
  ip -n m205-$side -6 addr del 2001:db8:205::$number/64 dev lan0
  ip -n m205-$side -6 addr add 2001:db8:205:1::$number/64 dev lan0 nodad
  ip -n m205-$side -6 route replace default via 2001:db8:205:1::fe
done
touch "$out/prefix"
wait_file "$out/provider.prefix-refreshed"
wait_file "$out/consumer.prefix-refreshed"
touch "$out/release-v4"
verify_routes 1
wait_file "$out/provider.stage-1"
wait_file "$out/consumer.stage-1"
touch "$out/hold-work"
touch "$out/start-work"
wait_file "$out/provider.held"
wait_file "$out/consumer.held"
work_suffix=""
if [[ $transport == tcp ]]; then work_suffix=-20584; fi
wait_file "$out/work-observed-10.205.0.1$work_suffix"
if [[ $moving == both ]]; then wait_file "$out/work-observed-10.205.0.2$work_suffix"; fi
for side in a b; do
  if [[ $moving == provider && $side == b ]]; then continue; fi
  ip link del m205-${side}0
done
for side in a b; do
  if [[ $moving == provider && $side == b ]]; then continue; fi
  ip -n m205-$side link set lan1 up
  ip -n m205-$side route add default via 10.205.1.254
  ip -n m205-$side -6 route add default via 2001:db8:205:2::fe
done
touch "$out/interface"
verify_routes 2
status=0
wait "${pids[1]}" || status=$?
wait "${pids[2]}" || status=$?
for side in a b; do
  ip -n m205-$side addr show >"$out/$side-addresses.txt"
  ip -n m205-$side route show table all >"$out/$side-routes.txt"
done
rg 'NETWORK_CHANGE_(ORIGINAL|REDIAL)' "$out/provider.log" "$out/consumer.log"
exit "$status"
