#!/usr/bin/env bash
set -euo pipefail

mnt03_pass=0
mnt03_fail=0
mnt03_pending=0

mnt03_aggregate_init() {
  local dir="$1"
  mkdir -p "${dir}"
  : >"${dir}/cases.txt"
  mnt03_pass=0
  mnt03_fail=0
  mnt03_pending=0
}

mnt03_run_case() {
  local gate_label="$1"
  shift

  echo "== ${gate_label}: $* =="
  set +e
  local out
  out="$("${runner}" "$@" 2>&1)"
  local rc=$?
  set -e
  printf '%s\n' "${out}"

  local line
  line="$(printf '%s\n' "${out}" | grep '^case=' | tail -n 1 || true)"
  if [[ -n "${line}" ]]; then
    printf '%s\n' "${line}" >>"${aggregate_dir}/cases.txt"
    if printf '%s\n' "${line}" | grep -q 'status=pending'; then
      mnt03_pending=$((mnt03_pending + 1))
    fi
  fi

  if [[ "${rc}" -eq 0 ]]; then
    mnt03_pass=$((mnt03_pass + 1))
  else
    mnt03_fail=$((mnt03_fail + 1))
  fi
}

mnt03_write_summary() {
  local summary_path="$1" gate="$2" extra_json="${3:-}"
  {
    printf '{\n'
    printf '  "gate": "%s",\n' "${gate}"
    printf '  "pass": %d,\n' "${mnt03_pass}"
    printf '  "fail": %d,\n' "${mnt03_fail}"
    printf '  "pending": %d,\n' "${mnt03_pending}"
    if [[ -n "${extra_json}" ]]; then
      printf '%s\n' "${extra_json}"
    fi
    printf '  "cases_file": "%s"\n' "${aggregate_dir}/cases.txt"
    printf '}\n'
  } >"${summary_path}"
}
