#!/usr/bin/env bash
set -euo pipefail

capture_wan_lines() {
  local filter="$1"
  ns_exec "${NS_WAN}" timeout 4 tcpdump -ni br0 -nn -tt ${filter}
}

tcpdump_parse_src_port() {
  local line="$1"
  local src="${line#IP }"
  src="${src%% >*}"
  src="${src%:}"
  echo "${src##*.}"
}

tcpdump_parse_dst() {
  local line="$1"
  local rest="${line#*> }"
  rest="${rest%%:*}"
  rest="${rest%:}"
  echo "${rest}"
}

mapping_classify_from_ports() {
  local p1="$1" p2="$2"
  if [[ "${p1}" == "${p2}" ]]; then
    echo "EIM"
    return 0
  fi
  echo "APDM"
}

filtering_classify_from_flags() {
  local got_other_ip="$1"
  local got_same_ip_diff_port="$2"

  if [[ "${got_other_ip}" -eq 1 ]]; then
    echo "EIF"
    return 0
  fi
  if [[ "${got_same_ip_diff_port}" -eq 1 ]]; then
    echo "ADF"
    return 0
  fi
  echo "APDF"
}

udp_send() {
  local ns="$1" dst_ip="$2" dst_port="$3" src_port="$4" payload="${5:-x}"
  printf '%s' "${payload}" | ns_exec "${ns}" socat -u - "udp-datagram:${dst_ip}:${dst_port},bind=:${src_port},reuseaddr"
}

peer_expect_udp() {
  local peer_ns="$1" dst_port="$2"
  local cap="/tmp/mlab-cap-${peer_ns}-${dst_port}-$$.log"
  rm -f "${cap}"

  ( ns_exec "${peer_ns}" timeout 2 tcpdump -ni eth0 -c 1 -nn "udp and dst port ${dst_port}" >"${cap}" 2>&1 ) &
  local tpid=$!
  sleep 0.2
  wait "${tpid}"
  local rc=$?
  if [[ ${rc} -eq 0 ]]; then
    return 0
  fi
  return 1
}

mapping_observe() {
  local peer_ns="$1" nat_wan_ip="$2" peer_src_port="$3"
  local dst1_ip="$4" dst1_port="$5" dst2_ip="$6" dst2_port="$7"

  local cap="/tmp/mlab-map-${peer_ns}-$$.log"
  rm -f "${cap}"

  ( ns_exec "${NS_WAN}" timeout 4 tcpdump -ni br0 -c 2 -nn -tt \
      "udp and src host ${nat_wan_ip} and ((dst host ${dst1_ip} and dst port ${dst1_port}) or (dst host ${dst2_ip} and dst port ${dst2_port}))" \
      >"${cap}" 2>&1 ) &
  local tpid=$!
  sleep 0.2

  udp_send "${peer_ns}" "${dst1_ip}" "${dst1_port}" "${peer_src_port}" "m1"
  udp_send "${peer_ns}" "${dst2_ip}" "${dst2_port}" "${peer_src_port}" "m2"

  wait "${tpid}" || true

  local p1 p2
  p1="$(grep -F " > ${dst1_ip}.${dst1_port}:" "${cap}" | head -n 1 || true)"
  p2="$(grep -F " > ${dst2_ip}.${dst2_port}:" "${cap}" | head -n 1 || true)"
  [[ -n "${p1}" && -n "${p2}" ]] || die "mapping observe failed; see ${cap}"

  local sp1 sp2
  sp1="$(tcpdump_parse_src_port "${p1}")"
  sp2="$(tcpdump_parse_src_port "${p2}")"
  echo "${sp1} ${sp2}"
}

mapping_classify() {
  local peer_ns="$1" nat_wan_ip="$2" peer_src_port="$3"
  local tries="${4:-5}"

  local i p1 p2 m1 m2
  for ((i = 0; i < tries; i++)); do
    p1=$((41000 + i * 2))
    p2=$((41001 + i * 2))
    read -r m1 m2 < <(mapping_observe "${peer_ns}" "${nat_wan_ip}" "${peer_src_port}" "${COORD_IP}" "${p1}" "${PROBE_IP}" "${p2}")
    if [[ "$(mapping_classify_from_ports "${m1}" "${m2}")" == "APDM" ]]; then
      echo "APDM"
      return 0
    fi
  done

  echo "EIM"
}

