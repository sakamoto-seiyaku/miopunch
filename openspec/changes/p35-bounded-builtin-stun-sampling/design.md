## Context

当前内置 STUN 采样路径有三个关键问题：

1. `global` 与 `cn` 组按顺序串行执行，共享同一个 `stun-timeout`，前一组的慢端点会压缩后一组预算。
2. 单组内部也是串行逐个 endpoint 请求，列表一长就会把验证过的优先服务淹没在队尾。
3. 虽然目标只是拿到足够的 NAT 观测信息（至少 2 个可用 `mapped_addr`），当前实现仍会继续顺序试更多端点，浪费预算。

这会直接影响 `P3.5` 的核心目标：默认内置 STUN 路径要能支撑真实公网实验，而不仅仅是显式 `--stun`。

## Goals / Non-Goals

**Goals:**
- 让内置 `cn` / `global` STUN 采样在一个 `stun-timeout` 内更快拿到可用观测。
- 让优先级高的、已实测稳定的服务先被请求。
- 在拿到足够的 NAT 观测信息后尽早停止剩余采样。
- 不改变显式 `--stun` 的已有语义与可预测性。
- 保持使用同一个实际 punching UDP socket，确保观测端口真实可用。

**Non-Goals:**
- 不实现“全量并发 fanout”。
- 不对内置 STUN 做动态在线排序或复杂自学习评分。
- 不改变后续 `selected_view` 仲裁规则。

## Decisions

### 1) 仅对内置 STUN 走受限并发，显式 `--stun` 保持串行

显式 `--stun` 代表用户明确给出的列表，应保持当前线性语义，避免引入新的不可预期顺序。受限并发只用于默认内置 STUN 路径。

### 2) 同一 UDP socket + 单一 read loop + transaction ID 分发

不能简单开多个 goroutine 直接同时对同一个 `UDPConn` 调 `ReadFromUDP`，否则会互相抢包。正确做法是在当前 punching 用的同一个 UDP socket 上：

- 启动一个受控 read loop；
- 按 STUN transaction ID 将响应分发给对应请求；
- worker 只负责发请求和等待自己的结果或取消。

这样既保持端口真实性，又能安全实现多个 in-flight STUN 请求。

### 3) 每组固定小并发度，默认 3

内置 `cn` / `global` 每组各维护一个优先级队列，并以固定小并发度推进。初始值采用 `3`：足以覆盖多个高优先级服务，又不会把网络打得过猛。

### 4) 两组并行采样，但各自独立提前停止

`cn` 与 `global` 组应并行推进，避免当前“global 先吃掉预算”的问题。每组独立维护自己的成功数、候选数与停止条件；某一组满足停止条件后，只取消该组剩余 worker，不影响另一组。

### 5) “足够信息即停止” 以 NAT 分类最小需求为准

当前 NAT 分类至少需要两个 `mapped_addr`。因此每组停止条件定为：

- 已获得至少 `2` 个不同 `mapped_addr` 且 `ok_count >= 2`；或
- 已获得至少 `3` 个不同 `mapped_addr`。

一旦满足，就取消该组未完成请求。

### 6) 内置列表按稳定性优先排序

优先把已在真实 case 或 host 探测中稳定回包的“大厂/成熟公共服务”放前面。IP 字面量仅作为 hostname 多 A 记录受限时的补充，不抢在主 hostname 之前。

## Risks / Trade-offs

- [并发读写复杂度上升] → 使用单 read loop + transaction 分发，避免多 goroutine 抢同一 socket。
- [过早停止导致样本偏少] → 停止条件以 NAT 分类最小需求为下限，且保留 3 个 `mapped_addr` 的兜底阈值。
- [列表继续膨胀] → 通过优先级前置 + 小并发 + 早停控制成本，而不是依赖全量跑完。
- [显式/内置路径语义混淆] → 显式 `--stun` 保持原逻辑，不复用内置优先级与早停规则。
