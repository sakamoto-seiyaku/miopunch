#!/usr/bin/env bash
set -euo pipefail

mnt03_profile_nodes() {
  local profile="$1"
  case "${profile}" in
    2node-substrate)
      printf '%s\n' n01 n03
      ;;
    3node-bootstrap)
      printf '%s\n' n01 n03 n04
      ;;
    4node-reachability)
      printf '%s\n' n01 n03 n04 n05
      ;;
    6node-bootstrap-more)
      printf '%s\n' n01 n03 n04 n05 n06 n07
      ;;
    12node-full)
      printf '%s\n' n01 n02 n03 n04 n05 n06 n07 n08 n09 n10 n11 n12
      ;;
    *)
      die "unknown MNT-03 profile: ${profile}"
      ;;
  esac
}

mnt03_node_role() {
  local node="$1"
  case "${node}" in
    n01) printf 'primary_admin' ;;
    n02) printf 'backup_admin' ;;
    n12) printf 'actor_lifecycle' ;;
    *) printf 'member' ;;
  esac
}

mnt03_node_profile() {
  local node="$1"
  case "${node}" in
    n01) printf 'nat1_dualstack_stable' ;;
    n02) printf 'nat2_ipv4_stable' ;;
    n03) printf 'nat1_dualstack_easy' ;;
    n04) printf 'nat2_ipv4_easy' ;;
    n05) printf 'nat3_portmap_ipv4' ;;
    n06) printf 'nat3_dualstack_fallback' ;;
    n07|n08) printf 'nat4_regular_ipv4' ;;
    n09) printf 'nat4_regular_lossy' ;;
    n10) printf 'nat4_irregular_ipv4' ;;
    n11) printf 'nat4_irregular_unknown' ;;
    n12) printf 'nat2_or_nat3_lifecycle' ;;
    *) die "unknown MNT-03 node: ${node}" ;;
  esac
}

mnt03_write_profile_manifest() {
  local run_dir="$1" profile="$2"
  [[ -n "${run_dir}" && -n "${profile}" ]] || die "mnt03_write_profile_manifest: missing args"

  mkdir -p "${run_dir}/network"

  local node first=1
  {
    printf '{\n'
    printf '  "format": "miopunch.mnt03.profile.v0",\n'
    printf '  "profile": "%s",\n' "${profile}"
    printf '  "nodes": [\n'
    while read -r node; do
      [[ -n "${node}" ]] || continue
      if [[ "${first}" -eq 0 ]]; then
        printf ',\n'
      fi
      first=0
      printf '    {"id": "%s", "role": "%s", "network_profile": "%s"}' \
        "${node}" "$(mnt03_node_role "${node}")" "$(mnt03_node_profile "${node}")"
    done < <(mnt03_profile_nodes "${profile}")
    printf '\n  ]\n'
    printf '}\n'
  } >"${run_dir}/network/profile.json"
}
