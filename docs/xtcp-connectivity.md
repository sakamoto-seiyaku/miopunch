# XTCP Connectivity (P2)

本项目的 `P2` 目标是补齐连通性（`UDP only`），在不引入 `relay/fallback` 的前提下，优先走更直接、更可靠的路径，同时保留 `P1 xtcp-kernel` 的 `IPv4 UDP punching(mode0..4)` 作为最后兜底。

## 路径与优先级

`Attempt` 固定顺序：

1. `IPv6 direct`
2. `IPv4 direct (portmap)`（`UPnP / NAT-PMP` best-effort）
3. `IPv4 punching (P1 kernel)`

## STUN（可选）

`STUN` **不是直连路径的硬依赖**：

- 未配置 `--stun` 时：`IPv6 direct / IPv4 portmap direct` 仍允许成功。
- 只有在回落到 `IPv4 punching` 时才依赖 `STUN` 产出的 `mapped_addrs`。

典型现象（用于排障）：
- `gather.stun.skip`：本次会话未配置 STUN。
- `attempt.punching.disabled`：直连都失败且没有可用 `mapped_addrs`，因此 punching 被禁用。

## Port mapping helpers（best-effort）

默认会并发尝试 `UPnP / NAT-PMP`，其原则是：

- 不阻塞 `exchange`（no trickle candidates：只交换一次快照，晚到结果不参与本次会话）。
- 作为“额外候选 + 诊断线索”，帮助更快直连或解释失败原因。

可用开关：
- `--disable-portmap`：完全禁用 portmap helpers。
- `--gather-timeout`：portmap 的 cutoff（仅影响 helper，不 gate STUN）。
- `--attempt-portmap-timeout`：`IPv4 direct` 尝试预算。

## 常用命令（最小示例）

启动 coordinator：

```bash
miopunch coord --listen 0.0.0.0:7000 --proto tcp
```

启动 client（A 端）：

```bash
miopunch peer client \
  --coord <coord-ip:7000> \
  --control-proto tcp \
  --proxy p1 \
  --secret <secret> \
  --user a \
  --allow-users "*"
```

启动 visitor（B 端）：

```bash
miopunch peer visitor \
  --coord <coord-ip:7000> \
  --control-proto tcp \
  --proxy p1 \
  --secret <secret> \
  --user b \
  --data-proto quic \
  --payload ping
```

如果需要启用 punching（显式配置 STUN）：

```bash
miopunch peer client  ... --stun <stun1,stun2> --stun-timeout 3s
miopunch peer visitor ... --stun <stun1,stun2> --stun-timeout 3s
```

## 事件与可观测性（JSON 行）

所有组件输出都是按行 JSON（可机读）。关键字段：

- `stage`: `gather | exchange | attempt | transport | signaling`
- `kind`: `start | ok | info | fail`
- `name`: 稳定事件名（便于 grep/聚合）

常见关键事件名：

- `gather.*`: `gather.start`, `gather.v6.result`, `gather.portmap.snapshot`, `gather.stun.result`, `gather.done`
- `exchange.*`: `exchange.start`, `exchange.ok`
- `attempt.*`: `attempt.v6.start/ok/fail`, `attempt.v4.start/ok/fail`, `attempt.punching.*`, `attempt.candidate.begin/end`
- `transport.*`: `data plane start`, `quic payload exchanged`

