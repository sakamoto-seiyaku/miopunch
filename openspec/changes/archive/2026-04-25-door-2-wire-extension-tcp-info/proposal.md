## Why

Door 2（TCP 打洞）要做成与 UDP 并列的直连路径，需要在“采集 → 交换 → attempt 编排”的控制面里携带 TCP 相关输入（TCP direct/mapped candidates、TCP STUN view）。当前 `internal/wire` 的 NAT-hole 消息只承载 UDP 字段，导致后续即使补齐 STUN over TCP / TCP gather，也无法把结果传到 coordinator 并回传给两端做决策。

因此本 change 先冻结并落地最小的 wire 扩展：在不引入新消息版本、不中断旧节点的前提下，让控制面能够端到端携带 TCP 信息，并由 coordinator 产出可用于后续 attempt 的 `tcp_candidate_addrs` 等派生字段。

## What Changes

- 在 NAT-hole wire 消息中新增并列的 TCP 字段（全部 optional）：
  - request（`NatHoleVisitor`/`NatHoleClient`）：`tcp_direct_addrs` / `tcp_mapped_addrs` / `tcp_stun_cn` / `tcp_stun_global`
  - response（`NatHoleResp`）：`peer_tcp_direct_addrs` / `tcp_candidate_addrs` / `tcp_selected_view` / `tcp_selected_reason`
  - 预留但本 change 不启用：`tcp_punching_enabled` / `tcp_detect_behavior` 等 attempt 行为字段
- coordinator 将请求侧 TCP 字段按规则回传/派生到响应侧（含可选的 cn/global view selection），不改变现有 UDP 行为。
- 补齐单元测试：wire roundtrip、coordinator 派生规则。

## Capabilities

### New Capabilities

- `miopunch-wire-tcp-info-v0`: 定义 NAT-hole wire 的 `tcp_*` 字段集合、校验/过滤规则以及 coordinator 的派生输出（candidate/view selection）。

### Modified Capabilities

- `miopunch-mqtt-signaling`: exchange 的 “program-defined information” 集合扩展为包含 `tcp_*` 字段（在 P3.5/MQTT signaling 下也必须可携带）。

## Impact

- Affected code:
  - `internal/wire/messages.go`（新增字段与 `TcpDetectBehavior` 类型）
  - `internal/coordinator/nathole_controller.go`（派生/回传响应字段）
- Tests:
  - `internal/wire/ctl_test.go`
  - `internal/coordinator/nathole_controller_test.go`
- Compatibility:
  - 不引入新的 message type byte；旧节点忽略未知 JSON 字段应保持兼容（但需用测试/文档明确）。

