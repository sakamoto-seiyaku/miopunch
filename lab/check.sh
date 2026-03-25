#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "== syntax =="
while IFS= read -r -d '' file; do
  shebang="$(head -n 1 "${file}" || true)"
  case "${shebang}" in
    *python*)
      python3 -m py_compile "${file}"
      ;;
    *sh*|*bash*)
      bash -n "${file}"
      ;;
  esac
done < <(
  find lab \
    \( -path 'lab/_*' -o -path '*/__pycache__/*' \) -prune -o \
    -type f \( -name '*.sh' -o -path '*/bin/*' -o -name 'labctl' \) -print0
)

echo "== unit =="
./lab/guest/tests/unit.sh

echo "== openspec =="
openspec validate --all --strict --no-interactive

echo "ok: lab checks passed"
