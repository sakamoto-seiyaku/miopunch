#!/usr/bin/env bash
set -euo pipefail

MNT03_NS_PREFIX="${MLAB_MNT03_NS_PREFIX:-mlab-mnt03}"

MNT03_NS_WAN="${MNT03_NS_PREFIX}-wan"
MNT03_NS_NAT_N01="${MNT03_NS_PREFIX}-nat-n01"
MNT03_NS_NAT_N02="${MNT03_NS_PREFIX}-nat-n02"
MNT03_NS_NAT_N03="${MNT03_NS_PREFIX}-nat-n03"
MNT03_NS_NAT_N04="${MNT03_NS_PREFIX}-nat-n04"
MNT03_NS_NAT_N05="${MNT03_NS_PREFIX}-nat-n05"
MNT03_NS_NAT_N06="${MNT03_NS_PREFIX}-nat-n06"
MNT03_NS_NAT_N07="${MNT03_NS_PREFIX}-nat-n07"
MNT03_NS_NAT_N08="${MNT03_NS_PREFIX}-nat-n08"
MNT03_NS_NAT_N09="${MNT03_NS_PREFIX}-nat-n09"
MNT03_NS_NAT_N10="${MNT03_NS_PREFIX}-nat-n10"
MNT03_NS_NAT_N11="${MNT03_NS_PREFIX}-nat-n11"
MNT03_NS_NAT_N12="${MNT03_NS_PREFIX}-nat-n12"
MNT03_NS_COORD="${MNT03_NS_PREFIX}-coord"
MNT03_NS_STUN="${MNT03_NS_PREFIX}-stun"
MNT03_NS_PROBE="${MNT03_NS_PREFIX}-probe"

WAN_CIDR="${MLAB_MNT03_WAN_CIDR:-100.65.0.0/24}"
WAN_GW_IP="${MLAB_MNT03_WAN_GW_IP:-100.65.0.1}"
WAN_N01_IP="${MLAB_MNT03_WAN_N01_IP:-100.65.0.2}"
WAN_N02_IP="${MLAB_MNT03_WAN_N02_IP:-100.65.0.8}"
WAN_N03_IP="${MLAB_MNT03_WAN_N03_IP:-100.65.0.3}"
WAN_N04_IP="${MLAB_MNT03_WAN_N04_IP:-100.65.0.4}"
WAN_N05_IP="${MLAB_MNT03_WAN_N05_IP:-100.65.0.5}"
WAN_N06_IP="${MLAB_MNT03_WAN_N06_IP:-100.65.0.6}"
WAN_N07_IP="${MLAB_MNT03_WAN_N07_IP:-100.65.0.7}"
WAN_N08_IP="${MLAB_MNT03_WAN_N08_IP:-100.65.0.21}"
WAN_N09_IP="${MLAB_MNT03_WAN_N09_IP:-100.65.0.22}"
WAN_N10_IP="${MLAB_MNT03_WAN_N10_IP:-100.65.0.23}"
WAN_N11_IP="${MLAB_MNT03_WAN_N11_IP:-100.65.0.24}"
WAN_N12_IP="${MLAB_MNT03_WAN_N12_IP:-100.65.0.25}"
COORD_IP="${MLAB_MNT03_COORD_IP:-100.65.0.10}"
STUN_IP="${MLAB_MNT03_STUN_IP:-100.65.0.11}"
PROBE_IP="${MLAB_MNT03_PROBE_IP:-100.65.0.12}"

WAN_GW_V6_CIDR="${MLAB_MNT03_WAN_GW_V6_CIDR:-fd31:6d69:6f70:3000::1/64}"
WAN_GW_V6_IP="${MLAB_MNT03_WAN_GW_V6_IP:-fd31:6d69:6f70:3000::1}"
WAN_N01_V6_IP="${MLAB_MNT03_WAN_N01_V6_IP:-fd31:6d69:6f70:3000::101}"
WAN_N03_V6_IP="${MLAB_MNT03_WAN_N03_V6_IP:-fd31:6d69:6f70:3000::103}"
WAN_N06_V6_IP="${MLAB_MNT03_WAN_N06_V6_IP:-fd31:6d69:6f70:3000::106}"

