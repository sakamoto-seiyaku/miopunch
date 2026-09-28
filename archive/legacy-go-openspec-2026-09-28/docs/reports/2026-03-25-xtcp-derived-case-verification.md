# 2026-03-25 XTCP Derived Case Verification

本报告记录 `add-lab-netem-loss-case` change 在单 VM 实验台上的实现后验证结果，仅覆盖本轮新增或收紧的派生 case，不重复 `P0` 主 NAT 矩阵。

## 环境

- Host: WSL2 (Debian 11)
- Lab: 单个 QEMU VM + VM 内 `netns/veth/iptables/tc` 拓扑
- 工具入口：`./lab/host/labctl`
- 高丢包方式：`core-01-loss` 使用定向 `P2P_UDP_LOSS_PROBABILITY=0.10`，避免对 `STUN/control` 路径施加全局 `WAN netem`

## 执行命令

```bash
/usr/local/go/bin/go test ./xtcp/connectivity
./lab/check.sh

./lab/host/labctl push-guest

./lab/host/labctl guest 'sudo /opt/miopunch-lab/guest/bin/mlab case deactivate >/dev/null 2>&1 || true'
./lab/host/labctl guest 'sudo /opt/miopunch-lab/guest/bin/mlab-xtcp-connectivity-selftest'

./lab/host/labctl guest 'sudo /opt/miopunch-lab/guest/bin/mlab case deactivate >/dev/null 2>&1 || true'
./lab/host/labctl guest "sudo /opt/miopunch-lab/guest/bin/mlab-xtcp-run core-01-loss --control-proto tcp --data-proto quic --expect success --disable-portmap --expect-events-file /opt/miopunch-lab/guest/cases/expect/core-01-loss.quic.events.json --verify-artifacts"

./lab/host/labctl guest 'sudo /opt/miopunch-lab/guest/bin/mlab case deactivate >/dev/null 2>&1 || true'
./lab/host/labctl guest "sudo /opt/miopunch-lab/guest/bin/mlab-xtcp-run core-01-loss --control-proto tcp --data-proto kcp --expect success --disable-portmap --expect-events-file /opt/miopunch-lab/guest/cases/expect/core-01-loss.kcp.events.json --verify-artifacts"

./lab/host/labctl pull-artifacts
```

## 结果摘要

### Local validation

- `/usr/local/go/bin/go test ./xtcp/connectivity`: pass
- `./lab/check.sh`: pass
- `openspec validate add-lab-netem-loss-case --strict --no-interactive`: pass

### P2 connectivity regression

- `xtcp-connectivity-selftest`: `pass=4 fail=0`
- `p2-01-v6-direct`
  - event 序列：`gather.v6.result -> gather.stun.skip -> attempt.v6.start -> attempt.v6.ok -> quic payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011742Z-xtcp-p2-01-v6-direct-tcp-quic/`
- `p2-02-portmap-direct`
  - event 序列：`gather.portmap.start -> gather.stun.skip -> gather.portmap.snapshot(included=true,direct_v4>0) -> attempt.v4.start -> attempt.v4.ok -> quic payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011745Z-xtcp-p2-02-portmap-direct-tcp-quic/`
- `p2-03-punching-fallback`
  - event 序列：`gather.portmap.start -> gather.stun.start -> gather.stun.result(count>0) -> gather.portmap.snapshot(included=false,direct_v4=0) -> attempt.punching.start -> attempt.punching.ok -> quic payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011748Z-xtcp-p2-03-punching-fallback-tcp-quic/`
- `p2-04-v6-fallback-direct-ipv4`
  - event 序列：`gather.v6.result(count>0) -> gather.stun.skip -> gather.portmap.snapshot(included=true,direct_v4>0) -> attempt.v6.start -> attempt.v6.fail -> attempt.v4.start -> attempt.v4.ok -> quic payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011751Z-xtcp-p2-04-v6-fallback-direct-ipv4-tcp-quic/`

### Transport under loss

- `core-01-loss` (`quic`): pass
  - event 序列：`attempt.punching.ok -> quic payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011705Z-xtcp-core-01-loss-tcp-quic/`
- `core-01-loss` (`kcp`): pass
  - event 序列：`attempt.punching.ok -> kcp payload exchanged`
  - artifacts：`lab/_artifacts/20260325T011708Z-xtcp-core-01-loss-tcp-kcp/`

## 验证口径

- 成功用例必须同时满足：退出成功、ordered event assertions 命中、`payload exchanged` 事件存在、最小 artifact 集合完整。
- `portmap` 与回退相关 case 额外使用 `kvs` 条件断言：
  - `p2-02` 要求 `gather.portmap.snapshot` 中 `included=true` 且 `direct_v4>0`
  - `p2-03` 要求 `gather.portmap.snapshot` 中 `included=false` 且 `direct_v4=0`
  - `p2-04` 要求 `attempt.v6.fail -> attempt.v4.ok` 顺序成立

## 备注

- `HY2` 未纳入本轮 change，实现与验证范围保持在 `P2` 派生连通性 case 与 `core-01-loss` 的 `kcp/quic` 数据面回归。
- 本轮不修改原有 `P0` 主 NAT 打洞场景定义，仅新增派生 case 与严格校验能力。
