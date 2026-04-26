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

profiles_apply_extra_port() {
  local ns="$1"
  local profile="$2"
  local lan_cidr="$3"
  local wan_ip="$4"
  local peer_ip="$5"
  local p2p_port="$6"

  case "${profile}" in
    nat1) profile_nat1_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat2) profile_nat2_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat3) profile_nat3_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    nat4-regular) ;;
    nat4-irregular) profile_nat4_irregular_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}" ;;
    *) die "unknown profile: ${profile}" ;;
  esac
}

nat_base() {
  local ns="$1" lan_cidr="$2" wan_ip="$3"
  ns_exec "${ns}" sh -c 'echo 1 > /proc/sys/net/ipv4/ip_forward'

  # Ensure clean slate across case switches. Note that the lab uses iptables-nft,
  # so flushing iptables tables is enough (no separate nft state is expected).
  ns_exec "${ns}" iptables -w -F
  ns_exec "${ns}" iptables -w -X
  ns_exec "${ns}" iptables -w -t nat -F
  ns_exec "${ns}" iptables -w -t nat -X
  ns_exec "${ns}" iptables -w -t mangle -F
  ns_exec "${ns}" iptables -w -t mangle -X
  ns_exec "${ns}" iptables -w -t raw -F
  ns_exec "${ns}" iptables -w -t raw -X

  # Reset recent module state used by some profiles (per-netns).
  ns_exec "${ns}" sh -c 'for n in miopunch_map_open miopunch_nat2_allowed; do f="/proc/net/xt_recent/${n}"; if [ -f "$f" ]; then echo clear >"$f" 2>/dev/null || true; : >"$f" 2>/dev/null || true; fi; done'

  # XTCP detect mode uses "low TTL" UDP packets that are intended to die somewhere
  # in the public Internet after creating local NAT state. In this lab, the WAN is
  # a single L2 segment, so such packets would otherwise reach the opposite NAT and
  # create conntrack entries that can break subsequent hole punching (e.g. NAT3×NAT3).
  #
  # Drop low-TTL UDP packets as early as possible (raw PREROUTING is before conntrack).
  ns_exec "${ns}" iptables -w -t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP

  ns_exec "${ns}" iptables -w -P FORWARD DROP
  ns_exec "${ns}" iptables -w -A FORWARD -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  ns_exec "${ns}" iptables -w -A FORWARD -i lan0 -o wan0 -j ACCEPT

  # Best-effort SNAT for TCP flows (control plane baseline).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p tcp \
    -j SNAT --to-source "${wan_ip}"
}

profile_nat1() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}" "${lan_cidr}" "${wan_ip}"
  profile_nat1_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}"
  # Best-effort SNAT for other UDP flows.
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"
}

profile_nat1_port_rules() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  # Door-2 TCP convention: base=P, listen/punch=L=P+100.
  #
  # Keep the external TCP ports stable for P and L so the coordinator can
  # produce explainable tcp_candidate_addrs and the lab remains deterministic.
  local tcp_base_port="${p2p_port}"
  local tcp_listen_port=$((p2p_port + 100))
  ns_exec "${ns}" iptables -w -t nat -I POSTROUTING 1 -o wan0 -s "${lan_cidr}" -p tcp --sport "${tcp_listen_port}" \
    -j SNAT --to-source "${wan_ip}:${tcp_listen_port}"
  ns_exec "${ns}" iptables -w -t nat -I POSTROUTING 1 -o wan0 -s "${lan_cidr}" -p tcp --sport "${tcp_base_port}" \
    -j SNAT --to-source "${wan_ip}:${tcp_base_port}"

  # Mapping open marker for the peer endpoint/port (required to avoid always-on static DNAT).
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_map_open --set

  # EIM-like mapping: pin the P2P source port to the same external port.
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp --sport "${p2p_port}" \
    -j SNAT --to-source "${wan_ip}:${p2p_port}"

  # EIF-like filtering implemented via gated static DNAT + open marker:
  # once the peer has sent an outbound packet, allow inbound from any remote to the mapped port.
  ns_exec "${ns}" iptables -w -t nat -A PREROUTING -i wan0 -p udp --dport "${p2p_port}" \
    -j DNAT --to-destination "${peer_ip}:${p2p_port}"

  ns_exec "${ns}" iptables -w -A FORWARD -i wan0 -o lan0 -p udp --dport "${p2p_port}" \
    -m recent --name miopunch_map_open --rcheck --seconds 600 --rdest -j ACCEPT
}

