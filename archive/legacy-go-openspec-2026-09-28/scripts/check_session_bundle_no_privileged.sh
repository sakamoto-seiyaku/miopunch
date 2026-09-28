#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

forbidden='install-system-daemon|uninstall-system-daemon|build_windows_installer|build_deb'

if grep -E "${forbidden}" "${repo_root}/scripts/release/build_bundles.sh" >/dev/null; then
  echo "session bundle builder must not invoke privileged install/package paths" >&2
  exit 1
fi

binary_job="$(
  awk '
    /^  session-bundles:/ { in_job=1 }
    /^  assemble:/ { in_job=0 }
    in_job { print }
  ' "${repo_root}/.github/workflows/build-artifacts.yml"
)"

if printf '%s\n' "${binary_job}" | grep -E "${forbidden}" >/dev/null; then
  echo "binary/session artifact job must not invoke privileged install/package paths" >&2
  exit 1
fi

echo "session bundle paths do not invoke privileged service/package commands"
