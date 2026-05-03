#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${MIOPUNCH_RELEASE_OUT:-${repo_root}/dist/release}"
work_dir="${MIOPUNCH_WINDOWS_INSTALLER_WORK:-${repo_root}/dist/build/windows-installer}"
version="${MIOPUNCH_VERSION:-}"

if [[ -z "${version}" ]]; then
  version="$(git -C "${repo_root}" describe --tags --exact-match 2>/dev/null || true)"
fi
if [[ -z "${version}" ]]; then
  version="0.0.0-git$(git -C "${repo_root}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
fi

command -v makensis >/dev/null 2>&1 || {
  echo "missing dependency: makensis" >&2
  exit 1
}

ldflags="-s -w"
if [[ -n "${MIOPUNCH_VERSION:-}" ]]; then
  ldflags="${ldflags} -X github.com/miopunch/miopunch/internal/buildinfo.releaseVersion=${MIOPUNCH_VERSION}"
fi
desktop_tags="desktop,production,wv2runtime.embed"
desktop_ldflags="${ldflags} -H windowsgui"

go_bin="${GO:-go}"
rm -rf "${work_dir}"
mkdir -p "${work_dir}" "${out_dir}"

cp "${repo_root}/packaging/windows/nsis/miopunch.nsi" "${work_dir}/miopunch.nsi"

(
  cd "${repo_root}"
  GOOS=windows GOARCH=amd64 CGO_ENABLED=0 "${go_bin}" build -trimpath -ldflags "${ldflags}" -o "${work_dir}/miopunch.exe" ./cmd/miopunch
  GOOS=windows GOARCH=amd64 "${go_bin}" build -trimpath -tags "${desktop_tags}" -ldflags "${desktop_ldflags}" -o "${work_dir}/miopunch-desktop.exe" ./cmd/miopunch-desktop
)

(cd "${work_dir}" && makensis -DMIOPUNCH_VERSION="${version}" miopunch.nsi)
mv "${work_dir}/miopunch-setup.exe" "${out_dir}/miopunch_${version}_windows_amd64_setup.exe"

echo "installer written to ${out_dir}/miopunch_${version}_windows_amd64_setup.exe"
