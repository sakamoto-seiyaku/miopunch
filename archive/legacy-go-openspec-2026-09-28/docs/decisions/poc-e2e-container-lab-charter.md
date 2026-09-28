# POC E2E 容器实验台纲领

日期：2026-04-22

本文定义 POC 阶段常态化端到端测试实验台。它不是只描述想法的备忘录，而是后续实现 `poc-e2e-selftest` 与 `poc-e2e-fulltest` 的技术边界、任务分配和验收合同。

## 目标

建立一套可重复运行的 VM 内 e2e 测试，用多个互相隔离的 Linux 实例验证当前 POC 产品闭环：

```text
system daemon install/start
-> LocalAPI readiness
-> real MQTT broker
-> invite
-> approve + join
-> ping
-> sh ls
-> sh attach
-> revoke 后拒绝
```

测试必须经过真实进程边界、真实 system daemon、真实 LocalAPI IPC、真实 MQTT broker，以及产品 CLI/LocalAPI 路径。测试不应直接调用内部 Go API 来伪造产品行为。

## 决策摘要

- **外层隔离**：复用现有 `lab/host/labctl` 单 QEMU VM。
- **VM 准备**：沿用 `up -> wait -> wait-guest -> push-guest -> push-bin -> guest runner -> pull-artifacts`。
- **实例隔离**：VM 内使用 Docker 多容器；第一版只支持 Docker，不支持 Podman。
- **node 形态**：每个 node 容器都是完整 Linux/systemd/miopunch system daemon 实例。
- **broker**：默认使用真实 `mosquitto` MQTT server，Docker DNS endpoint 固定为 `broker:1883`。
- **P2P 基础**：第一版只要求 Docker 网络内多实例互联，不构造 NAT1/NAT2/NAT3/NAT4 矩阵。
- **`sh attach` 自动化**：第一版通过 `miopunch-lab` LocalAPI WebSocket helper 驱动，不把 CLI raw-mode/伪终端作为必过路径。
- **抓包策略**：`poc-e2e-selftest` 不强制抓包；`poc-e2e-fulltest` 必须采集 broker/network pcap，用于 broker 非 data-plane relay 证据。
- **平台范围**：只覆盖 Linux POC；Windows 节点另行规划。

## 和现有 lab 的关系

现有 lab 已提供：

- `lab/host/labctl`：host 侧 QEMU VM 生命周期、guest runtime 同步、binary build/push、artifact pull。
- `lab/guest/bin/mlab*`：VM 内 guest runtime、case runner、artifact 约定和 cleanup 模式。
- `lab/_artifacts/`：host 侧 artifact 汇总目录。

本实验台必须复用这些约定，不另建 host VM 管理入口。

新增 host 命令：

```bash
./lab/host/labctl poc-e2e-selftest
./lab/host/labctl poc-e2e-fulltest
```

两个命令的 host 流程固定为：

```text
cmd_up
-> cmd_wait
-> cmd_wait_guest
-> cmd_push_guest
-> cmd_push_bin
-> guest runner
-> cmd_pull_artifacts
```

guest runner 失败时仍必须 pull artifacts，并保留 guest runner 的最终 exit code。

## 拓扑

目标拓扑：

```text
host
└── qemu vm: miopunch-lab
    ├── docker network: miopunch-poc-e2e-<run-id>
    │   ├── broker
    │   │   └── mosquitto on broker:1883
    │   ├── node-a
    │   │   ├── systemd PID 1
    │   │   ├── miopunch system daemon
    │   │   ├── /run/miopunch/localapi.sock
    │   │   ├── /var/lib/miopunch/state.json
    │   │   ├── /var/lib/miopunch/net.json
    │   │   ├── /var/lib/miopunch/identity/identity.json
    │   │   ├── /var/lib/miopunch/governance/head_snapshot.json
    │   │   ├── /var/lib/miopunch/decls/decls.json
    │   │   └── tmux
    │   ├── node-b
    │   │   └── approved member / joiner
    │   └── node-c
    │       └── second member / wrong approver / outsider
    └── guest harness
        ├── build node image
        ├── create docker network
        ├── start broker and node containers
        ├── run product CLI commands through docker exec
        ├── drive sh_attach through LocalAPI WebSocket helper
        ├── collect artifacts
        └── cleanup containers and networks
```

QEMU VM 是实验母机。测试不得修改宿主机网络，不得要求宿主机直接运行 Docker。

## Docker 和容器模型

### Broker 容器

broker 必须是真实 MQTT server。第一版默认：