MNT03_N01_LAN_CIDR="${MLAB_MNT03_N01_LAN_CIDR:-10.31.1.0/24}"
MNT03_N02_LAN_CIDR="${MLAB_MNT03_N02_LAN_CIDR:-10.31.2.0/24}"
MNT03_N03_LAN_CIDR="${MLAB_MNT03_N03_LAN_CIDR:-10.31.3.0/24}"
MNT03_N04_LAN_CIDR="${MLAB_MNT03_N04_LAN_CIDR:-10.31.4.0/24}"
MNT03_N05_LAN_CIDR="${MLAB_MNT03_N05_LAN_CIDR:-10.31.5.0/24}"
MNT03_N06_LAN_CIDR="${MLAB_MNT03_N06_LAN_CIDR:-10.31.6.0/24}"
MNT03_N07_LAN_CIDR="${MLAB_MNT03_N07_LAN_CIDR:-10.31.7.0/24}"
MNT03_N08_LAN_CIDR="${MLAB_MNT03_N08_LAN_CIDR:-10.31.8.0/24}"
MNT03_N09_LAN_CIDR="${MLAB_MNT03_N09_LAN_CIDR:-10.31.9.0/24}"
MNT03_N10_LAN_CIDR="${MLAB_MNT03_N10_LAN_CIDR:-10.31.10.0/24}"
MNT03_N11_LAN_CIDR="${MLAB_MNT03_N11_LAN_CIDR:-10.31.11.0/24}"
MNT03_N12_LAN_CIDR="${MLAB_MNT03_N12_LAN_CIDR:-10.31.12.0/24}"
MNT03_N01_GW_IP="${MLAB_MNT03_N01_GW_IP:-10.31.1.1}"
MNT03_N02_GW_IP="${MLAB_MNT03_N02_GW_IP:-10.31.2.1}"
MNT03_N03_GW_IP="${MLAB_MNT03_N03_GW_IP:-10.31.3.1}"
MNT03_N04_GW_IP="${MLAB_MNT03_N04_GW_IP:-10.31.4.1}"
MNT03_N05_GW_IP="${MLAB_MNT03_N05_GW_IP:-10.31.5.1}"
MNT03_N06_GW_IP="${MLAB_MNT03_N06_GW_IP:-10.31.6.1}"
MNT03_N07_GW_IP="${MLAB_MNT03_N07_GW_IP:-10.31.7.1}"
MNT03_N08_GW_IP="${MLAB_MNT03_N08_GW_IP:-10.31.8.1}"
MNT03_N09_GW_IP="${MLAB_MNT03_N09_GW_IP:-10.31.9.1}"
MNT03_N10_GW_IP="${MLAB_MNT03_N10_GW_IP:-10.31.10.1}"
MNT03_N11_GW_IP="${MLAB_MNT03_N11_GW_IP:-10.31.11.1}"
MNT03_N12_GW_IP="${MLAB_MNT03_N12_GW_IP:-10.31.12.1}"
MNT03_N01_IP="${MLAB_MNT03_N01_IP:-10.31.1.2}"
MNT03_N02_IP="${MLAB_MNT03_N02_IP:-10.31.2.2}"
MNT03_N03_IP="${MLAB_MNT03_N03_IP:-10.31.3.2}"
MNT03_N04_IP="${MLAB_MNT03_N04_IP:-10.31.4.2}"
MNT03_N05_IP="${MLAB_MNT03_N05_IP:-10.31.5.2}"
MNT03_N06_IP="${MLAB_MNT03_N06_IP:-10.31.6.2}"
MNT03_N07_IP="${MLAB_MNT03_N07_IP:-10.31.7.2}"
MNT03_N08_IP="${MLAB_MNT03_N08_IP:-10.31.8.2}"
MNT03_N09_IP="${MLAB_MNT03_N09_IP:-10.31.9.2}"
MNT03_N10_IP="${MLAB_MNT03_N10_IP:-10.31.10.2}"
MNT03_N11_IP="${MLAB_MNT03_N11_IP:-10.31.11.2}"
MNT03_N12_IP="${MLAB_MNT03_N12_IP:-10.31.12.2}"

