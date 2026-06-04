#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  echo "error: must run inside the miopunch git repo" >&2
  exit 1
fi
cd "${repo_root}"

export PATH="/usr/local/go/bin:${PATH}"

echo "== go test ./... =="
go test ./...

echo "== go vet ./... =="
go vet ./...

echo "== scripts/check_no_xtcp_imports.sh =="
bash scripts/check_no_xtcp_imports.sh

echo "== lab/host/labctl nat-profile-selftest =="
./lab/host/labctl nat-profile-selftest

if [[ "${LAB_DOWN_AFTER:-0}" == "1" ]]; then
  echo "== lab/host/labctl down =="
  ./lab/host/labctl down
fi

echo "ok: all dev test gates passed"