mapped_port_observe() {
  local peer_ns="$1" nat_wan_ip="$2" peer_src_port="$3" dst_ip="$4" dst_port="$5"

  local cap="/tmp/mlab-mapped-${peer_ns}-$$.log"
  rm -f "${cap}"

  ( ns_exec "${NS_WAN}" timeout 4 tcpdump -ni br0 -c 1 -nn -tt \
      "udp and src host ${nat_wan_ip} and dst host ${dst_ip} and dst port ${dst_port}" \
      >"${cap}" 2>&1 ) &
  local tpid=$!
  sleep 0.2

  udp_send "${peer_ns}" "${dst_ip}" "${dst_port}" "${peer_src_port}" "mp"

  wait "${tpid}" || true
  local line
  line="$(grep -F " > ${dst_ip}.${dst_port}:" "${cap}" | head -n 1 || true)"
  if [[ -z "${line}" ]]; then
    echo "error: mapped port observe failed; see ${cap}" >&2
    return 1
  fi
  tcpdump_parse_src_port "${line}"
}

expected_mapping() {
  local profile="$1"
  case "${profile}" in
    nat4-regular|nat4-irregular) echo "APDM" ;;
    nat1|nat2|nat3) echo "EIM" ;;
    *) echo "unknown" ;;
  esac
}

expected_filtering() {
  local profile="$1"
  case "${profile}" in
    nat1) echo "EIF" ;;
    nat2) echo "ADF" ;;
    nat3|nat4-regular|nat4-irregular) echo "APDF" ;;
    *) echo "unknown" ;;
  esac
}

nat_label_from() {
  local mapping="$1" filtering="$2"
  case "${mapping}+${filtering}" in
    EIM+EIF) echo "NAT1" ;;
    EIM+ADF) echo "NAT2" ;;
    EIM+APDF) echo "NAT3" ;;
    APDM+APDF) echo "NAT4" ;;
    *) echo "NAT-OTHER" ;;
  esac
}

