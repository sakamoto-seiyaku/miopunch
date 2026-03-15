#!/usr/bin/env bash
set -euo pipefail

profiles_apply() {
  local ns="$1"
  local profile="$2"
  local lan_cidr="$3"
  local wan_ip="$4"
  local peer_ip="$5"
  local p2p_port="$6"

  case "${profile}" in
    nat1) profile_nat1 "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat2) profile_nat2 "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat3) profile_nat3 "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat4-regular) profile_nat4_regular "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat4-irregular) profile_nat4_irregular "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    *) die "unknown profile: ${profile}" ;;
  esac
}

nat_base() {
  local ns="$1"
  ns_exec "${ns}" sh -c 'echo 1 > /proc/sys/net/ipv4/ip_forward'

  ns_exec "${ns}" iptables -w -F
  ns_exec "${ns}" iptables -w -X
  ns_exec "${ns}" iptables -w -t nat -F
  ns_exec "${ns}" iptables -w -t nat -X
  ns_exec "${ns}" iptables -w -t mangle -F
  ns_exec "${ns}" iptables -w -t mangle -X

  # Reset recent module state used by some profiles (per-netns).
  ns_exec "${ns}" sh -c 'for n in miopunch_map_open miopunch_nat2_allowed; do f="/proc/net/xt_recent/${n}"; if [ -f "$f" ]; then echo clear >"$f" 2>/dev/null || true; : >"$f" 2>/dev/null || true; fi; done'

  ns_exec "${ns}" iptables -w -P FORWARD DROP
  ns_exec "${ns}" iptables -w -A FORWARD -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ns_exec "${ns}" iptables -w -A FORWARD -i lan0 -o wan0 -j ACCEPT
}

profile_nat1() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}"

  # Mapping open marker for the peer endpoint/port (required to avoid always-on static DNAT).
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_map_open --set

  # EIM-like mapping (port-preserving when possible).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"

  # EIF-like filtering implemented via gated static DNAT + open marker:
  # once the peer has sent an outbound packet, allow inbound from any remote to the mapped port.
  ns_exec "${ns}" iptables -w -t nat -A PREROUTING -i wan0 -p udp --dport "${p2p_port}" \
    -j DNAT --to-destination "${peer_ip}:${p2p_port}"

  ns_exec "${ns}" iptables -w -A FORWARD -i wan0 -o lan0 -p udp --dport "${p2p_port}" \
    -m recent --name miopunch_map_open --rcheck --seconds 600 --rdest -j ACCEPT
}

profile_nat2() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}"

  # Record remote IPs that the peer has sent to (address-dependent filtering).
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_map_open --set
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_nat2_allowed --set --rdest

  # EIM-like mapping (port-preserving when possible).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"

  ns_exec "${ns}" iptables -w -t nat -A PREROUTING -i wan0 -p udp --dport "${p2p_port}" \
    -j DNAT --to-destination "${peer_ip}:${p2p_port}"

  ns_exec "${ns}" iptables -w -A FORWARD -i wan0 -o lan0 -p udp --dport "${p2p_port}" \
    -m recent --name miopunch_map_open --rcheck --seconds 600 --rdest \
    -m recent --name miopunch_nat2_allowed --rcheck --seconds 600 --rsource -j ACCEPT
}

profile_nat3() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}"

  # Port-restricted filtering: rely on conntrack (APDF-like).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"
}

profile_nat4_regular() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}"

  # Symmetric-like mapping (APDM-like), with "regular" port allocation (sequential within range).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}:40000-45000"
}

profile_nat4_irregular() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}"

  # Symmetric-like mapping (APDM-like), with "irregular" port allocation (random within range).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}:40000-45000" --random-fully
}
