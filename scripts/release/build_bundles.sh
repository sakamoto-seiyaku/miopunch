#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${MIOPUNCH_RELEASE_OUT:-${repo_root}/dist/release}"
build_dir="${MIOPUNCH_BUILD_DIR:-${repo_root}/dist/build/bundles}"
version="${MIOPUNCH_VERSION:-}"

if [[ -z "${version}" ]]; then
  version="$(git -C "${repo_root}" describe --tags --exact-match 2>/dev/null || true)"
fi
if [[ -z "${version}" ]]; then
  version="0.0.0-git$(git -C "${repo_root}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
fi

ldflags="-s -w"
if [[ -n "${MIOPUNCH_VERSION:-}" ]]; then
  ldflags="${ldflags} -X github.com/miopunch/miopunch/internal/buildinfo.releaseVersion=${MIOPUNCH_VERSION}"
fi

go_bin="${GO:-go}"
mkdir -p "${out_dir}" "${build_dir}"

copy_notices() {
  local dst="$1"
  [[ -f "${repo_root}/LICENSE" ]] && cp "${repo_root}/LICENSE" "${dst}/"
  [[ -f "${repo_root}/NOTICE" ]] && cp "${repo_root}/NOTICE" "${dst}/"
}

write_session_smoke_readme() {
  local dst="$1"
  local goos="$2"

  case "${goos}" in
    windows)
      cat >"${dst}/SMOKE.md" <<EOF
# miopunch Windows Session Smoke

## Launch

1. Extract this zip into a writable directory as a normal user.
2. Run .\\miopunch-desktop.exe from the extracted directory.
3. Verify Settings > Diagnostics shows a connected LocalAPI endpoint.
4. If you want to start the daemon manually first, run .\\miopunch.exe up --session, then open .\\miopunch-desktop.exe.

## Logs

- GUI log: .\\logs\\miopunch-desktop.log
- Daemon log: .\\logs\\miopunch.log

## Data

- Portable state: .\\data\\state.json
- Derived state: .\\data\\net.json, .\\data\\identity\\, .\\data\\decls\\, .\\data\\bootstrap\\, .\\data\\reports\\

Delete .\\data\\ to reset this extracted bundle to a clean node before a new smoke run.

If startup fails, send both log files with the terminal output.

## Desktop smoke

1. Verify the six-stage wizard starts and the top-right chip reports a connected LocalAPI endpoint.
2. Click Refresh and confirm runtime summary/evidence continue to render without falling back to the legacy task UI.
3. Export diagnostics and confirm the archive is written.
4. If a same-user runtime already has peers, optionally verify Punch > SecureSession > Shell locally.

Windows startup, daemon connection, and runtime contract consumption are the blocker checks here.
Windows/Linux real-machine interoperability is explicitly optional for this smoke bundle.

No installer, Administrator prompt, or system service install is required for this session smoke.
EOF
      ;;
    linux)
      cat >"${dst}/SMOKE.md" <<EOF
# miopunch Linux Session Smoke

## Launch

1. Extract this tarball into a writable directory as a normal user.
2. Run ./miopunch-desktop from the extracted directory.
3. Verify Settings > Diagnostics shows a connected LocalAPI endpoint.
4. If you want to start the daemon manually first, run ./miopunch up --session, then open ./miopunch-desktop.

## Logs

- GUI log: ./logs/miopunch-desktop.log
- Daemon log: ./logs/miopunch.log

## Data

- Portable state: ./data/state.json
- Derived state: ./data/net.json, ./data/identity/, ./data/decls/, ./data/bootstrap/, ./data/reports/

Delete ./data/ to reset this extracted bundle to a clean node before a new smoke run.

If startup fails with GTK/display guidance:

- Run from a local graphical desktop session, not a headless SSH shell.
- Check: echo "\$DISPLAY \$WAYLAND_DISPLAY"
- Check missing shared libraries: ldd ./miopunch-desktop | grep 'not found'
- Send ./logs/miopunch-desktop.log with the terminal output.

## Two-machine smoke

1. Start the GUI on both machines and verify both are connected locally.
2. On the first/admin machine, use Network to bootstrap the current network or create a new one.
3. On the first/admin machine, use Enroll > Create invite, then copy the invite code.
4. On the first/admin machine, use Enroll > Approve a joiner, paste the same code, and keep approval running.
5. On the second machine, use Enroll > Join a network, paste the invite code, and click Join.
6. After join and approval complete, click Refresh on both machines and verify each side can see the other peer in Discover.
7. Use Punch to run Ping on the remote peer, confirm SecureSession reports gate satisfied, then use Shell > Open shell and verify the terminal attaches.

No package install, root prompt, or system service install is required for this session smoke.
EOF
      ;;
    *)
      echo "unsupported session smoke target: ${goos}" >&2
      return 2
      ;;
  esac
}

verify_session_dir() {
  local target_dir="$1"
  local goos="$2"
  local ext="$3"

  test -s "${target_dir}/miopunch${ext}"
  test -s "${target_dir}/miopunch-desktop${ext}"
  test -s "${target_dir}/SMOKE.md"
  test -d "${target_dir}/data"
  test -d "${target_dir}/logs"

  if [[ "${goos}" == "linux" ]]; then
    test -x "${target_dir}/miopunch"
    test -x "${target_dir}/miopunch-desktop"
  fi
}

