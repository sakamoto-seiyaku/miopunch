## Why

当前仓库的 `miopunch` 二进制仍主要承载“实验主线工具链”（`coord/peer/stun/mqtt-broker`），而 POC 阶段的目标是交付“面向用户的产品 CLI”（`join → ping → sh(tmux)`）。两者继续共用同一个二进制会导致：

- 实验与产品语义互相污染（命令/flags/help/输出契约难以收敛）；
- lab 脚本与 runbook 不清楚依赖的入口与稳定性边界；
- 后续产品化（daemon/LocalAPI/输出冻结）难以在不破坏实验台的前提下推进。

因此需要在 POC-01 先把口径与入口切开，给后续 POC-02..POC-xx 提供稳定地基。

## What Changes

- 明确 POC “可用性边界/成功标准”（无数据面中心 relay 前提下）：
  - 最小验收闭环：`join → ping → sh(tmux)`；
  - 失败时的输出下限：`stage + reason_code + facts + suggestions`；
  - 支持/不支持的网络条件、以及失败时用户动作建议（校时/换 broker/重试/换 seed）。
- 拆二进制（lab vs product）并冻结职责边界：
  - 产品二进制：`miopunch`（POC CLI；不再承载实验用 `coord/peer/stun/mqtt-broker`）。
  - 实验二进制：`miopunch-lab`（承载现有 `coord/peer/stun/mqtt-broker` 与实验/回归所需 flags）。
- 更新实验脚本与文档统一改用 `miopunch-lab`（不维持两套入口）：
  - host：`lab/host/labctl` 的 build/push 目标改为 `./cmd/miopunch-lab`；
  - guest：`lab/guest/bin/*`、runbook/reports 中引用的二进制名与路径同步调整。
- 基础门禁验证与最小回归：
  - `gofmt`、`go test ./...`、`go vet ./...`、`scripts/check_no_xtcp_imports.sh`；
  - 最小 lab 自测：确保拆分不破坏现有实验线回归入口。

## Capabilities

### New Capabilities

- `miopunch-poc-scope`: 定义 POC 阶段的“可用性边界/成功标准/失败口径”，为后续实现提供稳定验收口径。
- `miopunch-lab-product-separation`: 约束 `miopunch`（产品）与 `miopunch-lab`（实验）二进制职责边界，并要求实验脚本/文档只依赖 `miopunch-lab`。

### Modified Capabilities

- (none)

## Impact

- Affected code:
  - `cmd/miopunch`：从“实验入口”收敛为“POC 产品 CLI 入口”（本 change 以拆分与边界冻结为主）。
  - `cmd/miopunch-lab`：新增实验二进制入口，承载现有 `coord/peer/stun/mqtt-broker`。
- Affected lab/scripts/docs:
  - `lab/host/labctl`、`lab/guest/bin/*`：build/push/run 路径与二进制名调整；
  - `docs/roadmap.md` + 相关 runbook/reports：统一更新为 `miopunch-lab`。
- No punching/control-plane protocol changes expected in this change; focus is on entrypoints, naming, and acceptance scope.
