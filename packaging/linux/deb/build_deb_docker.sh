#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
build_deb_docker.sh - build miopunch .deb packages in distro-matched Docker builders

Usage:
  build_deb_docker.sh [--webkit2_40|--webkit2_41] [--all] [--smoke-install]

Variants:
  --webkit2_40      Build the WebKitGTK 4.0 package in ubuntu:22.04
  --webkit2_41      Build the WebKitGTK 4.1 package in ubuntu:24.04
  --all             Build both variants

Environment (optional):
  MIOPUNCH_VERSION      Release tag/version to embed and convert to Debian version
  MIOPUNCH_DEB_VERSION  Debian package version override
  MIOPUNCH_DEB_OUT      Host output directory for generated .deb files
  MIOPUNCH_GO_VERSION   Go version to install in the builder image; defaults to go.mod
EOF
}

repo_root() {
  git rev-parse --show-toplevel 2>/dev/null || {
    cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd
  }
}

abs_path() {
  local path="$1"
  case "$path" in
    /*) printf '%s\n' "$path" ;;
    *) printf '%s/%s\n' "$(pwd)" "$path" ;;
  esac
}

ubuntu_for_variant() {
  case "$1" in
    webkit2_40) printf '22.04\n' ;;
    webkit2_41) printf '24.04\n' ;;
    *) echo "unknown variant: $1" >&2; exit 2 ;;
  esac
}

build_arg_for_variant() {
  case "$1" in
    webkit2_40) printf '%s\n' "--webkit2_40" ;;
    webkit2_41) printf '%s\n' "--webkit2_41" ;;
    *) echo "unknown variant: $1" >&2; exit 2 ;;
  esac
}

go_version() {
  if [[ -n "${MIOPUNCH_GO_VERSION:-}" ]]; then
    printf '%s\n' "$MIOPUNCH_GO_VERSION"
    return
  fi

  awk '$1 == "go" { print $2; exit }' "$(repo_root)/go.mod"
}

build_image() {
  local root="$1"
  local variant="$2"
  local ubuntu="$3"
  local go_version="$4"
  local image="$5"

  docker build \
    --file "$root/packaging/linux/deb/docker/Dockerfile" \
    --build-arg "UBUNTU_VERSION=$ubuntu" \
    --build-arg "WEBKIT_VARIANT=$variant" \
    --build-arg "GO_VERSION=$go_version" \
    --tag "$image" \
    "$root/packaging/linux/deb/docker"
}

run_build() {
  local root="$1"
  local variant="$2"
  local host_out="$3"
  local image="$4"
  local build_arg uid gid output deb_path deb_name host_deb

  build_arg="$(build_arg_for_variant "$variant")"
  uid="$(id -u)"
  gid="$(id -g)"

  output="$(
    docker run --rm \
      --mount "type=bind,source=$root,target=/workspace,readonly" \
      --mount "type=bind,source=$host_out,target=/deb-out" \
      --mount "type=volume,source=miopunch-go-mod,target=/go/pkg/mod" \
      --mount "type=volume,source=miopunch-go-build-${variant},target=/go/build-cache" \
      --env "MIOPUNCH_VERSION=${MIOPUNCH_VERSION:-}" \
      --env "MIOPUNCH_DEB_VERSION=${MIOPUNCH_DEB_VERSION:-}" \
      --env "MIOPUNCH_DEB_OUT=/deb-out" \
      --env "BUILD_ARG=$build_arg" \
      --env "HOST_UID=$uid" \
      --env "HOST_GID=$gid" \
      --workdir /workspace \
      "$image" \
      bash -lc 'git config --global --add safe.directory /workspace && bash packaging/linux/deb/build_deb.sh "$BUILD_ARG" && chown -R "$HOST_UID:$HOST_GID" /deb-out'
  )"

  printf '%s\n' "$output" >&2
  deb_path="$(printf '%s\n' "$output" | awk '/^built: / { print $2 }' | tail -n 1)"
  if [[ -z "$deb_path" ]]; then
    echo "could not find built package path in Docker output" >&2
    exit 1
  fi

  deb_name="$(basename "$deb_path")"
  host_deb="$host_out/$deb_name"
  if [[ ! -s "$host_deb" ]]; then
    echo "expected built package missing: $host_deb" >&2
    exit 1
  fi

  printf '%s\n' "$host_deb"
}

smoke_installability() {
  local variant="$1"
  local host_out="$2"
  local host_deb="$3"
  local ubuntu deb_name

  ubuntu="$(ubuntu_for_variant "$variant")"
  deb_name="$(basename "$host_deb")"

  echo "smoke: apt resolver check for $deb_name on ubuntu:$ubuntu"
  docker run --rm \
    --mount "type=bind,source=$host_out,target=/deb-out,readonly" \
    --env "DEB_NAME=$deb_name" \
    "ubuntu:$ubuntu" \
    bash -lc '
      set -euo pipefail
      export DEBIAN_FRONTEND=noninteractive
      apt-get update >/dev/null
      apt-get install -y --no-install-recommends --simulate "/deb-out/${DEB_NAME}" >/tmp/apt-sim.log
      grep -E "^(Inst|Conf) miopunch" /tmp/apt-sim.log
    '
}

variant="webkit2_40"
all=false
smoke_install=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --webkit2_40)
      variant="webkit2_40"
      shift
      ;;
    --webkit2_41)
      variant="webkit2_41"
      shift
      ;;
    --all)
      all=true
      shift
      ;;
    --smoke-install)
      smoke_install=true
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

if ! command -v docker >/dev/null 2>&1; then
  echo "missing required command: docker" >&2
  exit 1
fi

root="$(repo_root)"
host_out="$(abs_path "${MIOPUNCH_DEB_OUT:-$root/packaging/linux/deb/out}")"
go_version="$(go_version)"
mkdir -p "$host_out"

variants=("$variant")
if $all; then
  variants=("webkit2_40" "webkit2_41")
fi

for current_variant in "${variants[@]}"; do
  ubuntu="$(ubuntu_for_variant "$current_variant")"
  image="miopunch-deb-builder:ubuntu${ubuntu}-go${go_version}-${current_variant}"

  echo "builder: $image"
  build_image "$root" "$current_variant" "$ubuntu" "$go_version" "$image"
  host_deb="$(run_build "$root" "$current_variant" "$host_out" "$image")"
  echo "host package: $host_deb"

  if $smoke_install; then
    smoke_installability "$current_variant" "$host_out" "$host_deb"
  fi
done
