#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"
}

ns_exec() {
  local ns="$1"
  shift
  ip netns exec "${ns}" "$@"
}

