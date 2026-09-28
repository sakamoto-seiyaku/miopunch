# 2026-06-02 POC v1 XTCP Decision Regression

## Purpose

本文记录一次重要的 POC v1 重构偏离：`poc-v1-04-dial-punch`
原本应保留 XTCP 风格的 UDP NAT/STUN/打洞决策链路，但当前实现只剩
candidate pair 编排和简化的 mode0 punching fallback。

这不是面试口径问题，而是工程事实问题。本文用于防止后续继续把
`internal/punching.MakeHole` 误当成完整 XTCP 流程。

## Expected Direction

原始目标应包含：

- NAT/STUN 探测与候选交换。
- UDP punching path establishment。
- XTCP 风格的 NAT feature 分类与 mode0-mode4 决策。
- 根据 STUN mapped addrs / assisted addrs 生成两端 `NatHoleResp`。
- 用 `CandidatePorts`、`SendRandomPorts`、`ListenRandomPorts` 等行为参数驱动打洞。
- punching 成功后把成功的 mode/index 回写给 analyzer，用于后续评分。

也就是说，真正要保留的不是单个低层 UDP 发包函数，而是：

```text
connectivity.Gather
  -> NatHoleVisitor / NatHoleClient
  -> punchdecision.Analyze
  -> NatHoleResp(mode/index/role/ttl/ports/random/listen)
  -> connectivity.Attempt
  -> direct path first
  -> UDP punching fallback
  -> ReportDaemonUDPSuccess
```

## What Actually Happened

`poc-v1-04-dial-punch` 的 OpenSpec scope 把流程收窄成：

```text
dial_offer / dial_answer fixed body:
- dial_id
- punch_token
- candidates
- member_credential

bounded candidate-pair runtime:
- max concurrency 4
- total budget 10s
- first winner selected
```

并且文档写入了：

```text
legacy punching/connectivity only as leaf mechanics
do not carry legacy runtime/orchestration into v1
```

这个约束的本意是避免旧 `internal/task/poc_dial.go`、GUI 状态拼装、
session recipe、topology/recovery 等旧产品编排继续污染 POC v1 主线。

实际效果是：旧的 XTCP decision orchestration 也被一起排除。由于当前
`dial_offer/dial_answer` 不携带 `NatHoleVisitor` / `NatHoleClient` 所需的
mapped addrs、assisted addrs、STUN view 或 NAT analysis material，
POC v1 已经没有输入去调用完整 `punchdecision.Analyze`。

## Code Evidence

关键提交：

```text
35c5428 Add poc v1 punch and archive 04
2026-05-26 15:56:44 +0800
```

该提交首次新增 `internal/pocv1/punch`。其中
`internal/pocv1/punch/runtime.go` 的 `natHoleRespForPair()` 从第一版开始就
直接合成 mode0 风格的响应：

```text
Mode: mode0
Role: initiator ? sender : receiver
TTL: 0
SendDelayMs: 150ms / 0
CandidateAddrs: remote candidate
```

它没有调用：

```text
punchdecision.Analyze
connectivity.Gather
connectivity.Attempt
ReportDaemonUDPSuccess
```

后续提交只是沿这条简化路径继续修补：

- `12dd64c Implement headless CLI runtime and real-env candidate diagnostics`
  增加 runtime 和诊断输出，但没有恢复 XTCP decision。
- `31520b9 pocv1: handle mirrored host punch candidates`
  增加 mirrored host shortcut。
- `9beeda2 pocv1: stabilize Android WSL demo path`
  增加 `direct_ipv4` 优先，用于修复 Android/WSL 同网 host candidate
  在 simplified punching 中超时的问题。

`restore-pocv1-udp-direct-android-wsl-demo` change 明确把恢复完整
`connectivity.Gather` / `connectivity.Attempt` / STUN mapped candidates 列为
Non-Goals，因此它是 demo path 修复，不是 XTCP decision 恢复。

## Current State

本节记录的是恢复前状态。

当前 POC v1 dial/punch 实际路径是：