MNT03_N01_LAN_V6_CIDR="${MLAB_MNT03_N01_LAN_V6_CIDR:-fd31:6d69:6f70:3101::/64}"
MNT03_N03_LAN_V6_CIDR="${MLAB_MNT03_N03_LAN_V6_CIDR:-fd31:6d69:6f70:3103::/64}"
MNT03_N06_LAN_V6_CIDR="${MLAB_MNT03_N06_LAN_V6_CIDR:-fd31:6d69:6f70:3106::/64}"
MNT03_N01_GW_V6_IP="${MLAB_MNT03_N01_GW_V6_IP:-fd31:6d69:6f70:3101::1}"
MNT03_N03_GW_V6_IP="${MLAB_MNT03_N03_GW_V6_IP:-fd31:6d69:6f70:3103::1}"
MNT03_N06_GW_V6_IP="${MLAB_MNT03_N06_GW_V6_IP:-fd31:6d69:6f70:3106::1}"
MNT03_N01_V6_IP="${MLAB_MNT03_N01_V6_IP:-fd31:6d69:6f70:3101::2}"
MNT03_N03_V6_IP="${MLAB_MNT03_N03_V6_IP:-fd31:6d69:6f70:3103::2}"
MNT03_N06_V6_IP="${MLAB_MNT03_N06_V6_IP:-fd31:6d69:6f70:3106::2}"

mnt03_node_nat_ns() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_NS_NAT_N01}" ;;
    n02) printf '%s' "${MNT03_NS_NAT_N02}" ;;
    n03) printf '%s' "${MNT03_NS_NAT_N03}" ;;
    n04) printf '%s' "${MNT03_NS_NAT_N04}" ;;
    n05) printf '%s' "${MNT03_NS_NAT_N05}" ;;
    n06) printf '%s' "${MNT03_NS_NAT_N06}" ;;
    n07) printf '%s' "${MNT03_NS_NAT_N07}" ;;
    n08) printf '%s' "${MNT03_NS_NAT_N08}" ;;
    n09) printf '%s' "${MNT03_NS_NAT_N09}" ;;
    n10) printf '%s' "${MNT03_NS_NAT_N10}" ;;
    n11) printf '%s' "${MNT03_NS_NAT_N11}" ;;
    n12) printf '%s' "${MNT03_NS_NAT_N12}" ;;
    *) die "MNT-03 has no NAT namespace for node: ${node}" ;;
  esac
}

mnt03_node_lan_cidr() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_LAN_CIDR}" ;;
    n02) printf '%s' "${MNT03_N02_LAN_CIDR}" ;;
    n03) printf '%s' "${MNT03_N03_LAN_CIDR}" ;;
    n04) printf '%s' "${MNT03_N04_LAN_CIDR}" ;;
    n05) printf '%s' "${MNT03_N05_LAN_CIDR}" ;;
    n06) printf '%s' "${MNT03_N06_LAN_CIDR}" ;;
    n07) printf '%s' "${MNT03_N07_LAN_CIDR}" ;;
    n08) printf '%s' "${MNT03_N08_LAN_CIDR}" ;;
    n09) printf '%s' "${MNT03_N09_LAN_CIDR}" ;;
    n10) printf '%s' "${MNT03_N10_LAN_CIDR}" ;;
    n11) printf '%s' "${MNT03_N11_LAN_CIDR}" ;;
    n12) printf '%s' "${MNT03_N12_LAN_CIDR}" ;;
    *) die "MNT-03 has no LAN CIDR for node: ${node}" ;;
  esac
}

mnt03_node_has_ipv6() {
  local node="$1"
  case "${node}" in
    n01|n03|n06) return 0 ;;
    *) return 1 ;;
  esac
}

mnt03_node_lan_v6_cidr() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_LAN_V6_CIDR}" ;;
    n03) printf '%s' "${MNT03_N03_LAN_V6_CIDR}" ;;
    n06) printf '%s' "${MNT03_N06_LAN_V6_CIDR}" ;;
    *) die "MNT-03 has no IPv6 LAN CIDR for node: ${node}" ;;
  esac
}

