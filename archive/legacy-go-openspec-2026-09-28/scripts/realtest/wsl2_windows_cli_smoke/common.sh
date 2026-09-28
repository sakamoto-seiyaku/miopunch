#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"
}

smoke_root="${MIO_REALTEST_ROOT:-/tmp/miopunch-wsl2-windows-cli-smoke}"
windows_root="${MIO_WINDOWS_ROOT:-${smoke_root}/windows}"
wsl_root="${MIO_WSL_ROOT:-${smoke_root}/wsl}"
artifacts_root="${MIO_ARTIFACTS:-${smoke_root}/artifacts}"

mkdir -p "${windows_root}" "${wsl_root}" "${artifacts_root}"

report_path() {
  local side="$1"
  local name="$2"
  printf '%s/%s/%s.md' "${artifacts_root}" "${side}" "${name}"
}

stdout_path() {
  local side="$1"
  local name="$2"
  printf '%s/%s/%s.stdout' "${artifacts_root}" "${side}" "${name}"
}

stderr_path() {
  local side="$1"
  local name="$2"
  printf '%s/%s/%s.stderr' "${artifacts_root}" "${side}" "${name}"
}
