# Tasks

## 1. Scaffolding

- [ ] 定义 `xtcp-connectivity` 的包边界：outer attempt policy、helper、candidate snapshot、事件模型。
- [ ] CLI 增加最小开关与预算参数（提供合理默认值；可禁用 helper 以便回归对比）。
- [ ] `STUN` 配置变为可选：未配置/不可用时，`IPv6/portmap` 直连路径仍应可成功；仅在回落到 `IPv4 punching` 时才依赖 STUN。

## 2. Candidate Snapshot Model

- [ ] 统一候选模型：`IPv6 direct`、`IPv4 portmap direct`、`IPv4 punching` 共享同一候选表示与优先级规则。
- [ ] 扩展 `control plane` 信息交换：携带候选快照与必要诊断字段（单次交换；no trickle）。
- [ ] 单元测试：候选序列化/反序列化、版本兼容与输入校验。

## 3. IPv6 Gather (Prepare/Gather)

- [ ] 收集端侧 `IPv6` 候选（最小可用规则：全局可路由优先；过滤显然不可用地址）。
- [ ] 单元测试：地址过滤与排序；观测事件完整性。

## 4. IPv4 Port Mapping Helpers (Prepare/Gather)

- [ ] best-effort 获取 `IPv4` 端口映射（`UPnP / NAT-PMP`；`PCP` deferred），并输出可机读诊断信息。
- [ ] 依赖选择：优先 `github.com/huin/goupnp`（`UPnP IGD`）+ `github.com/jackpal/go-nat-pmp`（`NAT-PMP`）；`PCP` 另开 change（证据驱动）。
- [ ] helper 不阻塞主流程：gather 阶段并发启动；exchange 阶段尽力携带；没出结果不等待。
- [ ] 单元测试：错误分类、超时、取消、诊断输出；（必要时引入可测试的 fake gateway）。

## 5. Outer Attempt Policy (Attempt)

- [ ] 实现固定尝试顺序：`IPv6` → `IPv4 portmap direct` → `IPv4 punching(mode0..4)`。
- [ ] 定义超时/预算与取消语义：每条路径有明确的 `deadline`，失败/超时必须记录原因。
- [ ] 将 `P1 xtcp punching` 作为兜底路径集成（不得改变 `xtcp/nathole` 算法行为；仅允许粘合与扩展点调整）。
- [ ] 单元测试：attempt policy 的状态机、取消传播、顺序与回退行为。

## 6. Observability

- [ ] 扩展事件模型：`gather`（含 helper 子事件）、`exchange`、`attempt`（含候选级别 begin/end）。
- [ ] 事件必须可机读并稳定：测试可断言阶段、顺序、耗时与失败原因。

## 7. Testing

- [ ] 单元测试覆盖：候选聚合、优先级、attempt policy、超时/取消、事件流完整性。
- [ ] 集成回归入口：在 `P0` 实验台中运行 `P2` 场景回归（新增 `IPv6` 可用、`portmap` 可用、回落到 `punching` 三类路径）。
- [ ] 回归约束：既有 `P1` 集成矩阵必须继续通过，并在 `P0` 实验台中复测确认结果基线不漂移。

## 8. P0 VM 实测（必须）

> 说明：本章节强调“在 VM 内的真实网络栈 + netns + iptables/tc”的实测回归，不是纯单元测试。
> 只有本章节跑通，`P2` 才算完成。

- [ ] `P0` NAT 实验台基线通过：`./lab/host/labctl selftest`。
- [ ] `P1` 基线矩阵复测通过：`./lab/host/labctl xtcp-selftest`（结果不漂移）。
- [ ] `P2` 连通性矩阵通过：提供新的回归入口（例如 `./lab/host/labctl xtcp-connectivity-selftest`），并覆盖：
  - [ ] `IPv6` 直连优先路径（成功）
  - [ ] `IPv4 portmap` 直连优先于 punching（成功）
  - [ ] helper 不可用时回落到 `IPv4 punching`（成功或按预期失败，且诊断完整）
- [ ] 产物完整：每次运行都在 `lab/_artifacts/` 下生成独立 run dir，并包含 `coord/client/visitor` 日志、关键抓包与网络状态快照、`run.env`。

## 9. Docs & Reports

- [ ] 补充最小使用文档：如何启用/禁用 helper、如何解读候选与回退事件。
- [ ] 新增报告：`docs/reports/YYYY-MM-DD-xtcp-connectivity-fulltest.md`，汇总 `P0` 实测结果与关键结论。

## 10. Validation

- [ ] 运行 `openspec validate add-xtcp-connectivity --strict --no-interactive` 并修复问题。
