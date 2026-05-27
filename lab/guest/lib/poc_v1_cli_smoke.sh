#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
guest_root="$(cd -- "${script_dir}/.." && pwd)"

# shellcheck source=/dev/null
source "${guest_root}/lib/common.sh"

artifacts_root="${MLAB_ARTIFACTS_ROOT:-/opt/miopunch-lab/artifacts}"
bin_dir="${MLAB_MIOPUNCH_BIN_DIR:-/opt/miopunch-lab/bin}"
node_image_repo="${MIOPUNCH_POC_V1_CLI_SMOKE_NODE_IMAGE_REPO:-miopunch-poc-v1-cli-smoke-node}"
broker_url="${MIOPUNCH_POC_V1_CLI_SMOKE_BROKER_URL:-}"

run_id=""
run_dir=""
docker_network=""
node_image=""
declare -a node_containers=()

node_container() {
  local node="$1"
  printf '%s-%s' "${docker_network}" "${node}"
}

node_artifacts_dir() {
  local node="$1"
  printf '%s/nodes/%s' "${run_dir}" "${node}"
}

state_path_for() {
  local node="$1"
  printf '/var/lib/miopunch-smoke/%s/state.json' "${node}"
}

socket_path_for() {
  local node="$1"
  printf '/run/miopunch-%s.sock' "${node}"
}

daemon_log_path_for() {
  local node="$1"
  printf '/artifacts/diag/%s-daemon.log' "${node}"
}

poc_v1_cli_smoke_resolve_broker_url() {
  local value="${MIOPUNCH_POC_V1_CLI_SMOKE_BROKER_URL:-${MIOPUNCH_REMOTE_MQTT_BROKER_URL:-}}"

  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  [[ -n "${value}" ]] || die "missing broker endpoint: set MIOPUNCH_POC_V1_CLI_SMOKE_BROKER_URL"

  if [[ "${value}" != *"://"* ]]; then
    value="tcp://${value}"
  fi

  case "${value}" in
    tcp://127.0.0.1:*|tcp://localhost:*|127.0.0.1:*|localhost:*)
      die "invalid broker endpoint: ${value} (use a real remote MQTT broker)"
      ;;
  esac

  printf '%s\n' "${value}"
}

poc_v1_cli_smoke_preflight() {
  need_cmd docker
  need_cmd jq
  need_cmd sed
  need_cmd date

  broker_url="$(poc_v1_cli_smoke_resolve_broker_url)"
  [[ -x "${bin_dir}/miopunch" ]] || die "missing binary: ${bin_dir}/miopunch (run: labctl push-bin)"
}

poc_v1_cli_smoke_new_run() {
  local ts rand
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  rand="$RANDOM$RANDOM"

  run_id="${ts}-poc-v1-cli-smoke-${rand}"
  run_dir="${artifacts_root}/${run_id}"
  docker_network="miopunch-poc-v1-cli-smoke-${rand}"
  node_image="${node_image_repo}:${rand}"

  mkdir -p "${run_dir}" "${run_dir}/steps" "${run_dir}/cases" "${run_dir}/nodes"
  {
    echo "run_id=${run_id}"
    echo "broker_url=${broker_url}"
    echo "docker_network=${docker_network}"
    echo "node_image=${node_image}"
  } >"${run_dir}/run.env"
}

poc_v1_cli_smoke_collect_node() {
  local node="$1"
  local container
  container="$(node_container "${node}")"
  local out_dir
  out_dir="$(node_artifacts_dir "${node}")"

  mkdir -p "${out_dir}/diag" "${out_dir}/state"
  docker inspect "${container}" >"${run_dir}/docker.${node}.inspect.json" 2>&1 || true
  docker logs "${container}" >"${run_dir}/docker.${node}.log" 2>&1 || true
  docker exec "${container}" bash -lc '
set -euo pipefail
ps -ef > /artifacts/diag/ps.txt 2>&1 || true
ls -la /run > /artifacts/diag/run.ls.txt 2>&1 || true
ls -la /var/lib > /artifacts/diag/var_lib.ls.txt 2>&1 || true
if [[ -f "'"$(state_path_for "${node}")"'" ]]; then
  cp -f "'"$(state_path_for "${node}")"'" /artifacts/state/state.json 2>/dev/null || true
fi
runtime_root="$(dirname "'"$(state_path_for "${node}")"'")"
if [[ -f "${runtime_root}/runtime_v1.json" ]]; then
  cp -f "${runtime_root}/runtime_v1.json" /artifacts/state/runtime_v1.json 2>/dev/null || true
fi
' || true
}

