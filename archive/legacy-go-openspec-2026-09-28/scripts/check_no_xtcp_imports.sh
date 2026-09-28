#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

pattern='github.com/miopunch/miopunch/xtcp'

if command -v rg >/dev/null 2>&1; then
  if rg -n "${pattern}" -g'*.go' .; then
    echo "error: disallowed import prefix detected: ${pattern}" >&2
    exit 1
  fi
else
  if grep -RIn --include='*.go' "${pattern}" .; then
    echo "error: disallowed import prefix detected: ${pattern}" >&2
    exit 1
  fi
fi

echo "ok: no xtcp imports"

