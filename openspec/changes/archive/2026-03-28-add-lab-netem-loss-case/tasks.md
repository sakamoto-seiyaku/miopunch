# Tasks

## 1. Validation hardening

- [x] 1.1 扩展实验台回归校验器，使 case 可以按 `stage/kind/name|msg` 声明并校验有序输出事件。
- [x] 1.2 为回归校验器增加 `kvs` 条件断言能力，用于校验 `portmap` snapshot 纳入情况与关键回退证据。
- [x] 1.3 为成功 case 增加显式的 `payload exchanged` 断言，避免仅凭退出码或最终 path 判定通过。
- [x] 1.4 将每个 case 的预期事件序列与校验条件固化到 runner / case 定义附近，避免“脚本行为”和“验证口径”分离。

## 2. P2 derived case coverage

- [x] 2.1 保持 `p2-01-v6-direct`、`p2-02-portmap-direct`、`p2-03-punching-fallback` 的拓扑定义不变，但升级为严格校验对象。
- [x] 2.2 新增 `p2-04-v6-fallback-direct-ipv4`：`IPv6` 候选存在但不可达，`IPv4 portmap direct` 最终成功。
- [x] 2.3 更新 `P2` 回归入口，使其运行 `p2-01..p2-04` 并使用各自的严格事件断言。

## 3. Transport-under-loss coverage

- [x] 3.1 新增一个基于 `core-01` 的高丢包派生 case，但不改变 `P0` 主 NAT 矩阵。
- [x] 3.2 更新对应回归入口，使该派生 case 以 `data=kcp` 运行并校验 `kcp payload exchanged`。
- [x] 3.3 更新对应回归入口，使该派生 case 以 `data=quic` 运行并校验 `quic payload exchanged`。
- [x] 3.4 明确保持 `HY2` 不在本 change 范围内。

## 4. Case verification tasks

- [x] 4.1 运行并验证 `p2-01-v6-direct`，确认完整事件链与 artifacts 完整性。
- [x] 4.2 运行并验证 `p2-02-portmap-direct`，确认 helper snapshot 纳入、成功路径与 payload 证据。
- [x] 4.3 运行并验证 `p2-03-punching-fallback`，确认 helper 无有效 snapshot、已回退到 punching、且 payload 成功交换。
- [x] 4.4 运行并验证 `p2-04-v6-fallback-direct-ipv4`，确认 `attempt.v6.fail -> attempt.v4.ok` 的顺序与 payload 证据。
- [x] 4.5 运行并验证 `core-01` 高丢包 `data=kcp`，确认 payload 成功交换与 artifacts 完整性。
- [x] 4.6 运行并验证 `core-01` 高丢包 `data=quic`，确认 payload 成功交换与 artifacts 完整性。
- [x] 4.7 更新回归报告，记录执行命令、逐 case artifact 目录、逐 case 结果摘要。
- [x] 4.8 运行 `openspec validate add-lab-netem-loss-case --strict --no-interactive` 并修复验证错误。

## Verification

- `./lab/host/labctl xtcp-connectivity-selftest`
- 逐 case 复跑：`p2-01..p2-04`
- 逐 case 复跑：`core-01` 高丢包 `data=kcp`
- 逐 case 复跑：`core-01` 高丢包 `data=quic`
- `openspec validate add-lab-netem-loss-case --strict --no-interactive`