validate_one_side() {
  local side="$1" peer_ns="$2" nat_wan_ip="$3" peer_p2p_port="$4" profile="$5"

  need_cmd tcpdump
  need_cmd socat
  need_cmd timeout

  local exp_map exp_filt
  exp_map="$(expected_mapping "${profile}")"
  exp_filt="$(expected_filtering "${profile}")"

  local ok=1

  # Mapping: observe src port against two different WAN endpoints.
  local obs_map="unknown"
  obs_map="$(mapping_classify "${peer_ns}" "${nat_wan_ip}" "${peer_p2p_port}" "${MLAB_MAP_TRIES:-5}")" || ok=0

  # Filtering: establish a flow to COORD_IP:42000 and observe mapped port.
  local mapped_port=""
  if ! mapped_port="$(mapped_port_observe "${peer_ns}" "${nat_wan_ip}" "${peer_p2p_port}" "${COORD_IP}" 42000)"; then
    ok=0
  fi

  # 1) reply from same ip:port should arrive
  local got_reply got_same_ip_diff_port got_other_ip
  got_reply=0
  got_same_ip_diff_port=0
  got_other_ip=0

  local obs_filt="unknown"
  if [[ -n "${mapped_port}" ]]; then
    ( ns_exec "${peer_ns}" timeout 2 tcpdump -ni eth0 -c 1 -nn "udp and dst port ${peer_p2p_port}" >/dev/null 2>&1 ) &
    local cap_pid=$!
    sleep 0.2
    udp_send "${NS_COORD}" "${nat_wan_ip}" "${mapped_port}" 42000 "r"
    if wait "${cap_pid}"; then got_reply=1; else got_reply=0; fi

    ( ns_exec "${peer_ns}" timeout 2 tcpdump -ni eth0 -c 1 -nn "udp and dst port ${peer_p2p_port}" >/dev/null 2>&1 ) &
    cap_pid=$!
    sleep 0.2
    udp_send "${NS_COORD}" "${nat_wan_ip}" "${mapped_port}" 42001 "p"
    if wait "${cap_pid}"; then got_same_ip_diff_port=1; else got_same_ip_diff_port=0; fi

    ( ns_exec "${peer_ns}" timeout 2 tcpdump -ni eth0 -c 1 -nn "udp and dst port ${peer_p2p_port}" >/dev/null 2>&1 ) &
    cap_pid=$!
    sleep 0.2
    udp_send "${NS_STUN}" "${nat_wan_ip}" "${mapped_port}" 42002 "o"
    if wait "${cap_pid}"; then got_other_ip=1; else got_other_ip=0; fi

    obs_filt="$(filtering_classify_from_flags "${got_other_ip}" "${got_same_ip_diff_port}")"
    if [[ "${got_reply}" -ne 1 ]]; then
      ok=0
    fi
  fi

  local obs_nat exp_nat
  obs_nat="$(nat_label_from "${obs_map}" "${obs_filt}")"
  exp_nat="$(nat_label_from "${exp_map}" "${exp_filt}")"
  if [[ "${obs_map}" != "${exp_map}" ]]; then ok=0; fi
  if [[ "${obs_filt}" != "${exp_filt}" ]]; then ok=0; fi
  if [[ "${obs_nat}" != "${exp_nat}" ]]; then ok=0; fi

  echo "${side}: stage=mapping obs=${obs_map} exp=${exp_map}"
  echo "${side}: stage=filtering obs=${obs_filt} exp=${exp_filt}"
  echo "${side}: stage=labels obs_nat=${obs_nat} exp_nat=${exp_nat} profile=${profile}"
  if [[ "${got_reply}" -ne 1 ]]; then
    echo "${side}: error: expected reply packet not observed"
  fi

  [[ "${ok}" -eq 1 ]]
}

validate_case() {
  [[ -f "${active_case_file}" ]] || die "no active case; run: mlab case activate <id>"
  local case_id
  case_id="$(cat "${active_case_file}")"
  local case_file="${guest_root}/cases/${case_id}.sh"
  [[ -f "${case_file}" ]] || die "active case file missing: ${case_file}"

  # shellcheck source=/dev/null
  source "${case_file}"
  A_P2P_PORT="${A_P2P_PORT:-5000}"
  B_P2P_PORT="${B_P2P_PORT:-5000}"

  local ok=1
  validate_one_side "A" "${NS_PEER_A}" "${WAN_A_IP}" "${A_P2P_PORT}" "${A_PROFILE}" || ok=0
  validate_one_side "B" "${NS_PEER_B}" "${WAN_B_IP}" "${B_P2P_PORT}" "${B_PROFILE}" || ok=0

  [[ "${ok}" -eq 1 ]] || die "validation failed"
  echo "ok: validation passed"
}

validate_side() {
  local which="${1:-}"
  [[ -f "${active_case_file}" ]] || die "no active case; run: mlab case activate <id>"
  local case_id
  case_id="$(cat "${active_case_file}")"
  local case_file="${guest_root}/cases/${case_id}.sh"
  [[ -f "${case_file}" ]] || die "active case file missing: ${case_file}"

  # shellcheck source=/dev/null
  source "${case_file}"
  A_P2P_PORT="${A_P2P_PORT:-5000}"
  B_P2P_PORT="${B_P2P_PORT:-5000}"

  case "${which}" in
    a|A) validate_one_side "A" "${NS_PEER_A}" "${WAN_A_IP}" "${A_P2P_PORT}" "${A_PROFILE}" ;;
    b|B) validate_one_side "B" "${NS_PEER_B}" "${WAN_B_IP}" "${B_P2P_PORT}" "${B_PROFILE}" ;;
    *) die "validate side: need a|b" ;;
  esac
}
