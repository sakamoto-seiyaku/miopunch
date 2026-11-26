#!/bin/bash
# Run in a disposable namespace, for example
# unshare -Urnm bash scripts/check_native_lan.sh ABSOLUTE_TEST_BINARY ABSOLUTE_OUTPUT
# The binary is tests/lan_member.rs built with nat-lab-test-hooks.
set -euo pipefail
trap 'ip netns del lan-a 2>/dev/null || true; ip netns del lan-b 2>/dev/null || true' EXIT
binary=$1
out=$2
mkdir -p "$out"
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mkdir -p /run/netns
ip link set lo up
for mode in ipv4 ipv6; do
  directory="$out/$mode"
  mkdir -p "$directory"
  MIO_LAN_DIRECTORY="$directory" MIO_LAN_ROLE=prepare "$binary" --exact offline_lan_member --nocapture >"$directory/prepare.log" 2>&1
  rm -f "$directory/server/member.contacts.json" "$directory/client/member.contacts.json"
  ip netns add lan-a
  ip netns add lan-b
  ip -n lan-a link add decoy0 type dummy
  ip -n lan-a link add decoy1 type dummy
  ip -n lan-a link add decoy2 type dummy
  ip link add lan-a0 type veth peer name lan-b0
  ip link set lan-a0 netns lan-a
  ip link set lan-b0 netns lan-b
  for side in a b; do
    ip -n lan-$side link set lo up
    ip -n lan-$side link set lan-${side}0 name lan0
    ip -n lan-$side link set lan0 up
  done
  if [[ "$mode" == ipv4 ]]; then
    ip -n lan-a addr add 10.196.0.1/24 dev lan0
    ip -n lan-b addr add 10.196.0.2/24 dev lan0
    for side in a b; do ip netns exec lan-$side sysctl -qw net.ipv6.conf.all.disable_ipv6=1; done
    extra=()
  else
    ip -n lan-a -6 addr flush dev lan0
    ip -n lan-b -6 addr flush dev lan0
    ip -n lan-a -6 addr add fe80::196:1/64 dev lan0 nodad
    ip -n lan-b -6 addr add fe80::196:2/64 dev lan0 nodad
    extra=(MIO_LAN_IPV6_ONLY=1)
  fi
  for side in a b; do
    ip -n lan-$side addr >"$directory/$side-addresses.txt"
    ip -n lan-$side route show table all >"$directory/$side-routes.txt"
    ip -n lan-$side -6 route show table all >>"$directory/$side-routes.txt"
  done
  a_index=$(ip -n lan-a -j link show lan0 | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["ifindex"])')
  b_index=$(ip -n lan-b -j link show lan0 | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["ifindex"])')
  [[ "$a_index" != "$b_index" ]]
  ip netns exec lan-a env MIO_LAN_IFINDEX="$a_index" MIO_LAN_DIRECTORY="$directory" MIO_LAN_ROLE=server "${extra[@]}" "$binary" --exact offline_lan_member --nocapture >"$directory/server.log" 2>&1 &
  server_pid=$!
  ip netns exec lan-b env MIO_LAN_IFINDEX="$b_index" MIO_LAN_DIRECTORY="$directory" MIO_LAN_ROLE=client "${extra[@]}" "$binary" --exact offline_lan_member --nocapture >"$directory/client.log" 2>&1 &
  client_pid=$!
  status=0
  wait "$client_pid" || status=$?
  wait "$server_pid" || status=$?
  ip netns del lan-a
  ip netns del lan-b
  if [[ "$status" != 0 ]]; then cat "$directory/server.log" "$directory/client.log"; exit "$status"; fi
  rg LAN_DATA_OK "$directory/server.log" "$directory/client.log"
done
