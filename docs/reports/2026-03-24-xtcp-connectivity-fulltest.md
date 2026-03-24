# 2026-03-24 XTCP Connectivity Fulltest (P2)

本报告汇总 `add-xtcp-connectivity` change 在 `P0` 单 VM 实验台中的实测结果（含 `P0/P1/P2` 回归门禁），以及对应的可复盘产物位置。

## 环境

- Host: WSL2 (Debian 11)
- Lab: 单个 QEMU VM + VM 内 `netns/veth/iptables/tc` 拓扑
- 工具入口：`./lab/host/labctl`

## 执行命令

```bash
./lab/host/labctl selftest
./lab/host/labctl xtcp-selftest
./lab/host/labctl xtcp-connectivity-selftest
```

## 结果摘要

### P0 baseline

- `selftest`: pass=10 fail=0
- artifacts（节选）：
  - `lab/_artifacts/20260324T183105Z-core-01/`
  - `lab/_artifacts/20260324T183812Z-core-10/`

### P1 baseline（xtcp-kernel）

- `xtcp-selftest`: pass=11 fail=0
- artifacts（节选）：
  - `lab/_artifacts/20260324T183916Z-xtcp-core-01-tcp-quic/`
  - `lab/_artifacts/20260324T183926Z-xtcp-core-01-kcp-kcp/`
  - `lab/_artifacts/20260324T183944Z-xtcp-core-10-tcp-quic/`（预期诊断失败用例）

### P2 connectivity

- `xtcp-connectivity-selftest`: pass=3 fail=0
- 覆盖路径与产物目录：
  - `IPv6 direct`（STUN 未配置）：`lab/_artifacts/20260324T184001Z-xtcp-p2-01-v6-direct-tcp-quic/`
  - `IPv4 direct (portmap)`（NAT-PMP helper）：`lab/_artifacts/20260324T184003Z-xtcp-p2-02-portmap-direct-tcp-quic/`
  - `IPv4 punching`（兜底）：`lab/_artifacts/20260324T184006Z-xtcp-p2-03-punching-fallback-tcp-quic/`

## 产物说明（每个 run_dir）

每次运行都会在对应目录下生成可复盘材料（文件名可能随用例略有差异）：

- `run.env`: 用例参数与耗时统计
- `coord.log / client.log / visitor.log`: JSON 行事件流 + 错误上下文
- `wan.pcap`: WAN 段抓包
- `natA.iptables / natB.iptables`
- `natA.conntrack / natB.conntrack`
- `natA.tc / natB.tc`
- `netns.txt`: namespaces 快照

## 补充说明

- `P2` 仅做 `UDP` 连通性编排：`IPv6 direct → IPv4 direct(portmap) → IPv4 punching(P1)`，不包含 `TCP punching`、`relay/fallback`、候选增量更新（no trickle）。
- 已额外检查 VM 内没有残留 `tcpdump/mlab-natpmpd/miopunch` 背景进程，避免污染后续回归。

