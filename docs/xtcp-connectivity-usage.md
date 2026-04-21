# XTCP Connectivity (P2)

本文是 `P2` 的**使用 / 排障速查**。设计范围、边界与原则见 `docs/decisions/p2-connectivity-charter.md`。

## 行为概览（P2(v1)）

- `UDP only`：不包含 `TCP punching`，不包含 `relay/fallback`。
- `no trickle`：只交换一次候选快照，helper 结果晚到不阻塞建链。
- 固定 attempt 顺序：`IPv6 direct` → `IPv4 direct(portmap)` → `IPv4 punching(P1 kernel)`。
- `STUN` 可选：未配置时，仍允许 `IPv6/portmap` 直连成功；只有回落到 `punching` 才依赖 `mapped_addrs`。

## 常用开关（端侧）

- `--stun <addr1,addr2,...>` / `--stun-timeout <duration>`：配置 punching 所需的 STUN（未配置则 punching 会被禁用）。
- `--disable-portmap`：禁用 `UPnP / NAT-PMP` helpers。
- `--gather-timeout <duration>`：portmap helper 的 cutoff（**不 gate STUN**）。
- `--attempt-v6-timeout <duration>`：`IPv6 direct` 尝试预算。
- `--attempt-portmap-timeout <duration>`：`IPv4 direct(portmap)` 尝试预算。

## 常用命令（最小示例）

启动 coordinator：

```bash
miopunch-lab coord --listen 0.0.0.0:7000 --proto tcp
```

启动 client（A 端）：

```bash
miopunch-lab peer client \
  --coord <coord-ip:7000> \
  --control-proto tcp \
  --proxy p1 \
  --secret <secret> \
  --user a \
  --allow-users "*"
```

启动 visitor（B 端）：

```bash
miopunch-lab peer visitor \
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
miopunch-lab peer client  ... --stun <stun1,stun2> --stun-timeout 3s
miopunch-lab peer visitor ... --stun <stun1,stun2> --stun-timeout 3s
```

## P0 实验台回归（VM 内实测）

```bash
./lab/host/labctl xtcp-connectivity-selftest
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

常见排障线索：
- `gather.stun.skip`：本次会话未配置 STUN。
- `attempt.punching.disabled`：没有可用 `mapped_addrs`，因此无法回落到 punching。