poc_v1_cli_smoke_finish() {
  local rc=$?
  set +e

  if [[ -n "${docker_network}" ]]; then
    for node in node-a node-b; do
      local container
      container="$(node_container "${node}")"
      if docker inspect "${container}" >/dev/null 2>&1; then
        poc_v1_cli_smoke_collect_node "${node}" || true
      fi
    done
    docker network inspect "${docker_network}" >"${run_dir}/docker.network.inspect.json" 2>&1 || true
  fi

  local container
  for container in "${node_containers[@]}"; do
    docker rm -f "${container}" >/dev/null 2>&1 || true
  done
  if [[ -n "${docker_network}" ]]; then
    docker network rm "${docker_network}" >/dev/null 2>&1 || true
  fi
  chmod -R a+rX "${run_dir}" 2>/dev/null || true
  set -e
  exit "${rc}"
}

poc_v1_cli_smoke_build_node_image() {
  local build_ctx="${run_dir}/_image/node"
  mkdir -p "${build_ctx}/bin"
  cp "${guest_root}/poc-e2e/node/Dockerfile" "${build_ctx}/Dockerfile"
  cp "${bin_dir}/miopunch" "${build_ctx}/bin/miopunch"
  cat >"${build_ctx}/bin/miopunch-poc-e2e" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod 0755 "${build_ctx}/bin/miopunch-poc-e2e"
  docker build -t "${node_image}" "${build_ctx}" >/dev/null
}