```text
runtime-owned UDP socket
  -> local host candidates
  -> dial_offer / dial_answer
  -> candidate pair matrix
  -> mirrored host shortcut
  -> direct_ipv4 handshake
  -> simplified mode0 NatHoleResp
  -> punching.MakeHole fallback
```

当前实现与 XTCP 的关系只剩：

- 复用了 `internal/punching.MakeHole` 这个低层 SID probing / packet mechanics。
- 保留了一些 legacy `NatHoleResp` wire 类型。

当前实现没有完整接入：

- STUN 多样本 mapped addr 采集。
- easy/hard NAT 分类。
- 端口规律分析。
- mode0-mode4 决策。
- mode2/mode4 的完整 random/listen 策略闭环。
- 成功后 mode/index analyzer scoring。

因此，当前 POC v1 不能被描述为完整 XTCP 风格 UDP 打洞实现。

## Root Cause

这是一次架构收窄时的语义丢失。

正确应该排除的是：

```text
legacy task/runtime/gui/topology/session-recipe product orchestration
```

实际一起排除的是：

```text
XTCP NAT/STUN gather + punchdecision + NatHoleResp decision orchestration
```

`leaf mechanics` 和 `decision orchestration` 没有被明确区分：

- `internal/punching.MakeHole` 是 leaf mechanics。
- `connectivity.Gather + punchdecision.Analyze + connectivity.Attempt` 才是
  XTCP 风格 UDP path establishment 的核心链路。

## Lesson

OpenSpec / charter 在重构 legacy 系统时必须显式区分：

- 哪些 legacy 是产品编排噪音，必须移出。
- 哪些 legacy 是算法核心，必须保留或重新接入。
- 哪些字段是演示证据字段，哪些字段是算法决策输入。
- 哪些能力是当前演示承诺，哪些能力只是后续 capability。

不能只写“复用 legacy leaf mechanics”。如果目标是保留 XTCP，就必须写清楚：

```text
punchdecision is core, not optional legacy runtime.
STUN/NAT analysis material must remain in the POC v1 punch exchange.
NatHoleResp behavior must come from the decision engine, not local mode0 synthesis.
```

## Follow-up Direction

如果后续要恢复原目标，应单独开 change，例如：

```text
restore-pocv1-udp-xtcp-decision
```

该 change 至少需要：

- 扩展或替换 POC v1 `dial_offer/dial_answer` body，使其能携带
  NAT/STUN decision material。
- 重新接入 `connectivity.Gather` 或等价的新 v1 gather 模块。
- 用 `punchdecision.Analyze` 生成两端 attempt-ready `NatHoleResp`。
- 让当前 `internal/pocv1/punch` runtime 消费 decision result，而不是本地合成 mode0。
- 保留 current v1 的 roster-backed identity、peer_e2e_v1、PathResult、UDP owner/demux
  和 selected-path evidence。
- 为 `direct_ipv4`、`punching_ipv4`、mode decision、hard/easy fallback 和 failure evidence
  增加 focused tests。

在恢复前，任何文档或面试材料都必须明确说明：当前 POC v1 已验证主路径是
UDP direct-first + simplified punching fallback，不是完整 XTCP decision chain。

## Resolution

`restore-pocv1-udp-xtcp-decision` 已恢复 UDP-only 版本的核心链路：

```text
runtime-owned UDP socket
  -> connectivity.Gather snapshot
  -> dial_offer / dial_answer exchange UDP snapshot
  -> punchdecision.AnalyzeWithDaemonMemory
  -> answer carries both local/remote NatHoleResp decisions
  -> connectivity.Attempt direct-first + UDP punching fallback
  -> ReportDaemonUDPSuccess for punching_ipv4
```

恢复范围仍然刻意排除旧 POC v0 的 TCP 打洞、旧 task/gui/runtime 产品编排、
以及中国大陆/非中国大陆 STUN view 特判。POC v1 配置层现在只在显式提供
STUN server 时执行普通 STUN；未配置时保守地只交换 direct/assisted snapshot，
避免重新引入 internal cn/global STUN arbitration。

`internal/punching.MakeHole` 同时恢复了 `ListenRandomPorts` 可执行行为，并用
focused test 覆盖随机监听端口成为 winner 后不能被 cleanup 误关闭的问题。
