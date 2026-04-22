## Context

现有 lab 已有一套单 QEMU VM 生命周期：`lab/host/labctl` 负责 download/init/up/wait/push-guest/push-bin/pull-artifacts，VM cloud-init 已安装 Docker，guest runtime 位于 `/opt/miopunch-lab/guest`，artifacts 统一回收到 `lab/_artifacts/`。

当前 POC 产品侧已经具备 system daemon、system LocalAPI、`invite/approve/join/ping/sh/revoke`、report export、redaction、governance/decls 和 LocalAPI WebSocket `sh_attach`。已有测试覆盖单元与 no-network integration，但没有把这些能力放进真实多实例 Linux/systemd 边界中验证。

## Goals / Non-Goals

**Goals:**

- 复用现有 QEMU VM/labctl 机制，新增 POC e2e 常态化入口。
- 在 VM 内用 Docker 多容器模拟多个隔离 Linux 实例。
- 每个 node 容器都通过 `miopunch install-system-daemon` 安装并启动 system daemon。
- selftest 覆盖 POC 基础闭环，fulltest 覆盖负例、持久化和网络证据。
- 所有 case 都输出可复盘 artifacts，失败后不依赖立即重跑才能定位。

**Non-Goals:**

- 不替换现有 NAT/netns lab，也不把 NAT1/NAT2/NAT3/NAT4 矩阵并入本套 POC e2e。
- 不支持 Windows 节点、Podman backend、host Docker 直跑模式或公网 broker。
- 不以 CLI raw-mode 伪终端作为第一版 `sh_attach` 必过路径。
- 不重构 POC 产品协议；只在必要时新增 lab/helper 代码。

## Decisions

1. **复用单 QEMU VM，新增 labctl 命令。**
   - 选择：在 `lab/host/labctl` 中增加 `poc-e2e-selftest` 与 `poc-e2e-fulltest`，流程与现有 lab 命令一致：启动 VM、等待 cloud-init、push guest、push binaries、guest run、pull artifacts。
   - 理由：保留现有隔离与 artifact 约定，不污染 host 网络/服务。
   - 备选：直接在 host Docker 跑；拒绝，因为不符合现有 lab 隔离模型。

2. **VM 内 Docker 多容器是主拓扑。**
   - 选择：guest harness 创建 run-scoped Docker network，启动 `broker`、`node-a`、`node-b`、按需 `node-c`。
   - 理由：容器天然提供独立 state、systemd、socket、journal 和依赖边界，比 netns 多进程更接近“多台机器”。
   - 备选：复用 netns/veth NAT lab；拒绝作为主线，因为它不验证 system daemon install/start。

3. **broker 第一版使用真实 `mosquitto`。**
   - 选择：broker 容器使用 `mosquitto`，Docker 网络内 endpoint 固定为 `broker:1883`，node-a 的 `/var/lib/miopunch/state.json` pin 到该 endpoint。
   - 理由：验证真实 MQTT server 行为，不依赖公网 broker。
   - 备选：`miopunch-lab mqtt-broker`；保留为 fallback，不作为默认。

4. **node image 以 systemd 为 PID 1。**
   - 选择：node image 基于 Debian，预装 systemd、tmux、jq/curl、journal 工具和诊断工具；容器用 privileged/cgroup/tmpfs 方式启动 systemd。
   - 理由：必须验证 `install-system-daemon`、system service、system LocalAPI socket 和 `/var/lib/miopunch`。
   - 备选：容器内直接运行 `miopunch up`；拒绝，因为绕过 system daemon 安装路径。

5. **`sh_attach` 使用 LocalAPI WebSocket helper 自动化。**
   - 选择：新增一个 repo-local helper（`tools/miopunch-poc-e2e`，构建后作为 `miopunch-poc-e2e` 推送进 VM/node），通过 `POST /api/v0/tasks` 创建 `sh_attach`，再用 subprotocol `miopunch.sh.v0` 打开 WebSocket，发送 marker 并验证 tmux 回显。
   - 理由：仍走产品 daemon/task/WS/data-plane 路径，但 gate 不受 TTY/raw-mode flake 影响。
   - 备选：expect/pty 驱动 `miopunch sh`；可作为后续 fulltest smoke，不作为第一版核心。

6. **selftest 快速闭环，fulltest 完整证据。**
   - 选择：selftest 不强制 pcap，只收日志/report/state；fulltest 才采集 broker/network pcap 并执行更慢负例。
   - 理由：让基础 gate 可常态运行，同时保留足够诊断深度。
   - 备选：所有 case 都抓包；拒绝，运行时间和 flake 面过大。

## Risks / Trade-offs

- [Risk] systemd-in-Docker 在 VM 内对 cgroup/privileged 依赖较强 → Mitigation：node 启动参数固定到 harness，preflight 明确检查 docker/systemd readiness。
- [Risk] `mosquitto` 镜像拉取或包版本波动 → Mitigation：固定镜像 tag，并把 broker logs/inspect 纳入 artifacts；保留 `miopunch-lab mqtt-broker` fallback 任务但不默认启用。
- [Risk] LocalAPI WS helper 新增 Go surface → Mitigation：只放在 `tools/` 下的 repo-local 工具，不扩展产品 CLI/daemon；添加 focused Go test。
- [Risk] fulltest 时间较长 → Mitigation：`poc-e2e-selftest` 是快速必过，`poc-e2e-fulltest` 作为完整/诊断 gate。
- [Risk] artifacts 可能包含 secrets → Mitigation：reports 默认通过 `--redact` 覆盖，提交/评审 artifacts 必须脱敏；fulltest 专门验证 redaction。

## Migration Plan

- 新增命令不改变现有 `selftest`、connectivity lab 或 product CLI 行为。
- apply 后先跑 `poc-e2e-selftest` 作为最小验收；fulltest 用于合入前完整验证。
- 若实现中发现产品缺陷，不在 harness 中绕过；记录失败 artifacts 后针对产品另开修复或在本 change 中补最小必要修复。

## Open Questions

- None. 第一版默认已固定：Linux-only、Docker-only、`mosquitto`、LocalAPI WebSocket shell automation、fulltest-only pcap。
