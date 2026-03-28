## Context

`P3` 要解决的问题不是“更强的打洞”，而是把“能连上”和“传得好”拆成可独立演进的问题域。
这要求把打洞成功后的数据面能力抽成独立层，并把 `KCP / QUIC` 等传输选择从端侧主流程中收敛到统一入口。

`brutal` 的目标不是兼容完整 `Hysteria2` 产品协议，而是复用其在真实网络中验证充分的 `QUIC` 调度/拥塞控制思路。

在工程上，上游 `quic-go` 不提供可以直接插拔拥塞控制的公开接口，因此 `P3` 采用“跟随 HY2 实战实现”的策略：统一迁移到 HY2 使用的 QUIC fork，以便获得 `BBR` 与 `brutal` 的共存能力。

## Goals / Non-Goals

**Goals:**
- 引入一个新的 capability：`miopunch-dataplane`，用 spec 约束数据面能力边界与回归口径。
- QUIC 栈采用方案 A：全仓统一迁移到 HY2 最新 release 对应的 QUIC fork，并在 `miopunch` 侧钉死版本。
- 数据面对外选择面收敛为 `data-proto=kcp|quic` + `quic-cc=bbr|brutal`，默认 `bbr`。
- `P3` 阶段优先闭环：固定参数可跑通、事件可回归、分层可落位。

**Non-Goals:**
- 不实现完整 `Hysteria2` 产品协议与其认证/产品语义。
- 不在 `P3` 做参数自动探测、自动学习、或传输失败后的自动切换。
- 不在 `P3` 优化用户配置体验：暂不引入配置文件，不做复杂兼容层。

## Decisions

- **QUIC fork / versioning**
  - 采用 HY2 最新 release 对应的 QUIC fork。
  - 当前对齐目标：`apernet/hysteria` `app/v2.7.1` 使用的 `github.com/apernet/quic-go` 版本：
    - `v0.59.1-0.20260217092621-db4786c77a22`
  - `miopunch` 侧钉死该版本；后续升级由我们自己的变更驱动（或重大修复驱动）。
- **Spec ownership (kernel vs data plane)**
  - `xtcp-kernel` 仅承诺“打洞产出可用 UDP 通道 + UDP self-check”，不再承诺数据面协议选择与 `payload exchanged`。
  - `miopunch-dataplane` 作为数据面的 source of truth，负责 `kcp / quic(bbr|brutal)` 选择、握手、`payload exchanged` 证据链与统计。
- **QUIC modes**
  - `data-proto=quic` 始终表示“使用 QUIC 数据面”。
  - `quic-cc=bbr|brutal` 表示 QUIC 下的算法选择；默认 `bbr`。
  - 对外文档与 CLI 不使用 `HY2` 作为可选值；`brutal` 是对外名。
- **Brutal parameters (P3)**
  - `brutal` 需要显式配置 `up/down` 带宽上限。
  - `P3` 阶段固定参数为 `up=1, down=1`（单位与表达方式沿 HY2）；不做自动探测与用户便捷性设计。
- **Consistency rule**
  - `P3` 不考虑两端算法/参数不一致组合；两端必须一致，不一致在 `exchange` 阶段直接失败。

## Risks / Trade-offs

- **QUIC 栈迁移风险**：从上游 `quic-go` 切换到 fork 可能带来行为漂移，需要在实验台回归中显式覆盖 `control plane` 与 `data plane` 的 QUIC 路径。
- **术语风险**：`brutal` 是算法名但在 P3 会作为“数据面模式”被用户感知；需要在文档中明确“QUIC + CC”关系，避免误解为全新协议栈。
- **spec 归属风险**：现有 `xtcp-kernel` spec 已包含数据面选择要求，`P3` 需要决定保留/迁移的口径，避免能力边界长期混杂。
