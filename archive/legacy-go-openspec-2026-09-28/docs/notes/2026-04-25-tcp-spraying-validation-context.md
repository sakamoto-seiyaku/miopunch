# TCP spraying validation context snapshot (2026-04-25)

> 状态：临时上下文记录。  
> 目的：记录 `p2-05-tcp-spraying` 在 review fix 验证中暴露的新情况，避免后续上下文丢失。  
> 边界：本文不是 OpenSpec change，不提出最终方案，不代表已经决定测试重构的完整设计。

## 背景

在执行 `fix-review-design-contracts` 的验证阶段时，完整 host gates 已通过，但 `./lab/host/labctl xtcp-connectivity-selftest` 一度稳定失败在：

- case: `p2-05-tcp-spraying`
- control: `tcp`
- data: `quic`
- expected path: `punching_tcp4`
- expected result: success with `attempt.tcp_punching.ok` and payload evidence

这个失败是在 TCP 相关集成测试中暴露出来的，不是最初 Go review 中三项设计契约本身直接导致：

- `wire.Dispatcher` 运行期 handler/terminal error 契约
- `event.Emitter` 写失败可观察契约
- `sh_attach` interactive CLI remote-close 退出契约

但这个失败也不能简单归类为“测试偶发”。它说明 TCP 打通能力加入后，现有 lab/runtime 验证体系还没有被完整重新组织。

## 观察到的现象

`p2-05-tcp-spraying` 的失败形态是：

- visitor 和 client 都完成 gather/exchange。
- visitor 进入 TCP punching sender 角色，带 `send_delay_ms=3000` 和 `send_random_ports=128`。
- client 进入 TCP punching receiver 角色，带 `listen_random_ports=32`。
- 两端最终没有产生 `attempt.tcp_punching.ok`，visitor 在总预算窗口内超时。

PCAP/conntrack 观察显示：

- 原实现更接近“一轮 SYN 尝试”。
- receiver 过早发出 SYN 后收到 RST。
- sender 延迟后才开始发包时，前面依赖的 NAT/conntrack 状态已经可能被 RST 清掉。
- 两端发包相位容易错开，导致持续失败。

另外，lab profile 也暴露出不一致：

- `nat4-irregular` 注释描述为：TCP punching port `45100` 应可达。
- 但 profile 里只有 SNAT 规则，没有对应的 TCP DNAT/forward 规则。
- 因此测试环境“文字描述的 NAT 模型”和“实际 iptables 行为”不一致。

## 当时采取的临时修复

为了让当前 review fix 能完成验证，临时修复了三个点：

- TCP punching 在预算窗口内限速重试，而不是只投递一轮 dial jobs。
- coordinator 对 TCP sender/receiver 的发包延迟做了对齐，避免 receiver 过早消耗窗口。
- `nat4-irregular` 补齐 TCP `45100` 的 DNAT/forward，使 profile 行为符合注释。

这些修复让以下验证通过：

- 单独 `p2-05-tcp-spraying`
- `xtcp-connectivity-selftest`
- `xtcp-fulltest`

## 当前判断

这个问题本质上更像一个“TCP 打通后测试体系/验证体系没有重新收敛”的问题，而不是单独某一行代码的普通 bug。

它同时跨越三层：

1. **TCP punching runtime 语义**
   - TCP simultaneous-open / spraying 不应只定义为一轮 SYN。
   - 更合理的语义是：在总预算窗口内，按照受控并发和节流策略持续尝试，直到成功或预算耗尽。

2. **coordinator 行为下发语义**
   - sender/receiver 的 `SendDelayMs`、`ReadTimeoutMs`、spraying 参数需要形成明确配套关系。
   - 不能只让 sender 延迟，而不定义 receiver 的有效发包窗口如何与 sender 重叠。

3. **lab NAT profile 与验收语义**
   - profile 注释、iptables 实现、expect events 必须一致。
   - 测试应该验证当前主线 build 出来的正式二进制，而不是依赖“某个历史 lab 假设”继续成立。

## 不立即创建 change 的原因

当前不建议马上创建新的 OpenSpec change，因为上下文还不够完整。

更准确的下一步应该是先把测试环境和验证模型整体重新梳理：

- 现有 `lab` / `xtcp-*` / connectivity selftest / fulltest 各自到底覆盖什么。
- 哪些 case 是 release-blocking，哪些只是 diagnostic/allowed-fail。
- TCP direct、TCP punching、TCP spraying、UDP punching、dataplane payload evidence 分别如何验收。
- NAT profile 是否需要自校验，确保注释、iptables、conntrack 预期一致。
- 主线验证是否应该统一为“从当前代码 build 正式 binary → 推入 lab → 跑完整验证 → 拉 artifact”。

也就是说，这不是简单补一个 `fix-p2-05`，而是需要先重新定义“加上 TCP 打通之后，我们正式主线应该怎么测试”。

## 后续讨论方向

后续可以围绕以下问题继续讨论，再决定是否创建 change：

- 是否保留当前 `labctl selftest / xtcp-selftest / xtcp-connectivity-selftest / xtcp-fulltest` 的分层，还是重组为新的主线验证入口。
- TCP spraying 的 runtime contract 是否需要写入主 spec：预算窗口、重试节奏、并发上限、sender/receiver delay 对齐。
- NAT profile 是否要新增 profile-level selftest，例如检查某个注释承诺的端口确实存在 DNAT/forward。
- artifact 是否必须记录 build source、binary hash、case matrix、expect mode，方便定位“测试环境问题”还是“代码问题”。
- 是否应该把 `p2-05` 从“单个 case 修复”上升为“TCP punching/spraying 验证基准”的一部分。

## 当前临时结论

先不创建新的 change。

先把当前情况记录下来，并把后续目标定义为：

> 在 TCP 打通能力加入之后，重新组织主线测试环境和验证矩阵；等测试重组目标清楚后，再创建对应的 OpenSpec change。