mnt03_node_wan_ip() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${WAN_N01_IP}" ;;
    n02) printf '%s' "${WAN_N02_IP}" ;;
    n03) printf '%s' "${WAN_N03_IP}" ;;
    n04) printf '%s' "${WAN_N04_IP}" ;;
    n05) printf '%s' "${WAN_N05_IP}" ;;
    n06) printf '%s' "${WAN_N06_IP}" ;;
    n07) printf '%s' "${WAN_N07_IP}" ;;
    n08) printf '%s' "${WAN_N08_IP}" ;;
    n09) printf '%s' "${WAN_N09_IP}" ;;
    n10) printf '%s' "${WAN_N10_IP}" ;;
    n11) printf '%s' "${WAN_N11_IP}" ;;
    n12) printf '%s' "${WAN_N12_IP}" ;;
    *) die "MNT-03 has no WAN IP for node: ${node}" ;;
  esac
}

mnt03_node_wan_v6_ip() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${WAN_N01_V6_IP}" ;;
    n03) printf '%s' "${WAN_N03_V6_IP}" ;;
    n06) printf '%s' "${WAN_N06_V6_IP}" ;;
    *) die "MNT-03 has no IPv6 WAN IP for node: ${node}" ;;
  esac
}

mnt03_node_ip() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_IP}" ;;
    n02) printf '%s' "${MNT03_N02_IP}" ;;
    n03) printf '%s' "${MNT03_N03_IP}" ;;
    n04) printf '%s' "${MNT03_N04_IP}" ;;
    n05) printf '%s' "${MNT03_N05_IP}" ;;
    n06) printf '%s' "${MNT03_N06_IP}" ;;
    n07) printf '%s' "${MNT03_N07_IP}" ;;
    n08) printf '%s' "${MNT03_N08_IP}" ;;
    n09) printf '%s' "${MNT03_N09_IP}" ;;
    n10) printf '%s' "${MNT03_N10_IP}" ;;
    n11) printf '%s' "${MNT03_N11_IP}" ;;
    n12) printf '%s' "${MNT03_N12_IP}" ;;
    *) die "MNT-03 has no node IP for node: ${node}" ;;
  esac
}

mnt03_node_v6_ip() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_V6_IP}" ;;
    n03) printf '%s' "${MNT03_N03_V6_IP}" ;;
    n06) printf '%s' "${MNT03_N06_V6_IP}" ;;
    *) die "MNT-03 has no node IPv6 address for node: ${node}" ;;
  esac
}

mnt03_node_gateway() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_GW_IP}" ;;
    n02) printf '%s' "${MNT03_N02_GW_IP}" ;;
    n03) printf '%s' "${MNT03_N03_GW_IP}" ;;
    n04) printf '%s' "${MNT03_N04_GW_IP}" ;;
    n05) printf '%s' "${MNT03_N05_GW_IP}" ;;
    n06) printf '%s' "${MNT03_N06_GW_IP}" ;;
    n07) printf '%s' "${MNT03_N07_GW_IP}" ;;
    n08) printf '%s' "${MNT03_N08_GW_IP}" ;;
    n09) printf '%s' "${MNT03_N09_GW_IP}" ;;
    n10) printf '%s' "${MNT03_N10_GW_IP}" ;;
    n11) printf '%s' "${MNT03_N11_GW_IP}" ;;
    n12) printf '%s' "${MNT03_N12_GW_IP}" ;;
    *) die "MNT-03 has no gateway for node: ${node}" ;;
  esac
}

mnt03_node_v6_gateway() {
  local node="$1"
  case "${node}" in
    n01) printf '%s' "${MNT03_N01_GW_V6_IP}" ;;
    n03) printf '%s' "${MNT03_N03_GW_V6_IP}" ;;
    n06) printf '%s' "${MNT03_N06_GW_V6_IP}" ;;
    *) die "MNT-03 has no IPv6 gateway for node: ${node}" ;;
  esac
}

