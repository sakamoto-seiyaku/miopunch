## Context

当前公网实验流程依赖 `miopunch coord` 作为信令与决策中枢：两端需要先连到一台可达的公网机器，完成 hello、候选交换与 punching 行为决策，再进入 `attempt -> dataplane`。

在 `P3.5` 阶段我们希望把公网实验的前置条件降为“提供 STUN + MQTT broker”：
- STUN 继续用于候选发现（`mapped_addrs` 等）。
- MQTT 仅作为“交换信道 + 时序对齐（sync barrier）”的介质。

约束：
- 仍处于探索阶段，不做端到端加密与面向用户的兼容性保证。
- broker 仅支持配置一个（足够覆盖当前公网验证）。
- 交换信息不发明新协议，沿用当前程序已决定的结构与字段（`wire.NatHole*` / `wire.NatHoleResp`）。

## Goals / Non-Goals

**Goals:**
- `peer client|visitor` 支持 `--signaling mqtt`，不再强制依赖 `miopunch coord`。
- 提供 `--config <yaml>` 作为 peer 默认参数来源，CLI flags 覆盖 YAML。
- MQTT signaling 具备 sync barrier，避免两端启动时间不一致导致 attempt/punching 失败。
- 在 P0 NAT lab 中提供可回归入口：启动本地 MQTT broker，跑通 `core-01` 并显式校验 `transport.payload_exchanged`。

**Non-Goals:**
- 不在本阶段引入端到端加密、强认证、兼容旧版本互通。
- 不实现“传输失败后自动切换协议”或复杂协商（P3 仍是两端配置一致即可）。
- 不支持多 broker / broker failover。

## Decisions

- **MQTT 依赖选择：**使用 `github.com/256dpi/gomqtt`（client + broker 同一依赖，便于在 lab 内嵌一个最小 broker）。
- **会话标识（方案 A）：**`sid := sha256(proxy + "\\n" + secret)`（hex 截断）作为：
  - MQTT topic namespace 的 session key
  - `connectivity.Attempt` 与 punching 的 `sid`
  该阶段默认“一次只跑一个会话”，不引入额外 `--session` 输入。
- **决策位置：**visitor 端作为“leader”在拿到双方 `wire.NatHoleVisitor`/`wire.NatHoleClient` 后复用现有 coordinator 的 analysis 逻辑生成两份 `wire.NatHoleResp`，分别发布给 visitor/client。
- **Sync barrier：**两端各自收到自己的 `NatHoleResp` 后发布 `ready/<role>`；visitor 等待 `ready/client` 后发布 `start`（包含 `start_at` 时间戳），两端按同一 `start_at` 启动 `attempt`。
- **介质与消息分离：**MQTT 的连接、订阅、发布与 barrier 放在 `internal/signaling/mqtt`；消息载荷直接使用 `wire.*` 结构 JSON 序列化，避免在 P3.5 里引入新的“产品协议”。

## Risks / Trade-offs

- **[sid 稳定导致并发冲突]** 同一 `proxy+secret` 并发跑多组实验可能互相干扰 → P3.5 默认单会话；后续可引入 run nonce（仍不需要额外用户输入）。
- **[MQTT 时序/丢包]** broker/网络抖动可能造成 exchange/ready 丢失 → QoS1 + 超时重试；失败提供 stage-level 诊断。
- **[无加密/认证]** broker 上 topic 可能被旁路观察 → 本阶段使用私有 broker；P4 统一考虑加密与鉴权。

