#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
guest_root="$(cd -- "${script_dir}/.." && pwd)"

# shellcheck source=/dev/null
source "${guest_root}/lib/common.sh"

artifacts_root="${MLAB_ARTIFACTS_ROOT:-/opt/miopunch-lab/artifacts}"
bin_dir="${MLAB_MIOPUNCH_BIN_DIR:-/opt/miopunch-lab/bin}"

broker_image="${MIOPUNCH_POC_E2E_BROKER_IMAGE:-eclipse-mosquitto:2.0.18}"
sniffer_image="${MIOPUNCH_POC_E2E_SNIFFER_IMAGE:-nicolaka/netshoot:v0.13}"
node_image_repo="${MIOPUNCH_POC_E2E_NODE_IMAGE_REPO:-miopunch-poc-e2e-node}"

run_id=""
run_dir=""
cleanup_log=""

docker_network=""
node_image=""

broker_container=""
sniffer_container=""

declare -a node_containers=()

poc_e2e_preflight() {
  need_cmd docker
  need_cmd jq
  need_cmd sed
  need_cmd date

  [[ -x "${bin_dir}/miopunch" ]] || die "missing binary: ${bin_dir}/miopunch (run: labctl push-bin)"
  [[ -x "${bin_dir}/miopunch-lab" ]] || die "missing binary: ${bin_dir}/miopunch-lab (run: labctl push-bin)"
  [[ -x "${bin_dir}/miopunch-poc-e2e" ]] || die "missing binary: ${bin_dir}/miopunch-poc-e2e (run: labctl push-bin)"
}

poc_e2e_new_run() {
  local kind="$1"

  local ts rand
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  rand="$RANDOM$RANDOM"

  run_id="${ts}-poc-e2e-${kind}-${rand}"
  run_dir="${artifacts_root}/${run_id}"
  cleanup_log="${run_dir}/cleanup.log"
  mkdir -p "${run_dir}"
  : >"${cleanup_log}"
  mkdir -p "${run_dir}/steps" "${run_dir}/cases" "${run_dir}/nodes"

  docker_network="miopunch-poc-e2e-${rand}"
  node_image="${node_image_repo}:${rand}"

  {
    echo "run_id=${run_id}"
    echo "kind=${kind}"
    echo "broker_image=${broker_image}"
    echo "sniffer_image=${sniffer_image}"
    echo "node_image=${node_image}"
    echo "docker_network=${docker_network}"
  } >"${run_dir}/run.env"

  echo "run_dir=${run_dir}"
}

poc_e2e_finish() {
  local rc=$?
  set +e
  if [[ -n "${docker_network}" ]]; then
    local node container
    for node in node-a node-b node-c node-d; do
      container="${docker_network}-${node}"
      if docker inspect "${container}" >/dev/null 2>&1; then
        poc_e2e_collect_node_diagnostics "${node}" || true
      fi
    done
  fi
  poc_e2e_collect_docker_meta || true
  poc_e2e_cleanup || true
  chmod -R a+rX "${run_dir}" 2>/dev/null || true
  set -e
  exit "${rc}"
}

poc_e2e_docker_network_create() {
  docker network create \
    --label "miopunch.poc_e2e.run_id=${run_id}" \
    "${docker_network}" >/dev/null
}

poc_e2e_build_node_image() {
  local build_ctx="${run_dir}/_image/node"
  mkdir -p "${build_ctx}/bin"

  cp "${guest_root}/poc-e2e/node/Dockerfile" "${build_ctx}/Dockerfile"
  cp "${bin_dir}/miopunch" "${build_ctx}/bin/miopunch"
  cp "${bin_dir}/miopunch-lab" "${build_ctx}/bin/miopunch-lab"
  cp "${bin_dir}/miopunch-poc-e2e" "${build_ctx}/bin/miopunch-poc-e2e"

  docker build -t "${node_image}" "${build_ctx}" >/dev/null
}

poc_e2e_start_broker() {
  mkdir -p "${run_dir}/broker"
  broker_container="${docker_network}-broker"

  cat >"${run_dir}/broker/mosquitto.conf" <<'EOF'
listener 1883 0.0.0.0
allow_anonymous true
EOF

  docker run -d \
    --name "${broker_container}" \
    --hostname broker \
    --network "${docker_network}" \
    --network-alias broker \
    --label "miopunch.poc_e2e.run_id=${run_id}" \
    -v "${run_dir}/broker/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
    "${broker_image}" >/dev/null
}

poc_e2e_wait_broker() {
  local container="$1"
  local start now
  start="$(date +%s)"

  while true; do
    if docker exec "${container}" nc -z -w 1 broker 1883 >/dev/null 2>&1; then
      return 0
    fi

    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "broker not ready: container=${container}"
    fi
    sleep 0.2
  done
}

poc_e2e_start_sniffer() {
  mkdir -p "${run_dir}/pcap"
  sniffer_container="${docker_network}-sniffer"

  docker run -d \
    --name "${sniffer_container}" \
    --network "${docker_network}" \
    --label "miopunch.poc_e2e.run_id=${run_id}" \
    --cap-add NET_ADMIN \
    --cap-add NET_RAW \
    -v "${run_dir}/pcap:/artifacts" \
    "${sniffer_image}" \
    tcpdump -ni any -w /artifacts/traffic.pcap >/dev/null
}

