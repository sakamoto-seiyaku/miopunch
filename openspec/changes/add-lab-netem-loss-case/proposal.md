# Proposal: add-lab-netem-loss-case

## Why

当前实验台已经有 `P2` 历史回归结果，但这轮仍有三个缺口：
- `P2` 只覆盖了 `p2-01..p2-03`，还缺少“`IPv6` 候选存在但不可达，随后回落到 `IPv4 direct`”这个双栈回退场景。
- 现有 runner 对 case 的判定仍偏宽松，更多是看退出码或最终命中的成功路径，还没有把“有序事件链 + payload exchanged 证据”作为硬性通过条件。
- `KCP / QUIC` 在高丢包条件下的派生回归还没有正式纳入 change；这一轮只需要证明当前基线在代表性高丢包场景下仍能完成数据传输，`HY2` 仍然留在后续 `P3`。

同时，这一轮不应该改动 `P0` 既有 NAT 主覆盖集；新增覆盖必须以派生 case / 派生 run variant 的形式表达。

## What Changes

- 保持 `P0` 的 `core-01..core-10` 主覆盖集不变，只新增本轮需要的派生回归 case。
- 强化实验台回归校验：允许每个 case 声明有序的机读事件链，并要求成功 case 同时提供 `payload exchanged` 证据。
- 保留现有 `P2` 三个代表 case（`p2-01`、`p2-02`、`p2-03`），但把它们提升为严格校验对象。
- 新增一个 `P2` 派生 case：`p2-04-v6-fallback-direct-ipv4`。
- 新增一个基于 `core-01` 的高丢包派生 case，并分别以 `data=kcp`、`data=quic` 运行。
- 为每一个 case 明确对应的验证任务：逐 case 运行、逐 case 校验输出、逐 case 检查 artifacts，并更新回归报告。

## Inputs / References

- `docs/roadmap.md`
- `docs/decisions/p0-nat-lab-charter.md`
- `docs/decisions/p1-xtcp-kernel-charter.md`
- `docs/decisions/p2-connectivity-charter.md`
- `docs/reports/2026-03-24-xtcp-connectivity-fulltest.md`

## Impact

- Affected specs: `nat-lab-testbed`, `xtcp-connectivity`, `xtcp-kernel`
- Affected code: `lab/guest/cases/`, `lab/guest/bin/mlab-xtcp-run`, `lab/guest/bin/mlab-xtcp-connectivity-selftest`, `lab/guest/bin/mlab-xtcp-selftest`, `lab/guest/lib/`, 回归报告文档
- Out of scope: 修改 `P0` 主 NAT 矩阵、扩展新的 NAT 组合、引入 `HY2`、重新定义现有 `KCP / QUIC` 能力归属