- image：固定 tag 的 `eclipse-mosquitto` 或等价 `mosquitto`
- endpoint：`broker:1883`
- artifacts：broker stdout/stderr、container inspect、必要时 fulltest pcap

不允许使用公网 broker 作为默认测试依赖。

### Node 容器

每个 node 容器必须像一台小 Linux 主机：

- systemd 是 PID 1。
- 通过 `miopunch install-system-daemon` 安装服务。
- 通过 systemd 启动 `miopunch up`。
- CLI 命令默认访问 system LocalAPI socket `/run/miopunch/localapi.sock`。
- 独立持久状态目录为 `/var/lib/miopunch`。
- 容器内安装 `tmux`，测试使用固定 session `main`。
- 容器内不使用 user-mode daemon，不依赖 `$XDG_RUNTIME_DIR`。

node image 应基于 Debian，并包含：

- `systemd`
- `tmux`
- `jq`
- `curl`
- `ca-certificates`
- `journalctl` 所需组件
- `tcpdump` / `iproute2` / `netcat-openbsd` 等诊断工具
- `/usr/local/bin/miopunch`
- `/usr/local/bin/miopunch-lab`

systemd-in-Docker 的启动参数由 harness 固定，例如 privileged、cgroup mount、tmpfs `/run` 和 `/tmp`。实现不得把这些参数留给测试调用者临时决定。

## 状态初始化

selftest 第一版必须显式 pin broker，避免 invite 使用产品默认公网 broker。

`node-a` 的 `/var/lib/miopunch/state.json` 最小状态：

```json
{
  "format": "miopunch.state.poc.v0",
  "local": {
    "mqtt_broker": "broker:1883"
  }
}
```

随后由产品命令补齐 `peer_id`、`proxy_name`、`secret_key`、`topic_prefix`、`data_proto`、`quic_cc` 等默认值。

`node-b` 和 `node-c` 初始应为空 state，由 join 流程落盘 identity、net、governance、decls 和 seed peer state。

## 测试入口

### `poc-e2e-selftest`

定位：快速必过 POC 闭环 gate。

要求：

- 运行时间应明显短于 fulltest。
- 不强制抓包。
- 每条关键产品命令都必须带 `--report`。
- 失败时 artifact 足以定位 daemon、broker、LocalAPI、control-plane 或 data-plane 阶段。

### `poc-e2e-fulltest`

定位：更慢的负例、持久化、诊断和网络证据集合。

要求：

- 覆盖 selftest 之外的负例和诊断场景。
- 必须采集 broker/network pcap。
- 必须覆盖 `--redact`。
- 必须证明 broker 不是 shell data-plane relay。

## Selftest 用例

### E2E-S01 Daemon 安装与 LocalAPI readiness

步骤：

- 启动 `node-a` 和 `node-b` systemd 容器。
- 在每个 node 内执行 `miopunch install-system-daemon`。
- 验证 `systemctl is-active miopunch`。
- 验证 `/run/miopunch/localapi.sock` 可访问。
- 执行 `miopunch --format json ls` 作为 LocalAPI smoke。

验收：

- 两个 node 的 system daemon 都处于 active。
- LocalAPI 使用 system socket。
- `/var/lib/miopunch` 独立存在。
- 没有依赖 user-mode daemon。

### E2E-S02 Broker 与 state pinning

步骤：

- 启动 `mosquitto` broker 容器。
- 在 `node-a` 写入 `local.mqtt_broker=broker:1883`。
- 从 `node-a` 执行 `miopunch --report <path> invite --mode approve --uses 1 --expires 15m`。

验收：

- broker 可从所有 node 容器访问。
- invite report/stdout 中的 invite code 使用 lab broker。
- 不依赖 `mqtt.eclipseprojects.io:1883` 或其他公网默认值。

### E2E-S03 Invite、approve、join

步骤：

- 从 `node-a invite` 输出中提取 invite code。
- 并行启动 `node-a miopunch --report <path> approve <invite_code>`。
- 在 `node-b` 执行 `miopunch --report <path> join <invite_code>`。
- 等待 approve 和 join 都完成。

验收：

- `approve` 收到 join request 后成功。
- `join` 收到 membership bundle 后成功。
- `node-b` 落盘：
  - `/var/lib/miopunch/identity/identity.json`
  - `/var/lib/miopunch/net.json`
  - `/var/lib/miopunch/governance/head_snapshot.json`
  - `/var/lib/miopunch/decls/decls.json`
  - `/var/lib/miopunch/state.json`