poc_e2e_start_node() {
  local node="$1"
  local container="${docker_network}-${node}"
  local node_dir="${run_dir}/nodes/${node}"
  mkdir -p "${node_dir}" \
    "${node_dir}/reports/self" \
    "${node_dir}/reports/full" \
    "${node_dir}/diag" \
    "${node_dir}/state"

  docker run -d \
    --name "${container}" \
    --hostname "${node}" \
    --network "${docker_network}" \
    --network-alias "${node}" \
    --label "miopunch.poc_e2e.run_id=${run_id}" \
    --cgroupns=host \
    --privileged \
    --tmpfs /run \
    --tmpfs /run/lock \
    --tmpfs /tmp \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    -v "${node_dir}:/artifacts" \
    "${node_image}" >/dev/null

  node_containers+=("${container}")
}

poc_e2e_wait_systemd() {
  local container="$1"
  local start now
  start="$(date +%s)"

  while true; do
    local status=""
    status="$(docker exec "${container}" systemctl is-system-running 2>/dev/null || true)"
    case "${status}" in
      running|degraded)
        return 0
        ;;
    esac

    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "systemd not ready: container=${container}"
    fi
    sleep 0.2
  done
}

poc_e2e_step() {
  local scope="$1" step="$2"
  shift 2

  local out_dir="${run_dir}/steps/${scope}"
  mkdir -p "${out_dir}"

  local stdout_file="${out_dir}/${step}.stdout"
  local stderr_file="${out_dir}/${step}.stderr"
  local rc_file="${out_dir}/${step}.rc"

  set +e
  "$@" >"${stdout_file}" 2>"${stderr_file}"
  local rc=$?
  set -e

  echo "${rc}" >"${rc_file}"
  if [[ "${rc}" -ne 0 ]]; then
    return "${rc}"
  fi
}

poc_e2e_node_exec() {
  local node="$1" step="$2"
  shift 2
  local container="${docker_network}-${node}"
  poc_e2e_step "${node}" "${step}" docker exec "${container}" "$@"
}

poc_e2e_node_exec_sh() {
  local node="$1" step="$2" cmd="$3"
  local container="${docker_network}-${node}"
  poc_e2e_step "${node}" "${step}" docker exec "${container}" bash -lc "${cmd}"
}

poc_e2e_install_daemon() {
  local node="$1"
  poc_e2e_node_exec_sh "${node}" "install_daemon" "miopunch install-system-daemon"
}

poc_e2e_wait_daemon_active() {
  local node="$1"
  local container="${docker_network}-${node}"
  local start now
  start="$(date +%s)"

  while true; do
    if docker exec "${container}" systemctl is-active --quiet miopunch 2>/dev/null; then
      return 0
    fi
    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "miopunch systemd service not active: node=${node}"
    fi
    sleep 0.2
  done
}

poc_e2e_wait_localapi() {
  local node="$1"
  local container="${docker_network}-${node}"
  local start now
  start="$(date +%s)"

  while true; do
    if docker exec "${container}" test -S /run/miopunch/localapi.sock 2>/dev/null; then
      if docker exec "${container}" miopunch --localapi unix:/run/miopunch/localapi.sock --format json ls >/dev/null 2>&1; then
        return 0
      fi
    fi
    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "localapi not ready: node=${node}"
    fi
    sleep 0.2
  done
}

poc_e2e_wait_tmux_client() {
  local node="$1"
  local session="$2"
  local container="${docker_network}-${node}"
  local start now
  start="$(date +%s)"

  while true; do
    local client_pid=""
    client_pid="$(
      docker exec "${container}" tmux list-clients -t "${session}" -F '#{client_pid}' 2>/dev/null |
        head -n 1 |
        tr -d '[:space:]' || true
    )"
    if [[ -n "${client_pid}" ]]; then
      return 0
    fi

    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "tmux client not attached: node=${node} session=${session}"
    fi
    sleep 0.2
  done
}

poc_e2e_redact_file_in_node() {
  local node="$1" in_path="$2" out_path="$3"
  local container="${docker_network}-${node}"
  docker exec "${container}" bash -lc "sed -E 's/(invite_code=)[^[:space:]]+/\\1<redacted>/g; s/(secret_key=)[^[:space:]]+/\\1<redacted>/g; s/(net_secret_b64=)[^[:space:]]+/\\1<redacted>/g; s/(invite_secret_b64=)[^[:space:]]+/\\1<redacted>/g' \"${in_path}\" >\"${out_path}\""
}

poc_e2e_pin_broker_state() {
  local node="$1"
  local container="${docker_network}-${node}"
  docker exec "${container}" bash -lc 'set -euo pipefail
mkdir -p /var/lib/miopunch
if [[ -f /var/lib/miopunch/state.json ]]; then
  jq ".local.mqtt_broker = \"broker:1883\"" /var/lib/miopunch/state.json > /var/lib/miopunch/state.json.tmp
  mv /var/lib/miopunch/state.json.tmp /var/lib/miopunch/state.json
  exit 0
fi

cat > /var/lib/miopunch/state.json <<EOF
{
  "format": "miopunch.state.poc.v0",
  "local": {
    "mqtt_broker": "broker:1883"
  },
  "peers": {}
}
EOF
'
}

