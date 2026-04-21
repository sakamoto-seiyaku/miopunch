#!/usr/bin/env bash
set -euo pipefail

if ! command -v rg >/dev/null 2>&1; then
  echo "error: missing dependency: rg" >&2
  exit 1
fi

pattern='\\bmiopunch\\s+(coord|peer|stun|mqtt-broker)\\b'

set +e
matches="$(rg -n "${pattern}" docs lab scripts)"
rc=$?
set -e

if [[ "${rc}" -eq 0 ]]; then
  echo "error: found lab command instructions using 'miopunch' (use 'miopunch-lab' instead):" >&2
  echo "${matches}" >&2
  exit 1
fi

if [[ "${rc}" -ne 1 ]]; then
  echo "error: rg failed (rc=${rc})" >&2
  exit "${rc}"
fi

echo "ok: no miopunch lab command instructions"
