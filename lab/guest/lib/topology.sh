#!/usr/bin/env bash
set -euo pipefail

NS_PREFIX="${MLAB_NS_PREFIX:-mlab}"

NS_WAN="${NS_PREFIX}-wan"
NS_NAT_A="${NS_PREFIX}-natA"
NS_PEER_A="${NS_PREFIX}-a"
NS_NAT_B="${NS_PREFIX}-natB"
NS_PEER_B="${NS_PREFIX}-b"
NS_COORD="${NS_PREFIX}-coord"
NS_STUN="${NS_PREFIX}-stun"
NS_PROBE="${NS_PREFIX}-probe"

WAN_CIDR="${MLAB_WAN_CIDR:-100.64.0.0/24}"
WAN_GW_IP="${MLAB_WAN_GW_IP:-100.64.0.1}"

WAN_A_IP="${MLAB_WAN_A_IP:-100.64.0.2}"
WAN_B_IP="${MLAB_WAN_B_IP:-100.64.0.3}"
COORD_IP="${MLAB_COORD_IP:-100.64.0.10}"
STUN_IP="${MLAB_STUN_IP:-100.64.0.11}"
PROBE_IP="${MLAB_PROBE_IP:-100.64.0.12}"

LAN_A_CIDR="${MLAB_LAN_A_CIDR:-10.0.1.0/24}"
LAN_B_CIDR="${MLAB_LAN_B_CIDR:-10.0.2.0/24}"

NAT_A_LAN_IP="${MLAB_NAT_A_LAN_IP:-10.0.1.1}"
NAT_B_LAN_IP="${MLAB_NAT_B_LAN_IP:-10.0.2.1}"

PEER_A_IP="${MLAB_PEER_A_IP:-10.0.1.2}"
PEER_B_IP="${MLAB_PEER_B_IP:-10.0.2.2}"

topology_cleanup() {
  local ns
  for ns in "${NS_PROBE}" "${NS_STUN}" "${NS_COORD}" "${NS_PEER_B}" "${NS_NAT_B}" "${NS_PEER_A}" "${NS_NAT_A}" "${NS_WAN}"; do
    if ip netns list | awk '{print $1}' | grep -qx "${ns}"; then
      ip netns del "${ns}"
    fi
  done
}

