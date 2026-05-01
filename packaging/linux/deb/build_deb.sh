#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
build_deb.sh — build miopunch .deb packages

Usage:
  build_deb.sh [--webkit2_41] [--all]

Environment (optional):
  MIOPUNCH_BIN          Path to a prebuilt /usr/bin/miopunch binary
  MIOPUNCH_DESKTOP_BIN  Path to a prebuilt /usr/bin/miopunch-desktop binary
  MIOPUNCH_VERSION      Release tag/version to embed and convert to Debian version
  MIOPUNCH_DEB_VERSION  Debian package version override
  MIOPUNCH_DEB_OUT      Output directory for generated .deb files
EOF
}

repo_root() {
  git rev-parse --show-toplevel 2>/dev/null || {
    cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd
  }
}

version_string() {
  local root sha dirty
  if [[ -n "${MIOPUNCH_DEB_VERSION:-}" ]]; then
    printf '%s\n' "${MIOPUNCH_DEB_VERSION}"
    return
  fi
  if [[ -n "${MIOPUNCH_VERSION:-}" ]]; then
    local version="${MIOPUNCH_VERSION#v}"
    version="${version/-rc./~rc.}"
    version="${version//_/.}"
    printf '%s\n' "${version}"
    return
  fi

  root="$(repo_root)"
  sha="$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  dirty=""
  if ! git -C "$root" diff --quiet --ignore-submodules HEAD -- 2>/dev/null; then
    dirty="+dirty"
  fi
  printf '0.0.0+git%s%s' "$sha" "$dirty"
}

build_one() {
  local variant="$1"

  local root arch version depends webkit_suffix go_tags
  root="$(repo_root)"
  arch="$(dpkg --print-architecture)"

  version="$(version_string)"
  webkit_suffix=""
  go_tags="desktop"
  depends="libgtk-3-0, libwebkit2gtk-4.0-37"
  if [[ "$variant" == "webkit2_41" ]]; then
    version="${version}+webkit2.41"
    webkit_suffix="_webkit2_41"
    go_tags="desktop,webkit2_41"
    depends="libgtk-3-0, libwebkit2gtk-4.1-0"
  fi

  local work pkgdir outdir outfile
  work="$(mktemp -d)"
  pkgdir="$work/pkg"
  outdir="${MIOPUNCH_DEB_OUT:-$root/packaging/linux/deb/out}"
  mkdir -p "$pkgdir/DEBIAN"
  mkdir -p "$pkgdir/usr/bin"
  mkdir -p "$pkgdir/usr/share/applications"
  mkdir -p "$pkgdir/usr/share/icons/hicolor/scalable/apps"
  mkdir -p "$outdir"

  cat >"$pkgdir/DEBIAN/control" <<EOF
Package: miopunch
Version: $version
Section: net
Priority: optional
Architecture: $arch
Maintainer: miopunch developers <devnull@example.invalid>
Depends: $depends
Description: miopunch desktop shell (Wails) + daemon
 miopunch provides a desktop GUI (miopunch-desktop) and a local daemon/CLI (miopunch).
EOF

  install -m 0755 "$root/packaging/linux/deb/scripts/postinst" "$pkgdir/DEBIAN/postinst"
  install -m 0755 "$root/packaging/linux/deb/scripts/prerm" "$pkgdir/DEBIAN/prerm"
  install -m 0755 "$root/packaging/linux/deb/scripts/postrm" "$pkgdir/DEBIAN/postrm"

  if [[ -n "${MIOPUNCH_BIN:-}" ]]; then
    install -m 0755 "$MIOPUNCH_BIN" "$pkgdir/usr/bin/miopunch"
  else
    local ldflags="-s -w"
    if [[ -n "${MIOPUNCH_VERSION:-}" ]]; then
      ldflags="${ldflags} -X github.com/miopunch/miopunch/internal/buildinfo.releaseVersion=${MIOPUNCH_VERSION}"
    fi
    (cd "$root" && go build -trimpath -ldflags "$ldflags" -o "$pkgdir/usr/bin/miopunch" ./cmd/miopunch)
  fi

  if [[ -n "${MIOPUNCH_DESKTOP_BIN:-}" ]]; then
    install -m 0755 "$MIOPUNCH_DESKTOP_BIN" "$pkgdir/usr/bin/miopunch-desktop"
  else
    local desktop_ldflags="-s -w"
    if [[ -n "${MIOPUNCH_VERSION:-}" ]]; then
      desktop_ldflags="${desktop_ldflags} -X github.com/miopunch/miopunch/internal/buildinfo.releaseVersion=${MIOPUNCH_VERSION}"
    fi
    (cd "$root" && go build -trimpath -tags "$go_tags" -ldflags "$desktop_ldflags" -o "$pkgdir/usr/bin/miopunch-desktop" ./cmd/miopunch-desktop)
  fi

  install -m 0644 "$root/packaging/linux/deb/miopunch.desktop" "$pkgdir/usr/share/applications/miopunch.desktop"
  install -m 0644 "$root/packaging/linux/deb/icons/miopunch.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/miopunch.svg"

  outfile="$outdir/miopunch_${version}${webkit_suffix}_${arch}.deb"
  dpkg-deb --build "$pkgdir" "$outfile" >/dev/null

  echo "built: $outfile"
  rm -rf "$work"
}

variant="webkit2_40"
all=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --webkit2_41)
      variant="webkit2_41"
      shift
      ;;
    --all)
      all=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if $all; then
  build_one "webkit2_40"
  build_one "webkit2_41"
else
  build_one "$variant"
fi