- reports 中有 `peer_id`、`net_id` 和 approval 结果证据。

### E2E-S04 Member ping issuer

步骤：

- 从 `node-a` report/state 中提取 issuer peer ID。
- 在 `node-b` 执行 `miopunch --report <path> ping <node-a-peer-id>`。

验收：

- task 成功。
- report 包含 `hello=ok` 或等价 hello 授权证据。
- report 包含 data-plane 事实，例如 `sid`、`data_proto` 或可用 attempt path。

### E2E-S05 Member list issuer shell sessions

步骤：

- 在 `node-a` 创建 tmux session：`main`。
- 在 `node-b` 执行 `miopunch --report <path> sh ls <node-a-peer-id> local`。

验收：

- 命令成功。
- 输出或 report 包含 `main` session。
- shell listing 前经过 hello 授权。

### E2E-S06 Member attach issuer shell

步骤：

- 在 `node-a` 的 tmux `main` session 中准备可观察环境。
- 在 `node-b` 调用 `miopunch-poc-e2e sh-attach` helper。
- helper 通过 `/run/miopunch/localapi.sock` 创建 `sh_attach` task。
- helper 使用 WebSocket subprotocol `miopunch.sh.v0` 连接 `/api/v0/tasks/{task_id}/ws`。
- helper 发送唯一 marker 命令或字节序列。
- harness 在 `node-a` tmux output 或 WebSocket 回读中验证 marker。

验收：

- attach task 成功。
- LocalAPI WebSocket 路径被实际使用。
- 测试证明交互 shell bytes 经过产品 data-plane。
- 第一版不要求通过 CLI raw-mode/伪终端驱动 `miopunch sh`。

### E2E-S07 Revoke 后拒绝 member

步骤：

- `node-a` 执行 `miopunch --report <path> revoke <node-b-peer-id> --dangerous`。
- `node-b` 再次执行 `ping` 或 shell 操作访问 `node-a`。

验收：

- revoke 生成 `revoke_member` decl。
- 后续 member 访问失败。
- 失败报告指向 hello/authorization/revoke/not-approved 口径，而不是泛化网络错误。

## Fulltest 用例

### E2E-F01 approve 缺失时 join timeout

- `node-a` 生成 invite。
- 不运行 approve。
- `node-b` 执行 join。
- join 必须 timeout 或失败，并提示需要 approver。
- `node-b` 不应落盘为已 approved membership。

### E2E-F02 错误 approver 被拒绝

- `node-a` 生成 invite。
- `node-c` 执行 `approve <invite_code>`。
- approve 必须因 issuer mismatch 或等价原因失败。
- `node-c` 不产生 approval side effect。

### E2E-F03 invite max uses 生效

- 生成 `--uses 1` invite。
- `node-b` join 成功。
- `node-c` 使用同一个 invite 再 join。
- 第二次 join 无法获得有效 membership bundle。
- approver report 体现 uses exhausted 或等价最终状态。

### E2E-F04 invite expiry 生效

- 生成短过期 invite。
- 等待 invite 过期。
- 尝试 approve/join。
- 失败报告指向 expiry，而不是泛化网络错误。

### E2E-F05 daemon 重启后 membership 保持

- 完成 `node-a` 到 `node-b` 的 membership。
- 重启两个 node 的 miopunch system daemon。
- 重新验证 LocalAPI readiness。
- 重试 ping 和 shell listing。
- identity、net、governance、decls 和 peer seed state 保持一致。

### E2E-F06 broker outage 诊断清晰

- 停止或隔离 broker。
- 执行需要 MQTT 的命令。
- 命令确定性失败。
- report 指向 broker reachability，而不是 shell/data-plane 泛化错误。

### E2E-F07 broker 不是 data-plane relay

- 在 shell attach 期间采集 broker 和 Docker network pcap。
- 发送可识别 shell marker payload。
- broker pcap 不应出现明文 shell payload。
- node/data-plane 侧应有 shell marker 对应证据。

### E2E-F08 第二 member 不受 revoke 影响

- `node-b` 和 `node-c` 都完成 approved membership。
- revoke `node-b`。
- `node-b` 后续访问失败。
- `node-c` 仍可 ping 或 shell listing `node-a`。

### E2E-F09 single-writer shell lock

- 从 `node-b` 建立一个持续 `sh_attach`。
- 再从第二 attach 尝试连接同一 target/session。
- 第二 attach 必须失败，并报告 `SH_IN_USE` 或当前 shell lock reason。