mnt03_topology_cleanup() {
  local node link
  for node in n01 n02 n03 n04 n05 n06 n07 n08 n09 n10 n11 n12; do
    for link in "m3${node}w" "m3w${node}" "veth-${node}" "veth-nat-${node}"; do
      ip link del "${link}" 2>/dev/null || true
    done
  done

  for link in \
    m3coord m3wcoord \
    m3stun m3wstun \
    m3probe m3wprobe; do
    ip link del "${link}" 2>/dev/null || true
  done

  local ns
  for ns in \
    "${MNT03_NS_PROBE}" \
    "${MNT03_NS_STUN}" \
    "${MNT03_NS_COORD}" \
    "${MNT03_NS_NAT_N12}" \
    "${MNT03_NS_NAT_N11}" \
    "${MNT03_NS_NAT_N10}" \
    "${MNT03_NS_NAT_N09}" \
    "${MNT03_NS_NAT_N08}" \
    "${MNT03_NS_NAT_N07}" \
    "${MNT03_NS_NAT_N06}" \
    "${MNT03_NS_NAT_N05}" \
    "${MNT03_NS_NAT_N04}" \
    "${MNT03_NS_NAT_N03}" \
    "${MNT03_NS_NAT_N02}" \
    "${MNT03_NS_NAT_N01}" \
    "${MNT03_NS_WAN}"; do
    if ip netns list | awk '{print $1}' | grep -qx "${ns}"; then
      ip netns del "${ns}" 2>/dev/null || true
    fi
  done
}

mnt03_topology_create_3node() {
  need_cmd ip

  ip netns add "${MNT03_NS_WAN}"
  ip netns add "${MNT03_NS_NAT_N01}"
  ip netns add "${MNT03_NS_NAT_N03}"
  ip netns add "${MNT03_NS_NAT_N04}"
  ip netns add "${MNT03_NS_COORD}"
  ip netns add "${MNT03_NS_STUN}"
  ip netns add "${MNT03_NS_PROBE}"

  local ns
  for ns in "${MNT03_NS_WAN}" "${MNT03_NS_NAT_N01}" "${MNT03_NS_NAT_N03}" "${MNT03_NS_NAT_N04}" "${MNT03_NS_COORD}" "${MNT03_NS_STUN}" "${MNT03_NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  ip -n "${MNT03_NS_WAN}" link add br0 type bridge
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_V6_CIDR}" dev br0
  ip -n "${MNT03_NS_WAN}" link set br0 up
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.all.forwarding=1'
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.default.forwarding=1'

  mnt03_attach_nat_wan "${MNT03_NS_NAT_N01}" n01 "${WAN_N01_IP}" \
    m3n01w m3wn01 n01wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N03}" n03 "${WAN_N03_IP}" \
    m3n03w m3wn03 n03wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N04}" n04 "${WAN_N04_IP}" \
    m3n04w m3wn04 n04wan0
  mnt03_attach_wan_service "${MNT03_NS_COORD}" "${COORD_IP}" \
    m3coord m3wcoord coord0
  mnt03_attach_wan_service "${MNT03_NS_STUN}" "${STUN_IP}" \
    m3stun m3wstun stun0
  mnt03_attach_wan_service "${MNT03_NS_PROBE}" "${PROBE_IP}" \
    m3probe m3wprobe probe0

  mnt03_create_nat_lan "${MNT03_NS_NAT_N01}" "${MNT03_N01_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N03}" "${MNT03_N03_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N04}" "${MNT03_N04_GW_IP}"
}

mnt03_topology_create_4node() {
  need_cmd ip

  ip netns add "${MNT03_NS_WAN}"
  ip netns add "${MNT03_NS_NAT_N01}"
  ip netns add "${MNT03_NS_NAT_N03}"
  ip netns add "${MNT03_NS_NAT_N04}"
  ip netns add "${MNT03_NS_NAT_N05}"
  ip netns add "${MNT03_NS_COORD}"
  ip netns add "${MNT03_NS_STUN}"
  ip netns add "${MNT03_NS_PROBE}"

  local ns
  for ns in "${MNT03_NS_WAN}" "${MNT03_NS_NAT_N01}" "${MNT03_NS_NAT_N03}" "${MNT03_NS_NAT_N04}" "${MNT03_NS_NAT_N05}" "${MNT03_NS_COORD}" "${MNT03_NS_STUN}" "${MNT03_NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  ip -n "${MNT03_NS_WAN}" link add br0 type bridge
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${MNT03_NS_WAN}" link set br0 up

  mnt03_attach_nat_wan "${MNT03_NS_NAT_N01}" n01 "${WAN_N01_IP}" \
    m3n01w m3wn01 n01wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N03}" n03 "${WAN_N03_IP}" \
    m3n03w m3wn03 n03wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N04}" n04 "${WAN_N04_IP}" \
    m3n04w m3wn04 n04wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N05}" n05 "${WAN_N05_IP}" \
    m3n05w m3wn05 n05wan0
  mnt03_attach_wan_service "${MNT03_NS_COORD}" "${COORD_IP}" \
    m3coord m3wcoord coord0
  mnt03_attach_wan_service "${MNT03_NS_STUN}" "${STUN_IP}" \
    m3stun m3wstun stun0
  mnt03_attach_wan_service "${MNT03_NS_PROBE}" "${PROBE_IP}" \
    m3probe m3wprobe probe0

  mnt03_create_nat_lan "${MNT03_NS_NAT_N01}" "${MNT03_N01_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N03}" "${MNT03_N03_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N04}" "${MNT03_N04_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N05}" "${MNT03_N05_GW_IP}"
}

