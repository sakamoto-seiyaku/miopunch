#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "== syntax =="
find lab -type f \( -name '*.sh' -o -path '*/bin/*' -o -name 'labctl' \) -print0 | xargs -0 -n 1 bash -n

echo "== unit =="
./lab/guest/tests/unit.sh

echo "== openspec =="
openspec validate add-nat-lab-testbed --strict --no-interactive

echo "ok: lab checks passed"

