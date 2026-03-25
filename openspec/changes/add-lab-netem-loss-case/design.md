# Design: add-lab-netem-loss-case

## Summary

这个 change 只在当前实验台基线上增加派生回归，不修改 `P0` 主 NAT 矩阵。
工作分成两条线：
- `P2` 路径覆盖加严：保留 `p2-01..p2-03`，新增 `p2-04`，并把校验升级为“有序事件链 + payload 证据”。
- transport-under-loss 加严：新增一个基于 `core-01` 的高丢包派生 case，分别跑 `KCP` 和 `QUIC`，并显式验证 `payload exchanged`。

## Key Decisions

### Preserve the core NAT baseline

- `core-01..core-10` 继续作为 `P0/P1` 的 NAT 主覆盖基线。
- 这一轮不重排、不重标、不扩充 `P0` 主矩阵。
- 所有新增覆盖都通过派生 case 或派生 run variant 表达。

### Promote validation from “path hit” to “evidence chain”

- case 通过条件不能只看 visitor 进程是否退出 `0`。
- case 需要能够声明一条按顺序匹配的关键事件链。
- 对于“成功意味着数据已实际收发”的 case，必须额外要求 `stage=transport kind=ok` 的 `payload exchanged` 事件。

### Keep `P2` additions minimal

- 现有覆盖已经证明：
  - `IPv6 direct without STUN`
  - `IPv4 direct via portmap without STUN`
  - `IPv4 punching fallback`
- 本轮真正缺失的 `P2` 派生路径只有一个：
  - 已收集到 `IPv6` 候选，但 `IPv6` 不可达，最终回落到 `IPv4 direct(portmap)`
- 因此 `P2` 只新增 `p2-04-v6-fallback-direct-ipv4`。

### Use `core-01` for the first high-loss transport variant

- 高丢包 case 的重点是 transport 韧性，不是更复杂的 punching。
- `core-01` 可以最小化 NAT 噪声，让问题更容易归因到 `KCP / QUIC` 本身。
- 同一个派生 case 只需执行两次：`data=kcp` 与 `data=quic`。

### Keep `HY2` out of scope

- 这个 change 只验证当前 `KCP / QUIC` 基线在代表性高丢包下仍能传输 payload。
- `HY2` 仍属于后续 `P3` 范围，不在本 change 引入。

## Derived Case Catalog

### `p2-01-v6-direct`

Expected ordered evidence:
1. `stage=gather kind=ok name=gather.v6.result`
2. `stage=gather kind=info name=gather.stun.skip`
3. `stage=attempt kind=start name=attempt.v6.start`
4. `stage=attempt kind=ok name=attempt.v6.ok`
5. `stage=transport kind=ok msg="quic payload exchanged"`

### `p2-02-portmap-direct`

Expected ordered evidence:
1. `stage=gather kind=start name=gather.portmap.start`
2. `stage=gather name=gather.portmap.method.result` with `included_in_snapshot=true` and `count>0`
3. `stage=gather kind=info name=gather.stun.skip`
4. `stage=attempt kind=start name=attempt.v4.start`
5. `stage=attempt kind=ok name=attempt.v4.ok`
6. `stage=transport kind=ok msg="quic payload exchanged"`

### `p2-03-punching-fallback`

Expected ordered evidence:
1. `stage=gather kind=start name=gather.portmap.start`
2. `stage=gather` records helper completion without a usable direct snapshot
3. `stage=gather kind=start name=gather.stun.start`
4. `stage=attempt kind=start name=attempt.punching.start`
5. `stage=attempt kind=ok name=attempt.punching.ok`
6. `stage=transport kind=ok msg="quic payload exchanged"`

### `p2-04-v6-fallback-direct-ipv4`

Expected ordered evidence:
1. `stage=gather kind=ok name=gather.v6.result` with `count>0`
2. `stage=gather name=gather.portmap.method.result` with `included_in_snapshot=true` and `count>0`
3. `stage=attempt kind=start name=attempt.v6.start`
4. `stage=attempt kind=info name=attempt.v6.fail`
5. `stage=attempt kind=start name=attempt.v4.start`
6. `stage=attempt kind=ok name=attempt.v4.ok`
7. `stage=transport kind=ok msg="quic payload exchanged"`

### `core-01` high-loss derived transport run

Expected ordered evidence for each run:
1. `stage=attempt kind=ok name=attempt.punching.ok`
2. `stage=transport kind=ok msg="kcp payload exchanged"` or `msg="quic payload exchanged"`

## Validation Contract

- 有序事件校验必须至少支持按 `stage`、`kind`、`name|msg` 匹配。
- case 校验还应能断言必要的 `kvs` 条件，例如 `portmap` snapshot 是否真正纳入。
- 一个以“成功传输数据”为目标的 case，不能只凭退出码通过；必须看到对应的 `payload exchanged` 事件。
- 验证任务必须逐 case 检查 artifacts，而不是只看 suite 级别的 `pass/fail` 汇总。

## Out of Scope

- 为本轮新增更多 NAT 组合
- 重写 `IPv6` 地址过滤策略
- 引入 `HY2`
- 把现有 `KCP / QUIC` 重新包装成新的 `P3` 能力
