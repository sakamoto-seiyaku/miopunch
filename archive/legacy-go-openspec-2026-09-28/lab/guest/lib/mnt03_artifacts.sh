#!/usr/bin/env bash
set -euo pipefail

mnt03_artifacts_init() {
  local run_dir="$1" case_id="$2" stage="$3" profile="$4"
  [[ -n "${run_dir}" && -n "${case_id}" && -n "${stage}" ]] || \
    die "mnt03_artifacts_init: missing args"

  mkdir -p \
    "${run_dir}/commands" \
    "${run_dir}/daemons" \
    "${run_dir}/journals" \
    "${run_dir}/topology" \
    "${run_dir}/tasks" \
    "${run_dir}/broker" \
    "${run_dir}/network" \
    "${run_dir}/cleanup"

  {
    printf 'case_id=%s\n' "${case_id}"
    printf 'stage=%s\n' "${stage}"
    printf 'profile=%s\n' "${profile}"
  } >"${run_dir}/case.env"
}

mnt03_run_artifact_cmd() {
  local run_dir="$1" label="$2"
  shift 2
  [[ -n "${run_dir}" && -n "${label}" && "$#" -gt 0 ]] || \
    die "mnt03_run_artifact_cmd: missing args"

  local out_dir rc
  out_dir="${run_dir}/commands/${label}"
  mkdir -p "${out_dir}"

  set +e
  "$@" >"${out_dir}/stdout" 2>"${out_dir}/stderr"
  rc=$?
  set -e

  printf '%s\n' "${rc}" >"${out_dir}/rc"
  return "${rc}"
}

mnt03_collect_topology_snapshot() {
  local run_dir="$1" node="$2" namespace="$3" socket="$4" miopunch_bin="$5"
  [[ -n "${run_dir}" && -n "${node}" && -n "${namespace}" && -n "${socket}" && -n "${miopunch_bin}" ]] || \
    die "mnt03_collect_topology_snapshot: missing args"

  mkdir -p "${run_dir}/topology"
  set +e
  ip netns exec "${namespace}" "${miopunch_bin}" \
    --localapi "unix:${socket}" \
    --format json \
    topology \
    >"${run_dir}/topology/${node}.json" \
    2>"${run_dir}/topology/${node}.stderr"
  local rc=$?
  set -e
  printf '%s\n' "${rc}" >"${run_dir}/topology/${node}.rc"
  return "${rc}"
}

mnt03_collect_network_manifest() {
  local run_dir="$1"
  [[ -n "${run_dir}" ]] || die "mnt03_collect_network_manifest: missing run_dir"

  mkdir -p "${run_dir}/network"
  ip -j netns list >"${run_dir}/network/netns.json" 2>/dev/null || true
  ip -j link show >"${run_dir}/network/host.links.json" 2>/dev/null || true
}

mnt03_write_summary() {
  local summary_path="$1" gate="$2" case_id="$3" stage="$4" status="$5" pass="$6" fail="$7" reason="$8"
  [[ -n "${summary_path}" && -n "${gate}" && -n "${case_id}" && -n "${stage}" ]] || \
    die "mnt03_write_summary: missing args"

  cat >"${summary_path}" <<EOF
{
  "gate": "${gate}",
  "case_id": "${case_id}",
  "stage": "${stage}",
  "status": "${status}",
  "pass": ${pass},
  "fail": ${fail},
  "reason": "${reason}",
  "artifacts": "$(dirname -- "${summary_path}")"
}
EOF
}
