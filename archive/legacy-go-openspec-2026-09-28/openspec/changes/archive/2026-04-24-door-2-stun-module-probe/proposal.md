## Why

Door 2（TCP 打洞）需要把 STUN 能力从 “仅 UDP、仅用于 gather” 扩展为可复用组件（含 STUN over TCP），并且必须先解决一个现实问题：**我们缺少可复现证据来判断内置 STUN 端点哪些支持 TCP、哪些只支持 UDP**，否则默认列表会在 TCP 路径上产生大量无意义超时。

因此需要先做一次“STUN 模块化 + 独立 probe + 证据驱动更新内置列表”的铺垫性变更，作为后续 TCP 方向的前置依赖。

## What Changes

- 把现有 STUN（endpoint 解析/过滤、DNS 解析、request/response roundtrip、采样聚合）整理为可复用模块，并补齐 **STUN over TCP**。
- 统一 STUN endpoint 语法：
  - `host:port` 视为 **dual**（UDP/TCP 均可能可用）
  - `udp://host:port` 视为 UDP-only
  - `tcp://host:port` 视为 TCP-only
- 增加 `miopunch-lab stun probe`：可对一组 STUN 端点同时探测 UDP/TCP 可用性并输出机器可读证据（JSONL）。
- 跑 probe 产证据并据此更新 `connectivity` 内置 STUN buckets：把端点标注为 `udp://` / `tcp://` / dual，避免后续 TCP 侧默认列表“白白超时”。
- 补齐/更新单元测试以锁定 endpoint 解析、TCP STUN stream 读写、以及内置列表的 scheme 标注口径。

## Capabilities

### New Capabilities
- `miopunch-stun-probe-v0`: 提供 `miopunch-lab stun probe`（UDP/TCP STUN 探测 + 证据输出），并定义证据字段与端点分类口径。

### Modified Capabilities
- `miopunch-public-reachability`: 内置 STUN 默认列表与 STUN endpoint 解析支持 `udp://` / `tcp://` scheme；在仅 UDP 的 gather 语义下应忽略 `tcp://` 端点（反之亦然，为 Door 2 预留）。

## Impact

- Affected code:
  - 新增 `internal/stunclient`（可复用 STUN client + endpoint 解析/分类）
  - 重构 `connectivity/*` 的 STUN 相关逻辑以复用模块，并支持 scheme 标注
  - 扩展 `cmd/miopunch-lab`：增加 `stun probe` 子命令
- Affected docs/evidence:
  - 新增/更新一份可复现的 STUN probe 证据输出（用于支撑内置列表的 TCP/UDP 分类）
