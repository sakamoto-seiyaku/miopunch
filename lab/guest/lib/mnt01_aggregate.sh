#!/usr/bin/env bash
set -euo pipefail

mnt01_pass=0
mnt01_fail=0
mnt01_required_pass=0
mnt01_preferred_success=0
mnt01_allowed_diag_fail=0
mnt01_required_fail=0
mnt01_unexpected_fail=0

mnt01_aggregate_init() {
  local aggregate_dir="$1"

  mkdir -p "${aggregate_dir}"
  : >"${aggregate_dir}/cases.txt"
  mnt01_pass=0
  mnt01_fail=0
  mnt01_required_pass=0
  mnt01_preferred_success=0
  mnt01_allowed_diag_fail=0
  mnt01_required_fail=0
  mnt01_unexpected_fail=0
}

mnt01_case_field() {
  local line="$1" key="$2" default_value="$3"
  local field

  for field in ${line}; do
    case "${field}" in
      ${key}=*)
        printf '%s' "${field#*=}"
        return 0
        ;;
    esac
  done
  printf '%s' "${default_value}"
}

mnt01_aggregate_line() {
  local line="$1"

  mnt01_required_pass=$((mnt01_required_pass + $(mnt01_case_field "${line}" "required_pass" 0)))
  mnt01_preferred_success=$((mnt01_preferred_success + $(mnt01_case_field "${line}" "preferred_success" 0)))
  mnt01_allowed_diag_fail=$((mnt01_allowed_diag_fail + $(mnt01_case_field "${line}" "allowed_diag_fail" 0)))
  mnt01_required_fail=$((mnt01_required_fail + $(mnt01_case_field "${line}" "required_fail" 0)))
  mnt01_unexpected_fail=$((mnt01_unexpected_fail + $(mnt01_case_field "${line}" "unexpected_fail" 0)))
}

mnt01_run_case() {
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
    mnt01_aggregate_line "${line}"
  fi

  if [[ "${rc}" -eq 0 ]]; then
    mnt01_pass=$((mnt01_pass + 1))
  else
    mnt01_fail=$((mnt01_fail + 1))
  fi
}

mnt01_write_summary() {
  local summary_path="$1" gate="$2" extra_json="${3:-}"

  {
    printf '{\n'
    printf '  "gate": "%s",\n' "${gate}"
    printf '  "pass": %d,\n' "${mnt01_pass}"
    printf '  "fail": %d,\n' "${mnt01_fail}"
    printf '  "required_pass": %d,\n' "${mnt01_required_pass}"
    printf '  "preferred_success": %d,\n' "${mnt01_preferred_success}"
    printf '  "allowed_diag_fail": %d,\n' "${mnt01_allowed_diag_fail}"
    printf '  "required_fail": %d,\n' "${mnt01_required_fail}"
    printf '  "unexpected_fail": %d,\n' "${mnt01_unexpected_fail}"
    if [[ -n "${extra_json}" ]]; then
      printf '%s\n' "${extra_json}"
    fi
    printf '  "cases_file": "%s"\n' "${aggregate_dir}/cases.txt"
    printf '}\n'
  } >"${summary_path}"
}
