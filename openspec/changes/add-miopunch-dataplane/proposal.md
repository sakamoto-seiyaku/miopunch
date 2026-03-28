## Why

`P3` 的主线目标之一是把“打洞成功”与“后续数据传输”解耦，并引入来自 `HY2` 的 `brutal` 调度/拥塞控制思路。

当前代码中 `KCP / QUIC` 数据面与端侧流程耦合较深，且使用的上游 `quic-go` 不提供可直接插拔的拥塞控制接口，导致“仅靠配置引入 brutal”不可行。
因此 `P3` 需要先把数据面能力独立成一个可单独约束的 capability，并将 QUIC 栈统一迁移到 `HY2` 实战使用的 QUIC fork，以便在同一栈内同时支持默认算法（`BBR`）与 `brutal`。

## What Changes

- 新增一个 capability：`miopunch-dataplane`，定义并约束 `P3` 阶段的“打洞成功后的数据面”能力边界与回归口径。
- `P3(v1)` 中数据面的对外选择面收敛为：
  - `--data-proto kcp|quic`
  - `--quic-cc bbr|brutal`（仅在 `data-proto=quic` 时生效；默认 `bbr`）
- 采用方案 A：全仓 QUIC 统一迁移到 `HY2` 最新 release 对应的 QUIC fork，并在 `miopunch` 侧钉死版本（除非我们主动升级或追随重大修复）。
- `P3` 暂不做配置文件与用户便利性：先在固定参数下跑通闭环并建立回归，再迭代表达方式。

## Capabilities

### New Capabilities
- `miopunch-dataplane`: 打洞成功后的数据面会话建立、传输模式选择（`KCP` / `QUIC(BBR|brutal)`）、以及数据面可观测性与回归口径。

### Modified Capabilities
- `xtcp-kernel`: 仅承诺“打洞产出可用 UDP 通道 + UDP 自体确认（self-check）”；所有 `KCP / QUIC(BBR|brutal)` 数据面选择与 `payload exchanged` 验收由 `miopunch-dataplane` 承诺与约束。

## Impact

- Affected specs:
  - Add: `miopunch-dataplane`
  - Modify: `xtcp-kernel`（数据面归属与边界调整）
- Affected code (planned in P3 implementation, not in this explore stage):
  - QUIC library dependency (switch to HY2 fork; pinned to HY2 latest release)
  - `cmd/miopunch` / lab runner: add `--quic-cc` and wire through to both peers
  - data plane implementation: add QUIC CC selection and brutal parameters (fixed `up/down` in P3)
