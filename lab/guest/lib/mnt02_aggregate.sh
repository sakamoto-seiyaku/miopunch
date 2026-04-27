#!/usr/bin/env bash
set -euo pipefail

mnt02_pass=0
mnt02_fail=0

mnt02_aggregate_init() {
  local dir="$1"
  mkdir -p "${dir}"
  : >"${dir}/cases.txt"
  mnt02_pass=0
  mnt02_fail=0
}

mnt02_run_case() {
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
  fi

  if [[ "${rc}" -eq 0 ]]; then
    mnt02_pass=$((mnt02_pass + 1))
  else
    mnt02_fail=$((mnt02_fail + 1))
  fi
}

mnt02_write_summary() {
  local summary_path="$1" gate="$2" extra_json="${3:-}"
  {
    printf '{\n'
    printf '  "gate": "%s",\n' "${gate}"
    printf '  "pass": %d,\n' "${mnt02_pass}"
    printf '  "fail": %d,\n' "${mnt02_fail}"
    if [[ -n "${extra_json}" ]]; then
      printf '%s\n' "${extra_json}"
    fi
    printf '  "cases_file": "%s"\n' "${aggregate_dir}/cases.txt"
    printf '}\n'
  } >"${summary_path}"
}