### E2E-F10 非默认 data protocol smoke

- 使用现有 product state/config knob 设置非默认 data proto。
- 完成 ping 或 shell listing。
- report 显示非默认 data proto 被使用。

### E2E-F11 report redaction

- 对 invite/join/approve/ping/sh/revoke 中的关键命令使用 `--redact --report <path>`。
- exported report 不应包含 invite code、secret key、net secret、invite secret 明文。

### E2E-F12 cleanup evidence

- run 结束后列出匹配 run-id 的 containers/networks。
- cleanup log 必须记录删除动作和最终残留。
- 正常结束和失败路径都必须执行 cleanup。

## LocalAPI WebSocket helper

第一版必须在 `miopunch-lab` 增加一个 lab-only helper，建议命令形态：

```bash
miopunch-poc-e2e sh-attach \
  --localapi unix:/run/miopunch/localapi.sock \
  --peer-id <peer_id> \
  --target local \
  --session main \
  --send <marker-command> \
  --expect <marker-output> \
  --timeout 10s \
  --json
```

要求：

- 使用 LocalAPI 创建 `kind=sh_attach` task。
- 使用 WebSocket subprotocol `miopunch.sh.v0`。
- 支持 binary payload send/read。
- 输出 JSON summary，包含 task id、peer id、session、marker、result、reason。
- 失败时非零退出，并输出足够 stderr 诊断。
- 不扩展产品 CLI；helper 属于 lab 工具。

## Artifacts

每个 run 目录必须包含：

- `run.env`
- `topology.json`
- `cleanup.log`
- 每个 case 的 step stdout/stderr/exit code
- 每个关键命令的 `--report` markdown
- parsed summary，例如 peer IDs、net ID、task IDs、reason codes
- node daemon journal
- node `/var/lib/miopunch` snapshot
- broker logs
- `docker inspect` for broker/node containers
- Docker network inspect
- image metadata
- fulltest pcap artifacts

artifact 命名必须包含 run-id 和 case-id，避免并发/重跑混淆。

## 覆盖矩阵

| POC 能力 | `poc-e2e-selftest` | `poc-e2e-fulltest` / artifacts |
| --- | --- | --- |
| Linux system daemon 安装/启动 | 必测 | restart / uninstall / cleanup |
| system LocalAPI readiness | 必测 | restart 后再次验证 |
| 真实 MQTT broker | broker 可达、invite 使用 `broker:1883` | broker outage、broker pcap |
| invite code v0 | `node-a invite` | expiry / max uses |
| approve | `node-a approve` 等待 join request | wrong approver / uses exhausted |
| join | `node-b join` 获取 membership bundle | approve 缺失 timeout |
| identity / peer_id | 落盘快照 | restart 后保持 |
| net.json / net_id / net_secret | 落盘快照 | restart 后保持 |
| governance head v1 | 落盘快照 | restart 后保持 |
| decls approve_member | 落盘快照 | 多 member 共存 |
| hello 握手 | ping/sh 前出现授权证据 | revoked / not approved / invalid cases |
| ping | member ping issuer | broker outage / data proto 变体 |
| sh ls | member list issuer tmux session | restart 后保持 |
| sh attach | LocalAPI WebSocket 自动化 bytes | CLI 伪终端可作为后续扩展 |
| single-writer lock | 不进基础闭环 | 双 attach 触发 `SH_IN_USE` |
| revoke_member | revoke 后 ping/sh 拒绝 | revoke 一个 member 不影响另一个 |
| report export | 每个关键命令 `--report` | `--redact` 脱敏 |
| broker 非 data relay | 不抓包 | 抓包证明 shell payload 不走 broker |
| artifacts | command/report/state/journal/broker logs | packet capture / inspect / cleanup evidence |

## 验证合同

OpenSpec proposal 阶段只要求：

```bash
openspec status --change poc-e2e-container-lab-charter
openspec validate poc-e2e-container-lab-charter
```

apply 实现完成后要求：

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
./lab/host/labctl poc-e2e-selftest
./lab/host/labctl poc-e2e-fulltest
```

进入 mainline 前，还必须按 `$dev` 运行现有完整 gate，包括现有 host Go checks 和现有 lab checks。

## 明确不做

第一版不做：

- Windows node。
- Podman backend。
- host Docker 直跑模式。
- NAT1/NAT2/NAT3/NAT4 矩阵。
- 公网 broker 依赖。
- HTTP/UI 面板测试。
- 把 CLI raw-mode 伪终端作为 `sh_attach` 必过路径。
