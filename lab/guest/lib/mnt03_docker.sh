#!/usr/bin/env bash
set -euo pipefail

MNT03_DOCKER_PREFIX="${MLAB_MNT03_DOCKER_PREFIX:-miopunch-mnt03}"
MNT03_NODE_IMAGE="${MLAB_MNT03_NODE_IMAGE:-}"
MNT03_NETNS_DIR="${MLAB_MNT03_NETNS_DIR:-/var/run/netns}"
MNT03_BIN_DIR="${MLAB_MIOPUNCH_BIN_DIR:-/opt/miopunch-lab/bin}"

mnt03_node_container_name() {
  local node="$1"
  printf '%s-%s' "${MNT03_DOCKER_PREFIX}" "${node}"
}

mnt03_need_docker() {
  need_cmd docker
  need_cmd ip
}

mnt03_build_node_image() {
  local image="$1" build_ctx="$2" guest_root="$3"
  [[ -n "${image}" && -n "${build_ctx}" && -n "${guest_root}" ]] || \
    die "mnt03_build_node_image: missing args"

  mnt03_need_docker
  [[ -x "${MNT03_BIN_DIR}/miopunch" ]] || die "missing binary: ${MNT03_BIN_DIR}/miopunch"
  [[ -x "${MNT03_BIN_DIR}/miopunch-lab" ]] || die "missing binary: ${MNT03_BIN_DIR}/miopunch-lab"
  [[ -x "${MNT03_BIN_DIR}/miopunch-poc-e2e" ]] || die "missing binary: ${MNT03_BIN_DIR}/miopunch-poc-e2e"

  mkdir -p "${build_ctx}/bin"
  cp "${guest_root}/poc-e2e/node/Dockerfile" "${build_ctx}/Dockerfile"
  cp "${MNT03_BIN_DIR}/miopunch" "${build_ctx}/bin/miopunch"
  cp "${MNT03_BIN_DIR}/miopunch-lab" "${build_ctx}/bin/miopunch-lab"
  cp "${MNT03_BIN_DIR}/miopunch-poc-e2e" "${build_ctx}/bin/miopunch-poc-e2e"
  docker build -t "${image}" "${build_ctx}" >/dev/null
}

mnt03_start_systemd_node() {
  local node="$1" image="${2:-${MNT03_NODE_IMAGE}}" node_dir="${3:-}" run_id="${4:-}"
  [[ -n "${node}" ]] || die "mnt03_start_systemd_node: missing node"
  [[ -n "${image}" ]] || die "mnt03_start_systemd_node: missing image; set MLAB_MNT03_NODE_IMAGE"

  mnt03_need_docker

  local name
  name="$(mnt03_node_container_name "${node}")"
  if docker inspect "${name}" >/dev/null 2>&1; then
    docker start "${name}" >/dev/null
    return 0
  fi

  local -a extra_args=()
  if [[ -n "${node_dir}" ]]; then
    mkdir -p "${node_dir}"
    extra_args+=(--volume "${node_dir}:/artifacts")
  fi
  if [[ -n "${run_id}" ]]; then
    extra_args+=(--label "miopunch.mnt03.run_id=${run_id}")
  fi

  docker run \
    --detach \
    --name "${name}" \
    --hostname "${node}" \
    --privileged \
    --cgroupns=host \
    --network none \
    --tmpfs /run \
    --tmpfs /run/lock \
    --tmpfs /tmp \
    --volume /sys/fs/cgroup:/sys/fs/cgroup:rw \
    "${extra_args[@]}" \
    "${image}" \
    /sbin/init >/dev/null
}

mnt03_wait_systemd() {
  local node="$1" name start now status
  name="$(mnt03_node_container_name "${node}")"
  start="$(date +%s)"
  while true; do
    status="$(docker exec "${name}" systemctl is-system-running 2>/dev/null || true)"
    case "${status}" in
      running|degraded)
        return 0
        ;;
    esac
    now="$(date +%s)"
    if ((now - start > 30)); then
      die "systemd not ready: node=${node} status=${status}"
    fi
    sleep 0.2
  done
}

mnt03_container_pid() {
  local node="$1" name
  name="$(mnt03_node_container_name "${node}")"
  docker inspect -f '{{.State.Pid}}' "${name}"
}

