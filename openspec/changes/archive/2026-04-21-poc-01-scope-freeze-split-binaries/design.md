## Context

在进入 POC 阶段前，仓库当前的 `miopunch` 二进制主要承载实验主线工具链（`coord/peer/stun/mqtt-broker`），并被 NAT lab 的脚本直接构建与分发（例如 `lab/host/labctl` 通过 `go build ./cmd/miopunch` 推送到 VM）。

与此同时，POC 侧已经在文档中冻结了产品方向（`join → ping → sh(tmux)`、LocalAPI、输出契约等；见 `docs/notes/2026-04-15-alpha-product-discussion.md` 与 `docs/notes/2026-04-20-poc-implementation-checklist.md`）。若继续复用同一个二进制入口，会导致：

- `help/flags/subcommands` 无法收敛，实验面与产品面互相污染；
- lab/runbook 很难表达“依赖的入口稳定性与兼容边界”；
- 后续实现 `miopunch up`（daemon）与 LocalAPI 时，难以避免对实验脚本造成破坏性影响。

因此本变更优先冻结“入口与职责边界”，让后续 POC-02..POC-xx 在不影响实验台回归的前提下推进。

## Goals / Non-Goals

**Goals:**
- 将实验入口与产品入口拆成两个二进制：`miopunch-lab`（实验）与 `miopunch`（产品）。
- 冻结职责边界：实验二进制承载 `coord/peer/stun/mqtt-broker`；产品二进制不再承载这些实验命令。
- 更新 lab 脚本与文档，使“实验线”只依赖 `miopunch-lab`，不维持两套入口。
- 在不改变 punching/control-plane 语义的前提下完成迁移（本 change 的重点是入口、命名与回归链路）。

**Non-Goals:**
- 不在本 change 内实现完整 POC 产品功能（`up/invite/join/approve/ping/sh` 由后续 changes 逐步落地）。
- 不重构 punching/connectivity/dataplane 的内部包边界（除非为了拆二进制必须做的最小抽取）。
- 不引入发布/安装器层面的系统级打包逻辑（仅保证 `go build` 与 lab 脚本可用）。

## Decisions

### 1) 二进制职责边界（冻结）

- `miopunch`：POC 产品 CLI 入口（后续承载 daemon/LocalAPI client 与输出契约冻结）。
- `miopunch-lab`：实验/回归入口（承载现有 `coord/peer/stun/mqtt-broker`，以及实验所需的低层 flags）。

本变更不再让产品二进制继续承载实验命令，避免后续 POC CLI 设计被历史 flags 绑架。

### 2) 实现路径：迁移现有 CLI 到 `cmd/miopunch-lab`

选择最小改动的迁移方式：

- 将现有 `cmd/miopunch/*`（实验相关 `main.go` 与子命令实现）移动/复用到 `cmd/miopunch-lab/*`，并将 usage 文案与二进制名改为 `miopunch-lab`。
- `cmd/miopunch` 在本 change 内保持“保留入口”的最小形态（例如仅输出帮助/提示），并在用户误用 `miopunch coord|peer|stun|mqtt-broker` 时给出明确指引：请改用 `miopunch-lab ...`。

备选方案（未选）：
- 保留 `miopunch` 作为实验入口、另起产品二进制名：会与 POC 路线图目标冲突（产品入口要求为 `miopunch`）。
- 为 lab 保留双入口（例如 symlink 或兼容子命令）：会形成长期兼容包袱，不利于“实验线/产品线彻底解耦”。

### 3) Lab 迁移：脚本/文档一口径切换

lab 的回归链路以脚本为事实源（`lab/host/labctl`、`lab/guest/bin/*`）。本变更要求：

- `lab/host/labctl` 的 build/push 改为构建 `./cmd/miopunch-lab`，并推送到 VM 的稳定路径（例如 `/opt/miopunch-lab/bin/miopunch-lab`）。
- `lab/guest/bin/*` 默认使用 `miopunch-lab` 路径；若提供 env var 覆盖，也应以 `miopunch-lab` 为默认值。
- runbook/reports 中引用的命令统一更新为 `miopunch-lab`。

### 4) 回归策略：行为不变、入口改变

本变更不应改变实验行为（除了二进制名/usage 文案）。验证策略：

- Host 基础门禁：`gofmt`、`go test ./...`、`go vet ./...`、`check_no_xtcp_imports`。
- Lab 最小回归：至少跑通 selftest，确保拆分不破坏实验线入口与 artifacts 产出。

## Risks / Trade-offs

- [Breaking: 习惯使用 `miopunch coord/peer/...` 的开发者会被打断] → 产品二进制提供明确错误提示与迁移指引；同时更新所有仓库内脚本/文档为 `miopunch-lab`。
- [Lab 脚本遗漏引用导致回归链断裂] → 将脚本与文档引用作为显式 checklist；在 tasks 里用 `rg` 做“无 `miopunch coord` 残留”检查。
- [二进制名变化影响 VM 侧默认路径] → 统一修改 `labctl push-bin` 与 guest 侧默认路径；不保留双入口，避免后续混淆。
- [Minor: 为保证 lab gate 稳定性做了小幅时序修正] → 在 `nat analysis unavailable` 场景下，coordinator 的 punching fallback 会启用 punching；此前对 punching sender 统一 `sleep 1s` 可能吃掉 direct attempt 预算导致 flake。实现中已将延迟收敛为“仅当 punching 是唯一可行路径（无 peer direct addrs）”时才延迟；无 wire/protocol 变更，仅为稳定性修复。

## Open Questions

- `miopunch-lab` 在 VM 内的默认路径是否固定为 `/opt/miopunch-lab/bin/miopunch-lab`，以及是否需要在 artifacts 中记录该路径用于可解释性（倾向：记录）。
- guest 侧环境变量命名是否需要从 `MLAB_MIOPUNCH_BIN` 更名为更明确的 `MLAB_MIOPUNCH_LAB_BIN`（倾向：可以保留旧变量名但更新默认值；如更名则提供兼容读取）。