poc_e2e_collect_node_diagnostics() {
  local node="$1"
  local container="${docker_network}-${node}"

  docker exec "${container}" bash -lc 'set -euo pipefail
mkdir -p /artifacts/diag
journalctl -u miopunch --no-pager > /artifacts/diag/miopunch.journal.log 2>&1 || true
systemctl status miopunch --no-pager > /artifacts/diag/miopunch.systemctl.txt 2>&1 || true
ls -la /run/miopunch > /artifacts/diag/run_miopunch.ls.txt 2>&1 || true
ls -la /var/lib/miopunch > /artifacts/diag/var_lib_miopunch.ls.txt 2>&1 || true
'

  docker exec "${container}" bash -lc 'set -euo pipefail
mkdir -p /artifacts/state

if [[ -f /var/lib/miopunch/state.json ]]; then
  jq '\''(.local.secret_key? |= "<redacted>") | (.peers? |= with_entries(.value.secret_key = "<redacted>"))'\'' /var/lib/miopunch/state.json > /artifacts/state/state.json 2>/dev/null || true
fi
if [[ -f /var/lib/miopunch/net.json ]]; then
  jq '\''.net_secret_b64 = "<redacted>"'\'' /var/lib/miopunch/net.json > /artifacts/state/net.json 2>/dev/null || true
fi
if [[ -f /var/lib/miopunch/identity/identity.json ]]; then
  jq '\''.ed25519_seed_b64 = "<redacted>" | .x25519_priv_b64 = "<redacted>"'\'' /var/lib/miopunch/identity/identity.json > /artifacts/state/identity.json 2>/dev/null || true
fi
if [[ -f /var/lib/miopunch/governance/head_snapshot.json ]]; then
  cp -f /var/lib/miopunch/governance/head_snapshot.json /artifacts/state/head_snapshot.json 2>/dev/null || true
fi
if [[ -f /var/lib/miopunch/decls/decls.json ]]; then
  cp -f /var/lib/miopunch/decls/decls.json /artifacts/state/decls.json 2>/dev/null || true
fi
'
}

poc_e2e_collect_docker_meta() {
  mkdir -p "${run_dir}/docker"

  if [[ -n "${docker_network}" ]]; then
    docker network inspect "${docker_network}" >"${run_dir}/docker/network.inspect.json" 2>&1 || true
  fi
  if [[ -n "${broker_container}" ]]; then
    docker inspect "${broker_container}" >"${run_dir}/docker/broker.inspect.json" 2>&1 || true
    docker logs "${broker_container}" >"${run_dir}/broker/broker.log" 2>&1 || true
  fi
  if [[ -n "${sniffer_container}" ]]; then
    docker inspect "${sniffer_container}" >"${run_dir}/docker/sniffer.inspect.json" 2>&1 || true
  fi

  local c
  for c in "${node_containers[@]}"; do
    docker inspect "${c}" >"${run_dir}/docker/${c}.inspect.json" 2>&1 || true
    docker logs "${c}" >"${run_dir}/docker/${c}.log" 2>&1 || true
  done

  docker image inspect "${node_image}" >"${run_dir}/docker/node_image.inspect.json" 2>&1 || true
}

poc_e2e_cleanup() {
  {
    echo "cleanup_start_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "run_id=${run_id}"
  } >>"${cleanup_log}" 2>/dev/null || true

  if [[ -n "${sniffer_container}" ]]; then
    docker kill -s INT "${sniffer_container}" >/dev/null 2>&1 || true
  fi

  local c
  for c in "${node_containers[@]}"; do
    docker rm -f "${c}" >/dev/null 2>&1 || true
  done

  if [[ -n "${broker_container}" ]]; then
    docker rm -f "${broker_container}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${sniffer_container}" ]]; then
    docker rm -f "${sniffer_container}" >/dev/null 2>&1 || true
  fi

  if [[ -n "${docker_network}" ]]; then
    docker network rm "${docker_network}" >/dev/null 2>&1 || true
  fi

  {
    echo "cleanup_end_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "leftover_containers=$(docker ps -a --filter label=miopunch.poc_e2e.run_id=${run_id} --format '{{.Names}}' | tr '\n' ',' | sed 's/,$//')"
    echo "leftover_networks=$(docker network ls --filter label=miopunch.poc_e2e.run_id=${run_id} --format '{{.Name}}' | tr '\n' ',' | sed 's/,$//')"
  } >>"${cleanup_log}" 2>/dev/null || true
}

poc_e2e_extract_fact_value() {
  local json_file="$1" prefix="$2"
  jq -r --arg p "${prefix}" '
    .facts[]? | select(.message? | startswith($p)) | .message
  ' "${json_file}" | sed -E "s/^${prefix}//" | head -n 1
}

poc_e2e_assert_file_in_node() {
  local node="$1" path="$2"
  local container="${docker_network}-${node}"
  docker exec "${container}" test -f "${path}"
}

poc_e2e_assert_dir_in_node() {
  local node="$1" path="$2"
  local container="${docker_network}-${node}"
  docker exec "${container}" test -d "${path}"
}

