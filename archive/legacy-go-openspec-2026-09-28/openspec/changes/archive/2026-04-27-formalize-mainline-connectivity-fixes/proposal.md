## Why

MNT-01 场景一复测暴露的 F-001/F-002/F-003/F-005 已经从单点失败收敛为三类正式设计缺口：punching phase scheduler、TCP assisted candidate、peer transport session。需要先把这些结论同步到正式设计与 OpenSpec，避免后续实施 change 各自重复解释或漂移。

## What Changes

- 将 UDP/TCP punching 的执行模型正式定义为 backend-neutral、receive-first、bounded probe window 的 phase scheduler。
- 将 exchange schedule 与 punching phase schedule 分层：signaling backend 只负责 snapshot/start window，不编码 NAT role timing。
- 将 TCP 私网 listen 地址从 direct candidate 语义中剥离，正式引入 `tcp_assisted_addrs`。
- 将 dataplane 从裸 stream 模型提升为 peer transport session + generic logical stream。
- 将 F-002/F-004 的测试修正边界同步到 findings：测试修正跟随对应产品 change，不再作为独立产品设计问题。

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-punching-decision`: define backend-neutral punching phase scheduling and success-only analyzer memory.
- `miopunch-tcp-p2p-v0`: define TCP assisted/private candidates and assisted-only punching fallback.
- `miopunch-dataplane`: define peer transport sessions, logical streams, and stream/session lifecycle boundaries.
- `miopunch-mqtt-signaling`: clarify that MQTT exchange readiness does not encode punching phase ordering.

## Impact

- Affected docs:
  - `docs/decisions/door-2-tcp-punching-charter.md`
  - `docs/decisions/p3-miopunch-transport-charter.md`
  - `docs/decisions/door-3-signaling-backend-charter.md`
  - `docs/notes/mainline-network-test-findings.md`
- Affected future changes:
  - `fix-punching-phase-scheduler`
  - `align-tcp-assisted-candidates`
  - `add-peer-transport-sessions`
- Validation:
  - OpenSpec validation only; this change is docs/OpenSpec-only.
