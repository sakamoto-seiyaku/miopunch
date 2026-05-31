#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd -- "${script_dir}/.." && pwd)"
apk="${1:-${app_dir}/build/outputs/miopunch-control-lite-debug.apk}"

die() {
  echo "error: $*" >&2
  exit 1
}

to_win_path() {
  if command -v wslpath >/dev/null 2>&1; then
    wslpath -w "$1"
    return 0
  fi
  if [[ "$1" =~ ^/mnt/([a-zA-Z])/(.*)$ ]]; then
    local drive="${BASH_REMATCH[1]}"
    local rest="${BASH_REMATCH[2]//\//\\}"
    printf '%s:\\%s\n' "${drive^^}" "${rest}"
    return 0
  fi
  printf '%s\n' "$1"
}

run_adb() {
  if [[ -n "${MIOPUNCH_ADB:-}" ]]; then
    "${MIOPUNCH_ADB}" "$@"
    return
  fi

  local win_adb="/mnt/c/Android/SDK/platform-tools/adb.exe"
  local win_cmd="/mnt/c/Windows/System32/cmd.exe"
  if [[ -x /init && -x "${win_adb}" && -x "${win_cmd}" ]]; then
    local converted=()
    local arg
    for arg in "$@"; do
      if [[ "${arg}" == /* ]]; then
        converted+=("$(to_win_path "${arg}")")
      else
        converted+=("${arg}")
      fi
    done
    (
      cd /mnt/c/Windows/System32
      /init "${win_cmd}" /C "$(to_win_path "${win_adb}")" "${converted[@]}"
    )
    return
  fi

  if command -v adb >/dev/null 2>&1; then
    adb "$@"
    return
  fi

  die "adb not found; set MIOPUNCH_ADB"
}

[[ -f "${apk}" ]] || die "APK not found: ${apk} (run build-debug-apk.sh first)"

run_adb devices -l
run_adb install -r "${apk}"
echo "ok: installed ${apk}"