profile_nat2() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}" "${lan_cidr}" "${wan_ip}"
  profile_nat2_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}"
  # Best-effort SNAT for other UDP flows.
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"
}

profile_nat2_port_rules() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  # Record remote IPs that the peer has sent to (address-dependent filtering).
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_map_open --set
  ns_exec "${ns}" iptables -w -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "${p2p_port}" \
    -m recent --name miopunch_nat2_allowed --set --rdest

  # EIM-like mapping: pin the P2P source port to the same external port.
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp --sport "${p2p_port}" \
    -j SNAT --to-source "${wan_ip}:${p2p_port}"

  ns_exec "${ns}" iptables -w -t nat -A PREROUTING -i wan0 -p udp --dport "${p2p_port}" \
    -j DNAT --to-destination "${peer_ip}:${p2p_port}"

  ns_exec "${ns}" iptables -w -A FORWARD -i wan0 -o lan0 -p udp --dport "${p2p_port}" \
    -m recent --name miopunch_map_open --rcheck --seconds 600 --rdest \
    -m recent --name miopunch_nat2_allowed --rcheck --seconds 600 --rsource -j ACCEPT
}

profile_nat3() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}" "${lan_cidr}" "${wan_ip}"
  profile_nat3_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}"
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}"
}

profile_nat3_port_rules() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  # Port-restricted filtering: rely on conntrack (APDF-like).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp --sport "${p2p_port}" \
    -j SNAT --to-source "${wan_ip}:${p2p_port}"
}

profile_nat4_regular() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}" "${lan_cidr}" "${wan_ip}"

  # Symmetric-like mapping (APDM-like), with "regular" port allocation (sequential within range).
  # For the testbed we intentionally allocate from different sub-ranges based on the remote IP
  # to make the mapping address-dependent (and thus APDM-like under our simplified classifier).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp -d "${COORD_IP}" \
    -j SNAT --to-source "${wan_ip}:40000-42499"
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp -d "${PROBE_IP}" \
    -j SNAT --to-source "${wan_ip}:42500-45000"
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}:40000-45000"
}

profile_nat4_irregular() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  nat_base "${ns}" "${lan_cidr}" "${wan_ip}"
  profile_nat4_irregular_port_rules "${ns}" "${lan_cidr}" "${wan_ip}" "${peer_ip}" "${p2p_port}"

  # Symmetric-like mapping (APDM-like), with "irregular" port allocation (random within range).
  ns_exec "${ns}" iptables -w -t nat -A POSTROUTING -o wan0 -s "${lan_cidr}" -p udp \
    -j SNAT --to-source "${wan_ip}:40000-45000" --random-fully
}

profile_nat4_irregular_port_rules() {
  local ns="$1" lan_cidr="$2" wan_ip="$3" peer_ip="$4" p2p_port="$5"
  # Door-2 TCP: simulate a "hard" NAT for TCP STUN sampling while keeping the
  # punching port reachable via one of the derived (+100) candidate ports.
  local tcp_base_port="${p2p_port}"
  local tcp_listen_port=$((p2p_port + 100))
  ns_exec "${ns}" iptables -w -t nat -I POSTROUTING 1 -o wan0 -s "${lan_cidr}" -p tcp --sport "${tcp_base_port}" -d "${STUN_IP}" --dport 3478 \
    -j SNAT --to-source "${wan_ip}:40000"
  ns_exec "${ns}" iptables -w -t nat -I POSTROUTING 1 -o wan0 -s "${lan_cidr}" -p tcp --sport "${tcp_base_port}" -d "${STUN_IP}" --dport 3479 \
    -j SNAT --to-source "${wan_ip}:45000"
  ns_exec "${ns}" iptables -w -t nat -I POSTROUTING 1 -o wan0 -s "${lan_cidr}" -p tcp --sport "${tcp_listen_port}" \
    -j SNAT --to-source "${wan_ip}:45100"
  ns_exec "${ns}" iptables -w -t nat -A PREROUTING -i wan0 -p tcp --dport 45100 \
    -j DNAT --to-destination "${peer_ip}:${tcp_listen_port}"
  ns_exec "${ns}" iptables -w -A FORWARD -i wan0 -o lan0 -p tcp -d "${peer_ip}" --dport "${tcp_listen_port}" \
    -j ACCEPT
}