mnt03_node_netns() {
  local node="$1"
  printf '%s-%s' "${MNT03_DOCKER_PREFIX}" "${node}"
}

mnt03_bind_container_netns() {
  local node="$1" pid ns
  pid="$(mnt03_container_pid "${node}")"
  [[ -n "${pid}" && "${pid}" != "0" ]] || die "container for ${node} is not running"

  ns="$(mnt03_node_netns "${node}")"
  mkdir -p "${MNT03_NETNS_DIR}"
  ln -sfT "/proc/${pid}/ns/net" "${MNT03_NETNS_DIR}/${ns}"
  printf '%s' "${ns}"
}

mnt03_attach_node_veth() {
  local node="$1" nat_ns="$2" nat_bridge="$3" node_addr="$4" node_gw="$5" node_v6_addr="${6:-}" node_v6_gw="${7:-}"
  [[ -n "${node}" && -n "${nat_ns}" && -n "${node_addr}" && -n "${node_gw}" ]] || \
    die "mnt03_attach_node_veth: missing args"

  mnt03_need_docker

  local node_ns host_if nat_if
  node_ns="$(mnt03_bind_container_netns "${node}")"
  host_if="veth-${node}"
  nat_if="veth-nat-${node}"

  ip link del "${host_if}" >/dev/null 2>&1 || true
  ip link add "${host_if}" type veth peer name "${nat_if}"
  ip link set "${host_if}" netns "${node_ns}"
  ip link set "${nat_if}" netns "${nat_ns}"

  ip -n "${node_ns}" link set lo up
  ip -n "${node_ns}" link set "${host_if}" name eth0
  ip -n "${node_ns}" addr add "${node_addr}" dev eth0
  if [[ -n "${node_v6_addr}" && -n "${node_v6_gw}" ]]; then
    ip netns exec "${node_ns}" sh -c 'sysctl -qw net.ipv6.conf.all.disable_ipv6=0 || true'
    ip netns exec "${node_ns}" sh -c 'sysctl -qw net.ipv6.conf.default.disable_ipv6=0 || true'
    ip -n "${node_ns}" addr add "${node_v6_addr}" dev eth0
  fi
  ip -n "${node_ns}" link set eth0 up
  ip -n "${node_ns}" route replace default via "${node_gw}" dev eth0
  if [[ -n "${node_v6_addr}" && -n "${node_v6_gw}" ]]; then
    ip -n "${node_ns}" -6 route replace default via "${node_v6_gw}" dev eth0
  fi

  ip -n "${nat_ns}" link set "${nat_if}" up
  if [[ -n "${nat_bridge}" ]]; then
    ip -n "${nat_ns}" link set "${nat_if}" master "${nat_bridge}"
  fi
}

mnt03_collect_docker_node() {
  local node="$1" out_dir="$2"
  [[ -n "${node}" && -n "${out_dir}" ]] || die "mnt03_collect_docker_node: missing args"

  local name node_ns
  name="$(mnt03_node_container_name "${node}")"
  mkdir -p "${out_dir}"

  docker inspect "${name}" >"${out_dir}/${node}.docker.inspect.json" 2>"${out_dir}/${node}.docker.inspect.stderr" || true
  docker logs "${name}" >"${out_dir}/${node}.docker.log" 2>"${out_dir}/${node}.docker.stderr" || true
  docker exec "${name}" journalctl --no-pager >"${out_dir}/${node}.journal.log" 2>"${out_dir}/${node}.journal.stderr" || true

  node_ns="$(mnt03_node_netns "${node}")"
  if ip netns list | awk '{print $1}' | grep -qx "${node_ns}"; then
    ip -n "${node_ns}" -j addr >"${out_dir}/${node}.ip.addr.json" 2>/dev/null || true
    ip -n "${node_ns}" -j route >"${out_dir}/${node}.ip.route.json" 2>/dev/null || true
    ip -n "${node_ns}" -j -6 route >"${out_dir}/${node}.ip.route6.json" 2>/dev/null || true
  fi
}

mnt03_remove_systemd_node() {
  local node="$1" name ns
  name="$(mnt03_node_container_name "${node}")"
  ns="$(mnt03_node_netns "${node}")"

  docker rm -f "${name}" >/dev/null 2>&1 || true
  rm -f "${MNT03_NETNS_DIR}/${ns}" 2>/dev/null || true
}