poc_e2e_selftest() {
  poc_e2e_preflight
  poc_e2e_new_run "selftest"
  trap poc_e2e_finish EXIT

  mkdir -p "${run_dir}/steps/node-a" "${run_dir}/steps/node-b"

  poc_e2e_docker_network_create
  poc_e2e_build_node_image

  poc_e2e_start_broker
  poc_e2e_start_node "node-a"
  poc_e2e_start_node "node-b"

  poc_e2e_wait_systemd "${docker_network}-node-a"
  poc_e2e_wait_systemd "${docker_network}-node-b"
  poc_e2e_wait_broker "${docker_network}-node-a"

  poc_e2e_install_daemon "node-a"
  poc_e2e_install_daemon "node-b"
  poc_e2e_wait_daemon_active "node-a"
  poc_e2e_wait_daemon_active "node-b"
  poc_e2e_wait_localapi "node-a"
  poc_e2e_wait_localapi "node-b"

  poc_e2e_pin_broker_state "node-a"

  mkdir -p "${run_dir}/cases/self"

  # Invite (capture code, then redact report on disk).
  local invite_json_tmp invite_json invite_code issuer_peer_id
  invite_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/invite.md invite --mode approve --uses 1 --expires 15m" \
    >"${invite_json_tmp}" 2>>"${run_dir}/steps/node-a/invite.stderr" || true
  invite_json="$(sed -E 's/(invite_code=)[^[:space:]]+/\\1<redacted>/g; s/(secret_key=)[^[:space:]]+/\\1<redacted>/g; s/(net_secret_b64=)[^[:space:]]+/\\1<redacted>/g; s/(invite_secret_b64=)[^[:space:]]+/\\1<redacted>/g' "${invite_json_tmp}")"
  printf '%s\n' "${invite_json}" >"${run_dir}/cases/self/invite.json"
  invite_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  issuer_peer_id="$(jq -r '.facts[]? | select(.message? | startswith("peer_id=")) | .message' "${invite_json_tmp}" | sed -E 's/^peer_id=//' | head -n 1)"
  rm -f "${invite_json_tmp}"
  [[ -n "${invite_code}" ]] || die "failed to extract invite code"
  [[ -n "${issuer_peer_id}" ]] || die "failed to extract issuer peer id"

  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/self/invite.md" "/artifacts/reports/self/invite.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/self/invite.md" >/dev/null 2>&1 || true

  # Approve + Join.
  mkdir -p "${run_dir}/steps/node-a" "${run_dir}/steps/node-b"
  set +e
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/approve.md approve '${invite_code}'" \
    >"${run_dir}/cases/self/approve.json" 2>"${run_dir}/steps/node-a/approve.stderr" &
  local approve_pid=$!
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/join.md join '${invite_code}'" \
    >"${run_dir}/cases/self/join.json" 2>"${run_dir}/steps/node-b/join.stderr"
  local join_rc=$?
  wait "${approve_pid}"
  local approve_rc=$?
  set -e
  [[ "${join_rc}" -eq 0 ]] || die "join failed: rc=${join_rc}"
  [[ "${approve_rc}" -eq 0 ]] || die "approve failed: rc=${approve_rc}"

  # Assert member snapshots exist.
  poc_e2e_assert_file_in_node "node-b" "/var/lib/miopunch/identity/identity.json"
  poc_e2e_assert_file_in_node "node-b" "/var/lib/miopunch/net.json"
  poc_e2e_assert_file_in_node "node-b" "/var/lib/miopunch/governance/head_snapshot.json"
  poc_e2e_assert_file_in_node "node-b" "/var/lib/miopunch/decls/decls.json"
  poc_e2e_assert_file_in_node "node-b" "/var/lib/miopunch/state.json"

  # Ping.
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/ping.md ping '${issuer_peer_id}'" \
    >"${run_dir}/cases/self/ping.json" 2>"${run_dir}/steps/node-b/ping.stderr"

  # Prepare tmux session on node-a.
  docker exec "${docker_network}-node-a" bash -lc "tmux new-session -d -s main || true; tmux send-keys -t main 'echo POC_E2E_READY' C-m"

  # Shell list.
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/sh_ls.md sh ls '${issuer_peer_id}' local" \
    >"${run_dir}/cases/self/sh_ls.json" 2>"${run_dir}/steps/node-b/sh_ls.stderr"

  # Shell attach marker (LocalAPI WS helper).
  local marker="POC_E2E_MARKER_${RANDOM}"
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch-poc-e2e sh-attach --localapi unix:/run/miopunch/localapi.sock --peer-id '${issuer_peer_id}' --target local --session main --send 'echo ${marker}\\n' --expect '${marker}' --timeout 20s" \
    >"${run_dir}/cases/self/sh_attach.json" 2>"${run_dir}/steps/node-b/sh_attach.stderr"

  # Revoke and deny.
  local member_peer_id
  member_peer_id="$(poc_e2e_extract_fact_value "${run_dir}/cases/self/join.json" "peer_id=")"
  [[ -n "${member_peer_id}" ]] || die "failed to extract member peer_id"

  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/revoke.md revoke '${member_peer_id}' --dangerous" \
    >"${run_dir}/cases/self/revoke.json" 2>"${run_dir}/steps/node-a/revoke.stderr"

  set +e
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/self/ping_after_revoke.md ping '${issuer_peer_id}'" \
    >"${run_dir}/cases/self/ping_after_revoke.json" 2>"${run_dir}/steps/node-b/ping_after_revoke.stderr"
  local denied_rc=$?
  set -e
  [[ "${denied_rc}" -ne 0 ]] || die "expected revoked member to be denied"

  local denied_reason
  denied_reason="$(jq -r '.reason_code' "${run_dir}/cases/self/ping_after_revoke.json" | tr -d '\r\n')"
  [[ "${denied_reason}" == "FORBIDDEN" ]] || die "expected FORBIDDEN after revoke, got reason_code=${denied_reason}"

  poc_e2e_collect_node_diagnostics "node-a"
  poc_e2e_collect_node_diagnostics "node-b"
}