poc_v1_cli_smoke_start_node() {
  local node="$1"
  local container
  container="$(node_container "${node}")"
  local node_dir
  node_dir="$(node_artifacts_dir "${node}")"

  mkdir -p "${node_dir}/diag" "${node_dir}/state"
  docker run -d \
    --name "${container}" \
    --hostname "${node}" \
    --network "${docker_network}" \
    --network-alias "${node}" \
    --label "miopunch.poc_v1_cli_smoke.run_id=${run_id}" \
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

poc_v1_cli_smoke_wait_socket() {
  local node="$1"
  local socket_path
  socket_path="$(socket_path_for "${node}")"
  local container
  container="$(node_container "${node}")"
  local start now
  start="$(date +%s)"

  while true; do
    if docker exec "${container}" test -S "${socket_path}" 2>/dev/null; then
      return 0
    fi
    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "localapi socket not ready: node=${node} socket=${socket_path}"
    fi
    sleep 0.2
  done
}

poc_v1_cli_smoke_wait_status() {
  local node="$1"
  local container
  container="$(node_container "${node}")"
  local socket_path
  socket_path="$(socket_path_for "${node}")"
  local probe_json
  probe_json="$(node_artifacts_dir "${node}")/diag/localapi-probe.json"
  local start now
  start="$(date +%s)"

  while true; do
    if docker exec "${container}" bash -lc \
      "miopunch --localapi 'unix:${socket_path}' --format json ls >/artifacts/diag/localapi-probe.json 2>/dev/null || true"; then
      if jq -e '.status and .reason_code and .exit_code != null' "${probe_json}" >/dev/null 2>&1; then
        return 0
      fi
    fi
    now="$(date +%s)"
    if (( now - start > 30 )); then
      die "localapi status not ready: node=${node}"
    fi
    sleep 0.2
  done
}

poc_v1_cli_smoke_start_daemon() {
  local node="$1"
  local container
  container="$(node_container "${node}")"
  local socket_path state_path daemon_log
  socket_path="$(socket_path_for "${node}")"
  state_path="$(state_path_for "${node}")"
  daemon_log="$(daemon_log_path_for "${node}")"

  docker exec "${container}" bash -lc "
set -euo pipefail
mkdir -p \$(dirname '${state_path}') /artifacts/diag
nohup miopunch up --localapi unix:${socket_path} --state_path '${state_path}' --broker '${broker_url}' >'${daemon_log}' 2>&1 &
"
}

poc_v1_cli_smoke_run_cli() {
  local node="$1" step="$2" command="$3" out_file="$4" err_file="$5"
  local container
  container="$(node_container "${node}")"
  docker exec "${container}" bash -lc "${command}" >"${out_file}" 2>"${err_file}"
}

poc_v1_cli_smoke_extract_fact_value() {
  local json_file="$1" prefix="$2"
  jq -r --arg p "${prefix}" '.facts[]? | select(.message? | startswith($p)) | .message' "${json_file}" |
    sed -E "s/^${prefix}//" | head -n 1
}

poc_v1_cli_smoke() {
  poc_v1_cli_smoke_preflight
  poc_v1_cli_smoke_new_run
  trap poc_v1_cli_smoke_finish EXIT

  mkdir -p "${run_dir}/cases/smoke" "${run_dir}/steps/node-a" "${run_dir}/steps/node-b"

  docker network create \
    --label "miopunch.poc_v1_cli_smoke.run_id=${run_id}" \
    "${docker_network}" >/dev/null
  poc_v1_cli_smoke_build_node_image
  poc_v1_cli_smoke_start_node "node-a"
  poc_v1_cli_smoke_start_node "node-b"

  poc_v1_cli_smoke_start_daemon "node-a"
  poc_v1_cli_smoke_start_daemon "node-b"
  poc_v1_cli_smoke_wait_socket "node-a"
  poc_v1_cli_smoke_wait_socket "node-b"
  poc_v1_cli_smoke_wait_status "node-a"
  poc_v1_cli_smoke_wait_status "node-b"

  local init_json="${run_dir}/cases/smoke/init_network.json"
  local invite_json="${run_dir}/cases/smoke/invite.json"
  local approve_json="${run_dir}/cases/smoke/approve.json"
  local join_json="${run_dir}/cases/smoke/join.json"
  local ls_json="${run_dir}/cases/smoke/ls.json"
  local ping_json="${run_dir}/cases/smoke/ping.json"
  local shls_json="${run_dir}/cases/smoke/sh_ls.json"

  poc_v1_cli_smoke_run_cli \
    "node-a" "init_network" \
    "miopunch --localapi unix:$(socket_path_for node-a) --redact --format json --report /artifacts/diag/init-network.md init-network" \
    "${init_json}" "${run_dir}/steps/node-a/init_network.stderr"

  local invite_tmp="${run_dir}/cases/smoke/invite.raw.json"
  poc_v1_cli_smoke_run_cli \
    "node-a" "invite" \
    "miopunch --localapi unix:$(socket_path_for node-a) --format json --report /artifacts/diag/invite.md invite --mode approve --uses 1 --expires 15m" \
    "${invite_tmp}" "${run_dir}/steps/node-a/invite.stderr"
  sed -E 's/(invite_code=)[^[:space:]]+/\1<redacted>/g; s/(secret_key=)[^[:space:]]+/\1<redacted>/g; s/(net_secret_b64=)[^[:space:]]+/\1<redacted>/g; s/(invite_secret_b64=)[^[:space:]]+/\1<redacted>/g' \
    "${invite_tmp}" >"${invite_json}"

  local invite_code issuer_peer_id
  invite_code="$(poc_v1_cli_smoke_extract_fact_value "${invite_tmp}" "invite_code=")"
  issuer_peer_id="$(poc_v1_cli_smoke_extract_fact_value "${invite_tmp}" "peer_id=")"
  [[ -n "${invite_code}" ]] || die "failed to extract invite code"
  [[ -n "${issuer_peer_id}" ]] || die "failed to extract issuer peer_id"

  set +e
  docker exec "$(node_container node-a)" bash -lc \
    "miopunch --localapi unix:$(socket_path_for node-a) --redact --format json --report /artifacts/diag/approve.md approve '${invite_code}'" \
    >"${approve_json}" 2>"${run_dir}/steps/node-a/approve.stderr" &
  local approve_pid=$!
  sleep 1
  docker exec "$(node_container node-b)" bash -lc \
    "miopunch --localapi unix:$(socket_path_for node-b) --redact --format json --report /artifacts/diag/join.md join '${invite_code}'" \
    >"${join_json}" 2>"${run_dir}/steps/node-b/join.stderr"
  local join_rc=$?
  wait "${approve_pid}"
  local approve_rc=$?
  set -e
  [[ "${join_rc}" -eq 0 ]] || die "join failed: rc=${join_rc}"
  [[ "${approve_rc}" -eq 0 ]] || die "approve failed: rc=${approve_rc}"

  poc_v1_cli_smoke_run_cli \
    "node-b" "ls" \
    "miopunch --localapi unix:$(socket_path_for node-b) --redact --format json --report /artifacts/diag/ls.md ls" \
    "${ls_json}" "${run_dir}/steps/node-b/ls.stderr"
  jq -r '.facts[]?.message' "${ls_json}" | grep -q "peer_id=${issuer_peer_id} " || die "ls did not include issuer peer_id"

  poc_v1_cli_smoke_run_cli \
    "node-b" "ping" \
    "miopunch --localapi unix:$(socket_path_for node-b) --redact --format json --report /artifacts/diag/ping.md ping '${issuer_peer_id}'" \
    "${ping_json}" "${run_dir}/steps/node-b/ping.stderr"

  docker exec "$(node_container node-a)" bash -lc \
    "tmux new-session -d -s main || true; tmux send-keys -t main 'echo POC_V1_CLI_SMOKE_READY' C-m"

  poc_v1_cli_smoke_run_cli \
    "node-b" "sh_ls" \
    "miopunch --localapi unix:$(socket_path_for node-b) --redact --format json --report /artifacts/diag/sh_ls.md sh ls '${issuer_peer_id}' local" \
    "${shls_json}" "${run_dir}/steps/node-b/sh_ls.stderr"

  local shls_reason
  shls_reason="$(jq -r '.reason_code' "${shls_json}" | tr -d '\r\n')"
  [[ "${shls_reason}" == "OK" ]] || die "sh ls failed: reason_code=${shls_reason}"
}
