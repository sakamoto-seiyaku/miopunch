## Why

POC-06 与 POC-06.5 已经形成 `invite -> approve + join -> ping -> sh -> revoke` 产品闭环，但当前验证仍主要停留在单元测试、LocalAPI no-network integration 和底层网络 lab。缺口是：没有一个可常态运行、经过真实 VM、真实 Linux/systemd 节点、真实 MQTT broker、真实 CLI/LocalAPI 边界的多实例 e2e gate。

本 change 要把 VM 内多容器 POC e2e 实验台从纲领推进到可实现任务：apply 后应新增 `poc-e2e-selftest` 与 `poc-e2e-fulltest`，让 POC 阶段的产品闭环可以被稳定复测和复盘。

## What Changes

- 将 `docs/decisions/poc-e2e-container-lab-charter.md` 从纯纲领扩展为实现导向 charter，明确复用现有 QEMU VM/labctl 架构。
- 在 host lab 入口新增 `lab/host/labctl poc-e2e-selftest` 与 `lab/host/labctl poc-e2e-fulltest`。
- 在 guest runtime 新增 POC e2e harness，负责 Docker network、`mosquitto` broker、systemd node 容器、case 编排、artifacts 收集和 cleanup。
- 新增 Docker node image 定义：每个 node 是完整 Linux/systemd/miopunch daemon 实例，拥有独立 `/var/lib/miopunch`、`/run/miopunch/localapi.sock`、journal 与 tmux。
- 新增 LocalAPI WebSocket 自动化 helper，用于稳定驱动 `sh_attach`，避免第一版 gate 依赖 CLI raw-mode/伪终端。
- selftest 覆盖快速必过闭环；fulltest 覆盖负例、restart、diagnostics、redaction、single-writer lock、第二 member 与 broker 非 data-plane relay 证据。
- 不引入 Windows 节点、不支持 Podman、不构造 NAT1/NAT2/NAT3/NAT4 矩阵、不依赖公网 broker。

## Capabilities

### New Capabilities

- `miopunch-poc-e2e-container-lab`: VM 内 Docker 多容器 POC e2e 实验台，包括 host/guest 入口、真实 MQTT broker、systemd node、LocalAPI WS shell 自动化、selftest/fulltest 用例和 artifacts 证据。

### Modified Capabilities

- None.

## Impact

- Affected lab host runtime: `lab/host/labctl`
- Affected lab guest runtime: `lab/guest/bin/`、`lab/guest/lib/`、新增 POC e2e Docker assets / cases
- Affected Go code: 新增 repo-local helper：`tools/miopunch-poc-e2e`；必要测试随 helper 添加
- Affected docs: `docs/decisions/poc-e2e-container-lab-charter.md`、`lab/README.md`
- Affected validation: 本 change 是 code-affecting，apply 后需要 Go gates、OpenSpec validation，以及至少 `./lab/host/labctl poc-e2e-selftest`