poc_e2e_fulltest() {
  poc_e2e_preflight
  poc_e2e_new_run "fulltest"
  trap poc_e2e_finish EXIT

  mkdir -p "${run_dir}/steps/node-a" "${run_dir}/steps/node-b" "${run_dir}/steps/node-c" "${run_dir}/steps/node-d"

  poc_e2e_docker_network_create
  poc_e2e_build_node_image
  poc_e2e_start_broker
  poc_e2e_start_sniffer

  poc_e2e_start_node "node-a"
  poc_e2e_start_node "node-b"
  poc_e2e_start_node "node-c"
  poc_e2e_start_node "node-d"

  poc_e2e_wait_systemd "${docker_network}-node-a"
  poc_e2e_wait_systemd "${docker_network}-node-b"
  poc_e2e_wait_systemd "${docker_network}-node-c"
  poc_e2e_wait_systemd "${docker_network}-node-d"
  poc_e2e_wait_broker "${docker_network}-node-a"

  poc_e2e_install_daemon "node-a"
  poc_e2e_install_daemon "node-b"
  poc_e2e_install_daemon "node-c"
  poc_e2e_install_daemon "node-d"
  poc_e2e_wait_daemon_active "node-a"
  poc_e2e_wait_daemon_active "node-b"
  poc_e2e_wait_daemon_active "node-c"
  poc_e2e_wait_daemon_active "node-d"
  poc_e2e_wait_localapi "node-a"
  poc_e2e_wait_localapi "node-b"
  poc_e2e_wait_localapi "node-c"
  poc_e2e_wait_localapi "node-d"

  poc_e2e_pin_broker_state "node-a"

  mkdir -p "${run_dir}/cases/full"

  # F01: missing approve -> join timeout (short expiry).
  local invite_json_tmp invite_code issuer_peer_id
  invite_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f01_invite.md invite --mode approve --uses 1 --expires 12s" \
    >"${invite_json_tmp}" 2>>"${run_dir}/steps/node-a/f01_invite.stderr" || true
  invite_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  issuer_peer_id="$(jq -r '.facts[]? | select(.message? | startswith("peer_id=")) | .message' "${invite_json_tmp}" | sed -E 's/^peer_id=//' | head -n 1)"
  rm -f "${invite_json_tmp}"
  [[ -n "${invite_code}" ]] || die "f01: failed to extract invite code"
  [[ -n "${issuer_peer_id}" ]] || die "f01: failed to extract issuer peer id"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f01_invite.md" "/artifacts/reports/full/f01_invite.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f01_invite.md" >/dev/null 2>&1 || true

  set +e
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f01_join.md join '${invite_code}'" \
    >"${run_dir}/cases/full/f01_join.json" 2>"${run_dir}/steps/node-b/f01_join.stderr"
  local f01_join_rc=$?
  set -e
  [[ "${f01_join_rc}" -ne 0 ]] || die "f01: expected join timeout without approve"
  local f01_reason
  f01_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f01_join.json" | tr -d '\r\n')"
  [[ "${f01_reason}" == "TIMEOUT" ]] || die "f01: expected reason_code=TIMEOUT, got ${f01_reason}"

  # F02: wrong approver rejection (fresh invite).
  poc_e2e_pin_broker_state "node-a"
  local invite_wrong_json_tmp invite_wrong_code
  invite_wrong_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f02_invite_wrong_approver.md invite --mode approve --uses 1 --expires 30s" \
    >"${invite_wrong_json_tmp}" 2>>"${run_dir}/steps/node-a/f02_invite_wrong_approver.stderr" || true
  invite_wrong_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite_wrong_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  rm -f "${invite_wrong_json_tmp}"
  [[ -n "${invite_wrong_code}" ]] || die "f02: failed to extract invite code"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f02_invite_wrong_approver.md" "/artifacts/reports/full/f02_invite_wrong_approver.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f02_invite_wrong_approver.md" >/dev/null 2>&1 || true

  set +e
  docker exec "${docker_network}-node-c" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f02_wrong_approve.md approve '${invite_wrong_code}'" \
    >"${run_dir}/cases/full/f02_wrong_approve.json" 2>"${run_dir}/steps/node-c/f02_wrong_approve.stderr"
  local f02_rc=$?
  set -e
  [[ "${f02_rc}" -ne 0 ]] || die "f02: expected wrong approver failure"
  local f02_reason
  f02_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f02_wrong_approve.json" | tr -d '\r\n')"
  [[ "${f02_reason}" == "FORBIDDEN" ]] || die "f02: expected reason_code=FORBIDDEN, got ${f02_reason}"

  # F03/F05 baseline membership for node-b/node-c (uses=2).
  poc_e2e_pin_broker_state "node-a"
  local invite2_json_tmp invite2_code issuer2_peer_id
  invite2_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f03_invite.md invite --mode approve --uses 2 --expires 20s" \
    >"${invite2_json_tmp}" 2>>"${run_dir}/steps/node-a/f03_invite.stderr" || true
  invite2_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite2_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  issuer2_peer_id="$(jq -r '.facts[]? | select(.message? | startswith("peer_id=")) | .message' "${invite2_json_tmp}" | sed -E 's/^peer_id=//' | head -n 1)"
  rm -f "${invite2_json_tmp}"
  [[ -n "${invite2_code}" ]] || die "f03: failed to extract invite2 code"
  [[ -n "${issuer2_peer_id}" ]] || die "f03: failed to extract issuer2 peer id"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f03_invite.md" "/artifacts/reports/full/f03_invite.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f03_invite.md" >/dev/null 2>&1 || true

  set +e
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f03_approve.md approve '${invite2_code}'" \
    >"${run_dir}/cases/full/f03_approve.json" 2>"${run_dir}/steps/node-a/f03_approve.stderr" &
  local approve2_pid=$!
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f03_join_b.md join '${invite2_code}'" \
    >"${run_dir}/cases/full/f03_join_b.json" 2>"${run_dir}/steps/node-b/f03_join_b.stderr"
  local join_b_rc=$?
  docker exec "${docker_network}-node-c" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f05_join_c.md join '${invite2_code}'" \
    >"${run_dir}/cases/full/f05_join_c.json" 2>"${run_dir}/steps/node-c/f05_join_c.stderr"
  local join_c_rc=$?
  wait "${approve2_pid}"
  local approve2_rc=$?
  set -e
  [[ "${join_c_rc}" -eq 0 ]] || die "f05: join_c failed"
  [[ "${join_b_rc}" -eq 0 ]] || die "f03: join_b failed"
  [[ "${approve2_rc}" -eq 0 ]] || die "f03: approve failed"

  # F04: invite max-uses exhaustion (third joiner).
  set +e
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f04_approve_uses_exhausted.md approve '${invite2_code}'" \
    >"${run_dir}/cases/full/f04_approve_uses_exhausted.json" 2>"${run_dir}/steps/node-a/f04_approve_uses_exhausted.stderr" &
  local approve_ex_pid=$!
  docker exec "${docker_network}-node-d" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f04_join_d_uses_exhausted.md join '${invite2_code}'" \
    >"${run_dir}/cases/full/f04_join_d_uses_exhausted.json" 2>"${run_dir}/steps/node-d/f04_join_d_uses_exhausted.stderr"
  local join_d_rc=$?
  wait "${approve_ex_pid}"
  local approve_ex_rc=$?
  set -e
  [[ "${join_d_rc}" -ne 0 ]] || die "f04: expected join_d failure after max uses"
  [[ "${approve_ex_rc}" -eq 0 ]] || die "f04: expected approve to exit OK after max uses"
  local join_d_reason
  join_d_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f04_join_d_uses_exhausted.json" | tr -d '\r\n')"
  [[ "${join_d_reason}" == "TIMEOUT" ]] || die "f04: expected join_d reason_code=TIMEOUT, got ${join_d_reason}"
  jq -r '.facts[]?.message' "${run_dir}/cases/full/f04_approve_uses_exhausted.json" | grep -q 'invite uses exhausted' || die "f04: expected approve facts to include invite uses exhausted"

  # F12: invite expiry (join starts after invite expiry).
  poc_e2e_pin_broker_state "node-a"
  local invite4_json_tmp invite4_code
  invite4_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f12_invite_expire.md invite --mode approve --uses 1 --expires 2s" \
    >"${invite4_json_tmp}" 2>>"${run_dir}/steps/node-a/f12_invite_expire.stderr" || true
  invite4_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite4_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  rm -f "${invite4_json_tmp}"
  [[ -n "${invite4_code}" ]] || die "f12: failed to extract invite code"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f12_invite_expire.md" "/artifacts/reports/full/f12_invite_expire.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f12_invite_expire.md" >/dev/null 2>&1 || true

  sleep 3

  set +e
  docker exec "${docker_network}-node-d" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f12_join_expired.md join '${invite4_code}'" \
    >"${run_dir}/cases/full/f12_join_expired.json" 2>"${run_dir}/steps/node-d/f12_join_expired.stderr"
  local f12_join_rc=$?
  set -e
  [[ "${f12_join_rc}" -ne 0 ]] || die "f12: expected join failure with expired invite"
  local f12_reason
  f12_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f12_join_expired.json" | tr -d '\r\n')"
  [[ "${f12_reason}" == "BAD_REQUEST" ]] || die "f12: expected reason_code=BAD_REQUEST, got ${f12_reason}"
  jq -r '.facts[]?.message' "${run_dir}/cases/full/f12_join_expired.json" | grep -q 'invite already expired' || die "f12: expected join facts to include invite already expired"

  # Prepare tmux session for shell tests.
  docker exec "${docker_network}-node-a" bash -lc "tmux new-session -d -s main || true; tmux send-keys -t main 'echo FULLTEST_READY' C-m"

  # F07: marker bytes reach tmux; pcap is collected by sniffer container.
  local marker="POC_E2E_FULL_MARKER_${RANDOM}"
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch-poc-e2e sh-attach --localapi unix:/run/miopunch/localapi.sock --peer-id '${issuer2_peer_id}' --target local --session main --send 'echo ${marker}\\n' --expect '${marker}' --timeout 20s" \
    >"${run_dir}/cases/full/f07_sh_attach.json" 2>"${run_dir}/steps/node-b/f07_sh_attach.stderr"

  # F06: broker outage diagnostics (stop broker, then join should fail quickly).
  poc_e2e_pin_broker_state "node-a"
  local invite_broker_json_tmp invite_broker_code
  invite_broker_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f06_invite_broker_down.md invite --mode approve --uses 1 --expires 20s" \
    >"${invite_broker_json_tmp}" 2>>"${run_dir}/steps/node-a/f06_invite_broker_down.stderr" || true
  invite_broker_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite_broker_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  rm -f "${invite_broker_json_tmp}"
  [[ -n "${invite_broker_code}" ]] || die "f06: failed to extract broker invite code"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f06_invite_broker_down.md" "/artifacts/reports/full/f06_invite_broker_down.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f06_invite_broker_down.md" >/dev/null 2>&1 || true

  docker rm -f "${broker_container}" >/dev/null 2>&1 || true
  broker_container=""

  set +e
  docker exec "${docker_network}-node-d" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f06_join_broker_down.md join '${invite_broker_code}'" \
    >"${run_dir}/cases/full/f06_join_broker_down.json" 2>"${run_dir}/steps/node-d/f06_join_broker_down.stderr"
  local f06_rc=$?
  set -e
  [[ "${f06_rc}" -ne 0 ]] || die "f06: expected join failure with broker down"
  local f06_reason
  f06_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f06_join_broker_down.json" | tr -d '\r\n')"
  [[ "${f06_reason}" == "UNAVAILABLE" ]] || die "f06: expected reason_code=UNAVAILABLE, got ${f06_reason}"
  jq -r '.facts[]?.message' "${run_dir}/cases/full/f06_join_broker_down.json" | grep -q 'mqtt connect failed' || die "f06: expected join facts to include mqtt connect failed"

  # Restart broker for remaining checks.
  poc_e2e_start_broker
  poc_e2e_wait_broker "${docker_network}-node-a"

  # F05: restart persistence (restart daemons, re-ping).
  docker exec "${docker_network}-node-a" systemctl restart miopunch
  docker exec "${docker_network}-node-b" systemctl restart miopunch
  poc_e2e_wait_daemon_active "node-a"
  poc_e2e_wait_daemon_active "node-b"
  poc_e2e_wait_localapi "node-a"
  poc_e2e_wait_localapi "node-b"

  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f05_ping_after_restart.md ping '${issuer2_peer_id}'" \
    >"${run_dir}/cases/full/f05_ping_after_restart.json" 2>"${run_dir}/steps/node-b/f05_ping_after_restart.stderr"

  # F10: non-default data proto smoke (set issuer local to kcp, re-invite/join, and assert ping facts data_proto=kcp).
  docker exec "${docker_network}-node-a" bash -lc 'jq ".local.data_proto = \"kcp\"" /var/lib/miopunch/state.json > /var/lib/miopunch/state.json.tmp && mv /var/lib/miopunch/state.json.tmp /var/lib/miopunch/state.json'
  docker exec "${docker_network}-node-a" systemctl restart miopunch
  poc_e2e_wait_daemon_active "node-a"
  poc_e2e_wait_localapi "node-a"
  local invite3_json_tmp invite3_code issuer3_peer_id
  invite3_json_tmp="$(mktemp)"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f10_invite_kcp.md invite --mode approve --uses 1 --expires 30s" \
    >"${invite3_json_tmp}" 2>>"${run_dir}/steps/node-a/f10_invite_kcp.stderr" || true
  invite3_code="$(jq -r '.facts[]? | select(.message? | startswith("invite_code=")) | .message' "${invite3_json_tmp}" | sed -E 's/^invite_code=//' | head -n 1)"
  issuer3_peer_id="$(jq -r '.facts[]? | select(.message? | startswith("peer_id=")) | .message' "${invite3_json_tmp}" | sed -E 's/^peer_id=//' | head -n 1)"
  rm -f "${invite3_json_tmp}"
  [[ -n "${invite3_code}" ]] || die "f10: failed to extract invite3 code"
  [[ -n "${issuer3_peer_id}" ]] || die "f10: failed to extract issuer3 peer id"
  poc_e2e_redact_file_in_node "node-a" "/artifacts/reports/full/f10_invite_kcp.md" "/artifacts/reports/full/f10_invite_kcp.redacted.md"
  docker exec "${docker_network}-node-a" rm -f "/artifacts/reports/full/f10_invite_kcp.md" >/dev/null 2>&1 || true

  set +e
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json approve '${invite3_code}'" \
    >/dev/null 2>&1 &
  local approve3_pid=$!
  docker exec "${docker_network}-node-d" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json join '${invite3_code}'" \
    >/dev/null 2>&1
  wait "${approve3_pid}"
  set -e

  docker exec "${docker_network}-node-d" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json ping '${issuer3_peer_id}'" \
    >"${run_dir}/cases/full/f10_ping_kcp.json" 2>"${run_dir}/steps/node-d/f10_ping_kcp.stderr" || true
  local dp
  dp="$(poc_e2e_extract_fact_value "${run_dir}/cases/full/f10_ping_kcp.json" "data_proto=")"
  [[ "${dp}" == "kcp" ]] || die "f10: expected data_proto=kcp, got ${dp}"

  docker exec "${docker_network}-node-a" bash -lc 'jq ".local.data_proto = \"quic\"" /var/lib/miopunch/state.json > /var/lib/miopunch/state.json.tmp && mv /var/lib/miopunch/state.json.tmp /var/lib/miopunch/state.json'
  docker exec "${docker_network}-node-a" systemctl restart miopunch
  poc_e2e_wait_daemon_active "node-a"
  poc_e2e_wait_localapi "node-a"

  # F09: single-writer shell attach conflict.
  local hold_marker="POC_E2E_HOLD_${RANDOM}"
  set +e
  docker exec "${docker_network}-node-b" bash -lc \
    "miopunch-poc-e2e sh-attach --localapi unix:/run/miopunch/localapi.sock --peer-id '${issuer2_peer_id}' --target local --session main --send 'echo ${hold_marker}\\n' --expect '${hold_marker}' --timeout 40s --hold 20s" \
    >"${run_dir}/cases/full/f09_hold_attach.json" 2>"${run_dir}/steps/node-b/f09_hold_attach.stderr" &
  local hold_pid=$!
  poc_e2e_wait_tmux_client "node-a" "main"
  docker exec "${docker_network}-node-c" bash -lc \
    "miopunch-poc-e2e sh-attach --localapi unix:/run/miopunch/localapi.sock --peer-id '${issuer2_peer_id}' --target local --session main --send 'echo CONFLICT\\n' --expect 'CONFLICT' --timeout 20s" \
    >"${run_dir}/cases/full/f09_conflict_attach.json" 2>"${run_dir}/steps/node-c/f09_conflict_attach.stderr"
  local conflict_rc=$?
  wait "${hold_pid}"
  set -e
  [[ "${conflict_rc}" -ne 0 ]] || die "f09: expected conflict attach failure"
  local conflict_reason
  conflict_reason="$(jq -r '.reason_code' "${run_dir}/cases/full/f09_conflict_attach.json" | tr -d '\r\n')"
  [[ "${conflict_reason}" == "SH_IN_USE" ]] || die "f09: expected reason_code=SH_IN_USE, got ${conflict_reason}"

  # F08: revoke one member does not affect the other (revoke node-b; node-c still pings).
  local node_b_peer_id
  node_b_peer_id="$(poc_e2e_extract_fact_value "${run_dir}/cases/full/f03_join_b.json" "peer_id=")"
  [[ -n "${node_b_peer_id}" ]] || die "failed to extract node-b peer id"
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f08_revoke_b.md revoke '${node_b_peer_id}' --dangerous" \
    >"${run_dir}/cases/full/f08_revoke_b.json" 2>"${run_dir}/steps/node-a/f08_revoke_b.stderr"

  docker exec "${docker_network}-node-c" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format json --report /artifacts/reports/full/f08_ping_c_after_revoke_b.md ping '${issuer2_peer_id}'" \
    >"${run_dir}/cases/full/f08_ping_c_after_revoke_b.json" 2>"${run_dir}/steps/node-c/f08_ping_c_after_revoke_b.stderr"

  # F11: report redaction smoke (dedicated invite with --redact).
  docker exec "${docker_network}-node-a" bash -lc \
    "miopunch --redact --localapi unix:/run/miopunch/localapi.sock --format human --report /artifacts/reports/full/f11_invite_redact.md invite --mode approve --uses 1 --expires 30s" \
    >/dev/null 2>"${run_dir}/steps/node-a/f11_invite_redact.stderr" || true
  docker exec "${docker_network}-node-a" bash -lc \
    'set -euo pipefail
file=/artifacts/reports/full/f11_invite_redact.md
grep -q "invite_code=<redacted>" "${file}"
if grep -q "secret_key=" "${file}"; then
  grep -q "secret_key=<redacted>" "${file}"
fi
if grep -q "net_secret_b64=" "${file}"; then
  grep -q "net_secret_b64=<redacted>" "${file}"
fi
if grep -q "invite_secret_b64=" "${file}"; then
  grep -q "invite_secret_b64=<redacted>" "${file}"
fi
'

  if [[ -n "${sniffer_container}" ]]; then
    docker kill -s INT "${sniffer_container}" >/dev/null 2>&1 || true
    sleep 0.5
  fi
  [[ -f "${run_dir}/pcap/traffic.pcap" ]] || die "missing fulltest pcap artifact: ${run_dir}/pcap/traffic.pcap"

  # F13: broker non-relay proof (marker not present in MQTT broker traffic).
  docker run --rm --network none \
    -v "${run_dir}/pcap:/artifacts" \
    "${sniffer_image}" \
    sh -lc "tcpdump -nn -A -r /artifacts/traffic.pcap 'tcp port 1883' > /artifacts/broker_1883.ascii.txt 2>/artifacts/broker_1883.ascii.stderr || true"
  docker run --rm --network none \
    -v "${run_dir}/pcap:/artifacts" \
    "${sniffer_image}" \
    sh -lc "if grep -Fq \"${marker}\" /artifacts/broker_1883.ascii.txt; then echo 'marker_found_in_broker=1' > /artifacts/broker_marker_scan.txt; exit 1; else echo 'marker_found_in_broker=0' > /artifacts/broker_marker_scan.txt; fi"

  poc_e2e_collect_node_diagnostics "node-a"
  poc_e2e_collect_node_diagnostics "node-b"
  poc_e2e_collect_node_diagnostics "node-c"
  poc_e2e_collect_node_diagnostics "node-d"
}