verify_session_archive() {
  local archive="$1"
  local stem="$2"
  local goos="$3"
  local ext="$4"

  case "${goos}" in
    linux)
      tar -tzf "${archive}" | grep -Fx "${stem}/miopunch${ext}" >/dev/null
      tar -tzf "${archive}" | grep -Fx "${stem}/miopunch-desktop${ext}" >/dev/null
      tar -tzf "${archive}" | grep -Fx "${stem}/SMOKE.md" >/dev/null
      tar -tzf "${archive}" | grep -Fx "${stem}/data/" >/dev/null
      tar -tzf "${archive}" | grep -Fx "${stem}/logs/" >/dev/null
      ;;
    windows)
      command -v unzip >/dev/null 2>&1 || {
        echo "missing dependency: unzip" >&2
        return 1
      }
      unzip -Z1 "${archive}" | grep -Fx "${stem}/miopunch${ext}" >/dev/null
      unzip -Z1 "${archive}" | grep -Fx "${stem}/miopunch-desktop${ext}" >/dev/null
      unzip -Z1 "${archive}" | grep -Fx "${stem}/SMOKE.md" >/dev/null
      unzip -Z1 "${archive}" | grep -Fx "${stem}/data/" >/dev/null
      unzip -Z1 "${archive}" | grep -Fx "${stem}/logs/" >/dev/null
      ;;
    *)
      echo "unsupported session archive target: ${goos}" >&2
      return 2
      ;;
  esac
}

build_bundle() {
  local goos="$1"
  local goarch="$2"
  local ext="$3"

  local name="miopunch_${version}_${goos}_${goarch}"
  local target_dir="${build_dir}/${name}"
  rm -rf "${target_dir}"
  mkdir -p "${target_dir}"

  echo "building ${name}"
  (
    cd "${repo_root}"
    GOOS="${goos}" GOARCH="${goarch}" CGO_ENABLED=0 "${go_bin}" build -trimpath -ldflags "${ldflags}" -o "${target_dir}/miopunch${ext}" ./cmd/miopunch
    GOOS="${goos}" GOARCH="${goarch}" CGO_ENABLED=0 "${go_bin}" build -trimpath -ldflags "${ldflags}" -o "${target_dir}/miopunch-lab${ext}" ./cmd/miopunch-lab
    GOOS="${goos}" GOARCH="${goarch}" CGO_ENABLED=0 "${go_bin}" build -trimpath -ldflags "${ldflags}" -o "${target_dir}/miopunch-poc-e2e${ext}" ./tools/miopunch-poc-e2e
  )
  copy_notices "${target_dir}"

  case "${goos}" in
    linux)
      tar -C "${build_dir}" -czf "${out_dir}/${name}.tar.gz" "${name}"
      ;;
    windows)
      (cd "${target_dir}" && zip -q -r "${out_dir}/${name}.zip" .)
      ;;
    *)
      echo "unsupported bundle target: ${goos}/${goarch}" >&2
      return 2
      ;;
  esac
}

build_session_bundle() {
  local goos="$1"
  local goarch="$2"
  local ext="$3"

  local name="miopunch_${version}_${goos}_${goarch}_session"
  local target_dir="${build_dir}/${name}"
  rm -rf "${target_dir}"
  mkdir -p "${target_dir}" "${target_dir}/logs"
  install -d -m 700 "${target_dir}/data"

  echo "building ${name}"
  (
    cd "${repo_root}"
    GOOS="${goos}" GOARCH="${goarch}" CGO_ENABLED=0 "${go_bin}" build -trimpath -ldflags "${ldflags}" -o "${target_dir}/miopunch${ext}" ./cmd/miopunch
    case "${goos}" in
      linux)
        GOOS="${goos}" GOARCH="${goarch}" "${go_bin}" build -trimpath -tags desktop,production -ldflags "${ldflags}" -o "${target_dir}/miopunch-desktop${ext}" ./cmd/miopunch-desktop
        ;;
      windows)
        GOOS="${goos}" GOARCH="${goarch}" "${go_bin}" build -trimpath -tags desktop,production,wv2runtime.embed -ldflags "${ldflags} -H windowsgui" -o "${target_dir}/miopunch-desktop${ext}" ./cmd/miopunch-desktop
        ;;
      *)
        echo "unsupported session bundle target: ${goos}/${goarch}" >&2
        return 2
        ;;
    esac
  )
  copy_notices "${target_dir}"
  write_session_smoke_readme "${target_dir}" "${goos}"
  verify_session_dir "${target_dir}" "${goos}" "${ext}"

  case "${goos}" in
    linux)
      local archive="${out_dir}/${name}.tar.gz"
      tar -C "${build_dir}" -czf "${archive}" "${name}"
      verify_session_archive "${archive}" "${name}" "${goos}" "${ext}"
      ;;
    windows)
      local archive="${out_dir}/${name}.zip"
      (cd "${build_dir}" && zip -q -r "${archive}" "${name}")
      verify_session_archive "${archive}" "${name}" "${goos}" "${ext}"
      ;;
  esac
}

if [[ "${MIOPUNCH_BUILD_LEGACY_BUNDLES:-0}" == "1" ]]; then
  build_bundle linux amd64 ""
  build_bundle windows amd64 ".exe"
fi

build_session_bundle linux amd64 ""
build_session_bundle windows amd64 ".exe"

echo "bundles written to ${out_dir}"