mnt03_topology_create_6node() {
  need_cmd ip

  ip netns add "${MNT03_NS_WAN}"
  ip netns add "${MNT03_NS_NAT_N01}"
  ip netns add "${MNT03_NS_NAT_N03}"
  ip netns add "${MNT03_NS_NAT_N04}"
  ip netns add "${MNT03_NS_NAT_N05}"
  ip netns add "${MNT03_NS_NAT_N06}"
  ip netns add "${MNT03_NS_NAT_N07}"
  ip netns add "${MNT03_NS_COORD}"
  ip netns add "${MNT03_NS_STUN}"
  ip netns add "${MNT03_NS_PROBE}"

  local ns
  for ns in "${MNT03_NS_WAN}" "${MNT03_NS_NAT_N01}" "${MNT03_NS_NAT_N03}" "${MNT03_NS_NAT_N04}" "${MNT03_NS_NAT_N05}" "${MNT03_NS_NAT_N06}" "${MNT03_NS_NAT_N07}" "${MNT03_NS_COORD}" "${MNT03_NS_STUN}" "${MNT03_NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  ip -n "${MNT03_NS_WAN}" link add br0 type bridge
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_V6_CIDR}" dev br0
  ip -n "${MNT03_NS_WAN}" link set br0 up
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.all.forwarding=1'
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.default.forwarding=1'

  mnt03_attach_nat_wan "${MNT03_NS_NAT_N01}" n01 "${WAN_N01_IP}" \
    m3n01w m3wn01 n01wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N03}" n03 "${WAN_N03_IP}" \
    m3n03w m3wn03 n03wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N04}" n04 "${WAN_N04_IP}" \
    m3n04w m3wn04 n04wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N05}" n05 "${WAN_N05_IP}" \
    m3n05w m3wn05 n05wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N06}" n06 "${WAN_N06_IP}" \
    m3n06w m3wn06 n06wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N07}" n07 "${WAN_N07_IP}" \
    m3n07w m3wn07 n07wan0
  mnt03_attach_wan_service "${MNT03_NS_COORD}" "${COORD_IP}" \
    m3coord m3wcoord coord0
  mnt03_attach_wan_service "${MNT03_NS_STUN}" "${STUN_IP}" \
    m3stun m3wstun stun0
  mnt03_attach_wan_service "${MNT03_NS_PROBE}" "${PROBE_IP}" \
    m3probe m3wprobe probe0

  mnt03_create_nat_lan "${MNT03_NS_NAT_N01}" "${MNT03_N01_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N03}" "${MNT03_N03_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N04}" "${MNT03_N04_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N05}" "${MNT03_N05_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N06}" "${MNT03_N06_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N07}" "${MNT03_N07_GW_IP}"
  mnt03_configure_routed_ipv6 n01
  mnt03_configure_routed_ipv6 n03
  mnt03_configure_routed_ipv6 n06
}

