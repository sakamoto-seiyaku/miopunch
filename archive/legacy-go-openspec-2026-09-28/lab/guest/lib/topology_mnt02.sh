#!/usr/bin/env bash
set -euo pipefail

NS_PREFIX="${MLAB_NS_PREFIX:-mlab}"

NS_WAN="${NS_PREFIX}-wan"
NS_NAT="${NS_PREFIX}-nat"
NS_COORD="${NS_PREFIX}-coord"
NS_STUN="${NS_PREFIX}-stun"

WAN_CIDR="${MLAB_WAN_CIDR:-100.64.0.0/24}"
WAN_GW_IP="${MLAB_WAN_GW_IP:-100.64.0.1}"

WAN_NAT_IP="${MLAB_WAN_NAT_IP:-100.64.0.2}"
COORD_IP="${MLAB_COORD_IP:-100.64.0.10}"
STUN_IP="${MLAB_STUN_IP:-100.64.0.11}"

LAN_CIDR="${MLAB_LAN_CIDR:-10.0.1.0/24}"
NAT_LAN_IP="${MLAB_NAT_LAN_IP:-10.0.1.1}"
PEER_IP_OCTET_BASE="${MLAB_PEER_IP_OCTET_BASE:-10}" # p1=10.0.1.10

mnt02_peer_ns() {
  local idx="$1"
  printf '%s-p%s' "${NS_PREFIX}" "${idx}"
}

mnt02_peer_ip() {
  local idx="$1"
  printf '10.0.1.%d' "$((PEER_IP_OCTET_BASE + idx - 1))"
}

mnt02_topology_cleanup() {
  # Best-effort cleanup for orphaned links from interrupted runs.
  local link
  for link in veth-nat-wan veth-wan-nat veth-coord veth-wan-coord veth-stun veth-wan-stun; do
    ip link del "${link}" 2>/dev/null || true
  done

  local ns
  for ns in "${NS_STUN}" "${NS_COORD}" "${NS_NAT}" "${NS_WAN}"; do
    if ip netns list | awk '{print $1}' | grep -qx "${ns}"; then
      ip netns del "${ns}"
    fi
  done

  # Remove any peer namespaces for this prefix (e.g. mlab-p1..mlab-pN).
  local prefix="${NS_PREFIX}-p"
  while read -r ns; do
    if [[ "${ns}" == "${prefix}"* ]]; then
      ip netns del "${ns}" 2>/dev/null || true
    fi
  done < <(ip netns list | awk '{print $1}')
}

mnt02_topology_create() {
  local peer_count="$1"
  [[ "${peer_count}" -ge 2 ]] || { echo "error: mnt02_topology_create: peer_count must be >=2" >&2; return 1; }

  ip netns add "${NS_WAN}"
  ip netns add "${NS_NAT}"
  ip netns add "${NS_COORD}"
  ip netns add "${NS_STUN}"

  local i ns
  for ((i=1; i<=peer_count; i++)); do
    ns="$(mnt02_peer_ns "${i}")"
    ip netns add "${ns}"
  done

  for ns in "${NS_WAN}" "${NS_NAT}" "${NS_COORD}" "${NS_STUN}"; do
    ip -n "${ns}" link set lo up
  done
  for ((i=1; i<=peer_count; i++)); do
    ns="$(mnt02_peer_ns "${i}")"
    ip -n "${ns}" link set lo up
  done

  # WAN bridge (br0).
  ip -n "${NS_WAN}" link add br0 type bridge
  ip -n "${NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${NS_WAN}" link set br0 up

  # NAT WAN <-> WAN bridge.
  ip link add veth-nat-wan type veth peer name veth-wan-nat
  ip link set veth-nat-wan netns "${NS_NAT}"
  ip link set veth-wan-nat netns "${NS_WAN}"
  ip -n "${NS_NAT}" link set veth-nat-wan name wan0
  ip -n "${NS_WAN}" link set veth-wan-nat name nat0
  ip -n "${NS_NAT}" addr add "${WAN_NAT_IP}/24" dev wan0
  ip -n "${NS_NAT}" link set wan0 up
  ip -n "${NS_WAN}" link set nat0 up
  ip -n "${NS_WAN}" link set nat0 master br0
  ip -n "${NS_NAT}" route replace default via "${WAN_GW_IP}" dev wan0

  # Coord/broker namespace attached to WAN.
  ip link add veth-coord type veth peer name veth-wan-coord
  ip link set veth-coord netns "${NS_COORD}"
  ip link set veth-wan-coord netns "${NS_WAN}"
  ip -n "${NS_COORD}" link set veth-coord name eth0
  ip -n "${NS_WAN}" link set veth-wan-coord name coord0
  ip -n "${NS_COORD}" addr add "${COORD_IP}/24" dev eth0
  ip -n "${NS_COORD}" link set eth0 up
  ip -n "${NS_WAN}" link set coord0 up
  ip -n "${NS_WAN}" link set coord0 master br0
  ip -n "${NS_COORD}" route replace default via "${WAN_GW_IP}" dev eth0

  # STUN namespace attached to WAN.
  ip link add veth-stun type veth peer name veth-wan-stun
  ip link set veth-stun netns "${NS_STUN}"
  ip link set veth-wan-stun netns "${NS_WAN}"
  ip -n "${NS_STUN}" link set veth-stun name eth0
  ip -n "${NS_WAN}" link set veth-wan-stun name stun0
  ip -n "${NS_STUN}" addr add "${STUN_IP}/24" dev eth0
  ip -n "${NS_STUN}" link set eth0 up
  ip -n "${NS_WAN}" link set stun0 up
  ip -n "${NS_WAN}" link set stun0 master br0
  ip -n "${NS_STUN}" route replace default via "${WAN_GW_IP}" dev eth0

  # LAN bridge (lan0) inside NAT namespace.
  ip -n "${NS_NAT}" link add lan0 type bridge
  ip -n "${NS_NAT}" addr add "${NAT_LAN_IP}/24" dev lan0
  ip -n "${NS_NAT}" link set lan0 up

  # Peers attached to NAT LAN bridge.
  for ((i=1; i<=peer_count; i++)); do
    local peer_ns peer_ip
    peer_ns="$(mnt02_peer_ns "${i}")"
    peer_ip="$(mnt02_peer_ip "${i}")"

    ip link add "veth-p${i}" type veth peer name "veth-nat-p${i}"
    ip link set "veth-p${i}" netns "${peer_ns}"
    ip link set "veth-nat-p${i}" netns "${NS_NAT}"

    ip -n "${peer_ns}" link set "veth-p${i}" name eth0
    ip -n "${NS_NAT}" link set "veth-nat-p${i}" name "p${i}"

    ip -n "${peer_ns}" addr add "${peer_ip}/24" dev eth0
    ip -n "${peer_ns}" link set eth0 up
    ip -n "${peer_ns}" route replace default via "${NAT_LAN_IP}" dev eth0

    ip -n "${NS_NAT}" link set "p${i}" up
    ip -n "${NS_NAT}" link set "p${i}" master lan0
  done
}

