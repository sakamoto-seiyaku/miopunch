## Why

为了做公网环境下的打洞/穿透实验，我们目前需要运行 `miopunch coord` 作为中间信令通道。即便 STUN 可用，这一步仍要求“自建一台公网机器”，带来额外成本与摩擦。

`gonc` 证明了“STUN + MQTT”在真实网络中可用：STUN 用于候选发现，MQTT 作为已存在且稳定的交换信道用于信息交换与时序对齐。本变更将引入一个最小的 MQTT 信令路径，把公网实验的前置条件从“自建 coord 服务”降低到“提供 STUN + MQTT broker”。

## What Changes

- 新增 `mqtt` 信令后端：两端通过一个 MQTT broker 完成会话发现、候选交换与开始屏障（sync barrier），不再需要自建 `miopunch coord`。
- 保持“介质与消息分离”：信令传输介质（MQTT）与交换消息（沿用现有程序已决定的交换信息）解耦，便于后续替换为其他信令介质。
- 增加 `--config <yaml>`：把公网实验参数（broker/stun/超时/传输选项等）固化在 YAML 配置中，减少手敲长命令；CLI 仍可覆盖配置文件。
- 约束范围为 `P3.5`：以“联通性 + 可观测 + 可回归”为目标，不在本阶段引入端到端加密、强兼容或面向用户的 UX 打磨。

## Capabilities

### New Capabilities
- `miopunch-mqtt-signaling`: 支持以 MQTT 作为信令交换与时序对齐通道，用于公网 NAT traversal 实验（单 broker；需要 sync barrier；不依赖自建 coord）。

### Modified Capabilities
- (none)

## Impact

- CLI/配置：
  - `cmd/miopunch`：新增 `--config`（YAML），新增/扩展信令选择面以启用 MQTT 路径。
- 核心流程：
  - `internal/peer/*`：新增 MQTT 信令路径（会话发现/候选交换/同步屏障），复用现有 `connectivity`/`punching`/`dataplane`。
  - `internal/wire/*`：预计复用现有消息结构作为交换载荷；不在 proposal 阶段锁定字段细节。
- 依赖：
  - 引入 Go MQTT client 与 YAML 解析依赖（实现阶段需确认许可证兼容与最小依赖集）。
- 测试：
  - 需要在 `P0` 虚拟 NAT 实验台内增加一个“带本地 MQTT broker 的回归入口”，验证 MQTT 信令模式下仍能产出 `payload exchanged` 证据链。

