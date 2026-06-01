#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${MIOPUNCH_RELEASE_OUT:-${repo_root}/dist/release}"
version="${MIOPUNCH_VERSION:-}"

if [[ -z "${version}" ]]; then
  version="$(git -C "${repo_root}" describe --tags --exact-match 2>/dev/null || true)"
fi
if [[ -z "${version}" ]]; then
  version="0.0.0-git$(git -C "${repo_root}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
fi

mkdir -p "${out_dir}"

bash "${repo_root}/android/control-lite/scripts/build-debug-apk.sh"

src="${repo_root}/android/control-lite/build/outputs/miopunch-control-lite-debug.apk"
dst="${out_dir}/miopunch_${version}_android_arm64_control-lite-debug.apk"

test -s "${src}"
cp "${src}" "${dst}"
test -s "${dst}"

echo "android control-lite APK written to ${dst}"