topology_create() {
  need_cmd ip

  ip netns add "${NS_WAN}"
  ip netns add "${NS_NAT_A}"
  ip netns add "${NS_PEER_A}"
  ip netns add "${NS_NAT_B}"
  ip netns add "${NS_PEER_B}"
  ip netns add "${NS_COORD}"
  ip netns add "${NS_STUN}"
  ip netns add "${NS_PROBE}"

  local ns
  for ns in "${NS_WAN}" "${NS_NAT_A}" "${NS_PEER_A}" "${NS_NAT_B}" "${NS_PEER_B}" "${NS_COORD}" "${NS_STUN}" "${NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  # WAN bridge
  ip -n "${NS_WAN}" link add br0 type bridge
  ip -n "${NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${NS_WAN}" link set br0 up

  # A: peer <-> NAT (LAN)
  ip link add veth-a type veth peer name veth-natA-lan
  ip link set veth-a netns "${NS_PEER_A}"
  ip link set veth-natA-lan netns "${NS_NAT_A}"
  ip -n "${NS_PEER_A}" link set veth-a name eth0
  ip -n "${NS_NAT_A}" link set veth-natA-lan name lan0
  ip -n "${NS_PEER_A}" addr add "${PEER_A_IP}/24" dev eth0
  ip -n "${NS_NAT_A}" addr add "${NAT_A_LAN_IP}/24" dev lan0
  ip -n "${NS_PEER_A}" link set eth0 up
  ip -n "${NS_NAT_A}" link set lan0 up
  ip -n "${NS_PEER_A}" route add default via "${NAT_A_LAN_IP}"

  # A: NAT (WAN) <-> WAN bridge
  ip link add veth-natA-wan type veth peer name veth-wan-natA
  ip link set veth-natA-wan netns "${NS_NAT_A}"
  ip link set veth-wan-natA netns "${NS_WAN}"
  ip -n "${NS_NAT_A}" link set veth-natA-wan name wan0
  ip -n "${NS_WAN}" link set veth-wan-natA name natA0
  ip -n "${NS_NAT_A}" addr add "${WAN_A_IP}/24" dev wan0
  ip -n "${NS_NAT_A}" link set wan0 up
  ip -n "${NS_WAN}" link set natA0 up
  ip -n "${NS_WAN}" link set natA0 master br0

  # B: peer <-> NAT (LAN)
  ip link add veth-b type veth peer name veth-natB-lan
  ip link set veth-b netns "${NS_PEER_B}"
  ip link set veth-natB-lan netns "${NS_NAT_B}"
  ip -n "${NS_PEER_B}" link set veth-b name eth0
  ip -n "${NS_NAT_B}" link set veth-natB-lan name lan0
  ip -n "${NS_PEER_B}" addr add "${PEER_B_IP}/24" dev eth0
  ip -n "${NS_NAT_B}" addr add "${NAT_B_LAN_IP}/24" dev lan0
  ip -n "${NS_PEER_B}" link set eth0 up
  ip -n "${NS_NAT_B}" link set lan0 up
  ip -n "${NS_PEER_B}" route add default via "${NAT_B_LAN_IP}"

  # B: NAT (WAN) <-> WAN bridge
  ip link add veth-natB-wan type veth peer name veth-wan-natB
  ip link set veth-natB-wan netns "${NS_NAT_B}"
  ip link set veth-wan-natB netns "${NS_WAN}"
  ip -n "${NS_NAT_B}" link set veth-natB-wan name wan0
  ip -n "${NS_WAN}" link set veth-wan-natB name natB0
  ip -n "${NS_NAT_B}" addr add "${WAN_B_IP}/24" dev wan0
  ip -n "${NS_NAT_B}" link set wan0 up
  ip -n "${NS_WAN}" link set natB0 up
  ip -n "${NS_WAN}" link set natB0 master br0

  # coord namespace attached to WAN
  ip link add veth-coord type veth peer name veth-wan-coord
  ip link set veth-coord netns "${NS_COORD}"
  ip link set veth-wan-coord netns "${NS_WAN}"
  ip -n "${NS_COORD}" link set veth-coord name eth0
  ip -n "${NS_WAN}" link set veth-wan-coord name coord0
  ip -n "${NS_COORD}" addr add "${COORD_IP}/24" dev eth0
  ip -n "${NS_COORD}" link set eth0 up
  ip -n "${NS_WAN}" link set coord0 up
  ip -n "${NS_WAN}" link set coord0 master br0

  # stun namespace attached to WAN
  ip link add veth-stun type veth peer name veth-wan-stun
  ip link set veth-stun netns "${NS_STUN}"
  ip link set veth-wan-stun netns "${NS_WAN}"
  ip -n "${NS_STUN}" link set veth-stun name eth0
  ip -n "${NS_WAN}" link set veth-wan-stun name stun0
  ip -n "${NS_STUN}" addr add "${STUN_IP}/24" dev eth0
  ip -n "${NS_STUN}" link set eth0 up
  ip -n "${NS_WAN}" link set stun0 up
  ip -n "${NS_WAN}" link set stun0 master br0

  # probe namespace attached to WAN (used to avoid polluting RFC4787 filtering validation)
  ip link add veth-probe type veth peer name veth-wan-probe
  ip link set veth-probe netns "${NS_PROBE}"
  ip link set veth-wan-probe netns "${NS_WAN}"
  ip -n "${NS_PROBE}" link set veth-probe name eth0
  ip -n "${NS_WAN}" link set veth-wan-probe name probe0
  ip -n "${NS_PROBE}" addr add "${PROBE_IP}/24" dev eth0
  ip -n "${NS_PROBE}" link set eth0 up
  ip -n "${NS_WAN}" link set probe0 up
  ip -n "${NS_WAN}" link set probe0 master br0
}