mnt03_topology_create_12node() {
  need_cmd ip

  ip netns add "${MNT03_NS_WAN}"
  local node
  for node in n01 n02 n03 n04 n05 n06 n07 n08 n09 n10 n11 n12; do
    ip netns add "$(mnt03_node_nat_ns "${node}")"
  done
  ip netns add "${MNT03_NS_COORD}"
  ip netns add "${MNT03_NS_STUN}"
  ip netns add "${MNT03_NS_PROBE}"

  local ns
  for ns in \
    "${MNT03_NS_WAN}" \
    "${MNT03_NS_NAT_N01}" \
    "${MNT03_NS_NAT_N02}" \
    "${MNT03_NS_NAT_N03}" \
    "${MNT03_NS_NAT_N04}" \
    "${MNT03_NS_NAT_N05}" \
    "${MNT03_NS_NAT_N06}" \
    "${MNT03_NS_NAT_N07}" \
    "${MNT03_NS_NAT_N08}" \
    "${MNT03_NS_NAT_N09}" \
    "${MNT03_NS_NAT_N10}" \
    "${MNT03_NS_NAT_N11}" \
    "${MNT03_NS_NAT_N12}" \
    "${MNT03_NS_COORD}" \
    "${MNT03_NS_STUN}" \
    "${MNT03_NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  ip -n "${MNT03_NS_WAN}" link add br0 type bridge
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_V6_CIDR}" dev br0
  ip -n "${MNT03_NS_WAN}" link set br0 up
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.all.forwarding=1'
  ns_exec "${MNT03_NS_WAN}" sh -c 'sysctl -qw net.ipv6.conf.default.forwarding=1'

  for node in n01 n02 n03 n04 n05 n06 n07 n08 n09 n10 n11 n12; do
    mnt03_attach_nat_wan \
      "$(mnt03_node_nat_ns "${node}")" \
      "${node}" \
      "$(mnt03_node_wan_ip "${node}")" \
      "m3${node}w" \
      "m3w${node}" \
      "${node}wan0"
  done
  mnt03_attach_wan_service "${MNT03_NS_COORD}" "${COORD_IP}" \
    m3coord m3wcoord coord0
  mnt03_attach_wan_service "${MNT03_NS_STUN}" "${STUN_IP}" \
    m3stun m3wstun stun0
  mnt03_attach_wan_service "${MNT03_NS_PROBE}" "${PROBE_IP}" \
    m3probe m3wprobe probe0

  for node in n01 n02 n03 n04 n05 n06 n07 n08 n09 n10 n11 n12; do
    mnt03_create_nat_lan "$(mnt03_node_nat_ns "${node}")" "$(mnt03_node_gateway "${node}")"
  done
  mnt03_configure_routed_ipv6 n01
  mnt03_configure_routed_ipv6 n03
  mnt03_configure_routed_ipv6 n06
}

mnt03_topology_create_2node() {
  need_cmd ip

  ip netns add "${MNT03_NS_WAN}"
  ip netns add "${MNT03_NS_NAT_N01}"
  ip netns add "${MNT03_NS_NAT_N03}"
  ip netns add "${MNT03_NS_COORD}"
  ip netns add "${MNT03_NS_STUN}"
  ip netns add "${MNT03_NS_PROBE}"

  local ns
  for ns in "${MNT03_NS_WAN}" "${MNT03_NS_NAT_N01}" "${MNT03_NS_NAT_N03}" "${MNT03_NS_COORD}" "${MNT03_NS_STUN}" "${MNT03_NS_PROBE}"; do
    ip -n "${ns}" link set lo up
  done

  ip -n "${MNT03_NS_WAN}" link add br0 type bridge
  ip -n "${MNT03_NS_WAN}" addr add "${WAN_GW_IP}/24" dev br0
  ip -n "${MNT03_NS_WAN}" link set br0 up

  mnt03_attach_nat_wan "${MNT03_NS_NAT_N01}" n01 "${WAN_N01_IP}" \
    m3n01w m3wn01 n01wan0
  mnt03_attach_nat_wan "${MNT03_NS_NAT_N03}" n03 "${WAN_N03_IP}" \
    m3n03w m3wn03 n03wan0
  mnt03_attach_wan_service "${MNT03_NS_COORD}" "${COORD_IP}" \
    m3coord m3wcoord coord0
  mnt03_attach_wan_service "${MNT03_NS_STUN}" "${STUN_IP}" \
    m3stun m3wstun stun0
  mnt03_attach_wan_service "${MNT03_NS_PROBE}" "${PROBE_IP}" \
    m3probe m3wprobe probe0

  mnt03_create_nat_lan "${MNT03_NS_NAT_N01}" "${MNT03_N01_GW_IP}"
  mnt03_create_nat_lan "${MNT03_NS_NAT_N03}" "${MNT03_N03_GW_IP}"
}

mnt03_attach_nat_wan() {
  local nat_ns="$1" label="$2" wan_ip="$3" nat_link="$4" wan_link="$5" wan_name="$6"

  ip link add "${nat_link}" type veth peer name "${wan_link}"
  ip link set "${nat_link}" netns "${nat_ns}"
  ip link set "${wan_link}" netns "${MNT03_NS_WAN}"
  ip -n "${nat_ns}" link set "${nat_link}" name wan0
  ip -n "${MNT03_NS_WAN}" link set "${wan_link}" name "${wan_name}"
  ip -n "${nat_ns}" addr add "${wan_ip}/24" dev wan0
  ip -n "${nat_ns}" link set wan0 up
  ip -n "${MNT03_NS_WAN}" link set "${wan_name}" up
  ip -n "${MNT03_NS_WAN}" link set "${wan_name}" master br0
  ip -n "${nat_ns}" route replace default via "${WAN_GW_IP}" dev wan0
  ns_exec "${nat_ns}" sh -c 'echo 1 > /proc/sys/net/ipv4/ip_forward'
  ip -n "${nat_ns}" link set lo up

  printf 'nat=%s ns=%s wan_ip=%s\n' "${label}" "${nat_ns}" "${wan_ip}" >/dev/null
}

mnt03_attach_wan_service() {
  local service_ns="$1" service_ip="$2" service_link="$3" wan_link="$4" wan_name="$5"

  ip link add "${service_link}" type veth peer name "${wan_link}"
  ip link set "${service_link}" netns "${service_ns}"
  ip link set "${wan_link}" netns "${MNT03_NS_WAN}"
  ip -n "${service_ns}" link set "${service_link}" name eth0
  ip -n "${MNT03_NS_WAN}" link set "${wan_link}" name "${wan_name}"
  ip -n "${service_ns}" addr add "${service_ip}/24" dev eth0
  ip -n "${service_ns}" link set eth0 up
  ip -n "${MNT03_NS_WAN}" link set "${wan_name}" up
  ip -n "${MNT03_NS_WAN}" link set "${wan_name}" master br0
  ip -n "${service_ns}" route replace default via "${WAN_GW_IP}" dev eth0
}

mnt03_create_nat_lan() {
  local nat_ns="$1" gw_ip="$2"

  ip -n "${nat_ns}" link add lan0 type bridge
  ip -n "${nat_ns}" addr add "${gw_ip}/24" dev lan0
  ip -n "${nat_ns}" link set lan0 up
}

mnt03_configure_routed_ipv6() {
  local node="$1"
  mnt03_node_has_ipv6 "${node}" || return 0

  local nat_ns wan_v6 lan_v6 gw_v6 lan_cidr
  nat_ns="$(mnt03_node_nat_ns "${node}")"
  wan_v6="$(mnt03_node_wan_v6_ip "${node}")"
  lan_v6="$(mnt03_node_v6_gateway "${node}")"
  gw_v6="${WAN_GW_V6_IP}"
  lan_cidr="$(mnt03_node_lan_v6_cidr "${node}")"

  ns_exec "${nat_ns}" sh -c 'sysctl -qw net.ipv6.conf.all.disable_ipv6=0 || true'
  ns_exec "${nat_ns}" sh -c 'sysctl -qw net.ipv6.conf.default.disable_ipv6=0 || true'
  ns_exec "${nat_ns}" sh -c 'sysctl -qw net.ipv6.conf.all.forwarding=1'
  ns_exec "${nat_ns}" sh -c 'sysctl -qw net.ipv6.conf.default.forwarding=1'
  ip -n "${nat_ns}" addr add "${wan_v6}/64" dev wan0
  ip -n "${nat_ns}" addr add "${lan_v6}/64" dev lan0
  ip -n "${nat_ns}" -6 route replace default via "${gw_v6}" dev wan0
  ip -n "${MNT03_NS_WAN}" -6 route replace "${lan_cidr}" via "${wan_v6}" dev br0
}
