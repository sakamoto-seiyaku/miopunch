# 主线网络测试发现问题记录

日期：2026-04-25

本文用于记录实现或运行主线网络测试时发现的项目代码问题。这里记录的问题不是测试设计本身的问题，也不在测试重构过程中顺手修复；后续按本清单单独排期、定位和修复。

## 记录原则

- 只记录项目实现、产品行为、诊断字段、状态落盘、恢复语义或权限边界中的问题。
- 不记录测试夹具、runner、环境依赖或用例编排本身的问题；这些应直接在对应测试变更中修正。
- 每条问题必须包含可复现条件、期望行为、实际行为、证据位置和建议后续动作。
- 能归类为场景 1/2/3 的问题，应标注对应场景和 case。
- 不在本文件中隐藏失败；若问题影响 required gate，应明确标注阻塞级别。

## 问题模板

### F-000：标题

- 场景：场景 1 / 场景 2 / 场景 3
- 影响：blocker / high / medium / low
- 状态：open / investigating / fixed / wontfix
- 复现条件：
- 期望行为：
- 实际行为：
- 证据：
- 初步判断：
- 后续动作：

## 最新复测摘要（2026-04-26）

- `./lab/host/labctl xtcp-connectivity-selftest`：通过，`pass=6 fail=0`；`p2-05-tcp-spraying` 当前可复现为成功，包含 `attempt.tcp_punching.ok` 与 payload 证据。证据：`lab/_artifacts/20260426T065218Z-xtcp-p2-05-tcp-spraying-tcp-quic-bbr/visitor.log`。
- `./lab/host/labctl mnt01-selftest`：失败，`pass=21 fail=1`；UDP NAT2/NAT3/NAT4-regular 相关失败仍存在，TCP4 direct/portmap 本轮未复现失败，`mnt01-self-ipv6-udp4-fallback` 已确认为测试用例初始条件与 `direct_ipv4` 期望不匹配。证据：`lab/_artifacts/20260426T065238Z-mnt01-selftest-aggregate/summary.json`。
- `./lab/host/labctl mnt01-smoke`：通过，`pass=8 fail=0`；KCP transport 仍以 `diag-fail-allowed` 形式复现为 `hello=ok` 后读取 ping response 超时。证据：`lab/_artifacts/20260426T065845Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`。

## Open Findings

### F-001：MNT-01 真实双节点 UDP NAT2/NAT3 组合未稳定打通

- 场景：场景 1 / MNT-01
- 影响：high
- 状态：open
- 复现条件：`./lab/host/labctl mnt01-selftest`，真实 `miopunch up` 双节点、MQTT-only、fixture 使用 per-peer SID；代表失败 case 包括 `mnt01-udp-udp-nat2-x-udp-nat1`、`mnt01-udp-udp-nat3-x-udp-nat1`、`mnt01-udp-udp-nat2-x-udp-nat2`、`mnt01-udp-udp-nat3-x-udp-nat2`、`mnt01-udp-udp-nat3-x-udp-nat3`。
- 期望行为：NAT1/NAT2/NAT3 代表 UDP punching case 能证明 payload exchange，或输出更细的稳定诊断。
- 实际行为：case 进入 `PunchAttempt` 后超时，例如 `wait detect message error: read udp4 0.0.0.0:5001: i/o timeout`。
- 证据：`lab/_artifacts/20260426T045623Z-mnt01-mnt01-udp-udp-nat2-x-udp-nat1/attempt-1.md` 及同轮 MNT-01 selftest artifacts；2026-04-26 复测仍可复现，代表证据为 `lab/_artifacts/20260426T065245Z-mnt01-mnt01-udp-udp-nat2-x-udp-nat1/attempt-1.md`、`lab/_artifacts/20260426T065257Z-mnt01-mnt01-udp-udp-nat3-x-udp-nat1/attempt-1.md`、`lab/_artifacts/20260426T065424Z-mnt01-mnt01-udp-udp-nat3-x-udp-nat3/attempt-1.md`。
- 初步判断：旧 smoke/selftest 里共享 SID 会发生本机 acceptor self-pairing；改为 per-peer SID 后暴露真实跨节点 UDP 打洞缺口。
- 后续动作：单独创建产品 connectivity fix change，定位 candidate selection、detect message path、NAT2/NAT3 端口过滤与 bounded retry 诊断。

### F-002：MNT-01 真实双节点 TCP4 direct/portmap direct 未稳定打通

- 场景：场景 1 / MNT-01
- 影响：high
- 状态：investigating
- 复现条件：`./lab/host/labctl mnt01-selftest`，代表失败 case 包括 `mnt01-self-tcp4-direct` 和 `mnt01-self-tcp-portmap`。
- 期望行为：TCP4 direct 或 NAT-PMP portmap direct 能证明 payload exchange，或输出明确 selected/failed path 诊断。
- 实际行为：历史运行中 case 完成 MQTT candidate exchange 后进入 `PunchAttempt`，最终 `context deadline exceeded`；2026-04-26 复测中 `mnt01-self-tcp4-direct` 与 `mnt01-self-tcp-portmap` 均已证明 payload exchange，`attempt_path=punching_tcp4`。
- 证据：历史证据为 `lab/_artifacts/20260426T045921Z-mnt01-mnt01-self-tcp4-direct/attempt-1.md`、`lab/_artifacts/20260426T045939Z-mnt01-mnt01-self-tcp-portmap/attempt-1.md`；最新复测证据为 `lab/_artifacts/20260426T065541Z-mnt01-mnt01-self-tcp4-direct/attempt-1.md`、`lab/_artifacts/20260426T065553Z-mnt01-mnt01-self-tcp-portmap/attempt-1.md`。
- 初步判断：原失败当前未复现；需要确认是近期 runtime/fixture 改动已修复，还是该路径仍存在稳定性波动。
- 后续动作：连续复测 TCP4 direct/portmap；若保持稳定通过，可降级或关闭该 finding，否则单独拆分 TCP4 direct/portmap 产品诊断与修复。

### F-003：MNT-01 KCP transport specialty 已建立路径但 ping response 超时

- 场景：场景 1 / MNT-01
- 影响：medium
- 状态：open
- 复现条件：`./lab/host/labctl mnt01-smoke`，case `mnt01-smoke-kcp-transport`，真实 `miopunch up` 双节点、MQTT-only、`data_proto=kcp`、UDP direct/punching 路径。
- 期望行为：KCP transport variant 在已完成 candidate exchange、`PunchAttempt` 和 hello handshake 后能证明 `ping=ok` payload exchange。
- 实际行为：case 能进入 `PunchAttempt`，建立 `attempt_path=punching_ipv4`，并完成 `hello=ok`，但读取 ping response 超时；MNT-01 当前将该 specialty 作为 `diag-fail-allowed`，要求完整诊断证据而非静默通过。
- 证据：`lab/_artifacts/20260426T062455Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`；2026-04-26 复测仍可复现，证据为 `lab/_artifacts/20260426T065845Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`。
- 初步判断：KCP stream transport 的 payload response 路径仍需单独定位；本 change 只确保超时能按 task deadline 生成报告和停止条件。
- 后续动作：单独创建 KCP dataplane/shell handshake fix change，定位 KCP stream response、deadline 和 close 语义。

### F-004：MNT-01 IPv6 到 UDP4 fallback 成功但 attempt_path 与验收期望不一致

- 场景：场景 1 / MNT-01 / `mnt01-self-ipv6-udp4-fallback`
- 影响：high
- 状态：wontfix（产品侧；转测试用例修正）
- 复现条件：`./lab/host/labctl mnt01-selftest`，case 使用 `--enable-ipv6 --block-forward-udp6 --expect-path direct_ipv4`。
- 期望行为：IPv6 forward UDP 被阻断后，UDP4 fallback 应证明 payload exchange，并给出与验收一致的 IPv4 fallback path 诊断。
- 实际行为：`miopunch ping` 返回成功，`hello=ok` 且 `ping=ok`，但 report 中为 `attempt_path=punching_ipv4`；runner 随后因缺少 `attempt_path=direct_ipv4` 判定该 required case 失败，使 `mnt01-selftest` 汇总为 `pass=21 fail=1`。
- 证据：`lab/_artifacts/20260426T065629Z-mnt01-mnt01-self-ipv6-udp4-fallback/attempt-1.md`、`lab/_artifacts/20260426T065238Z-mnt01-selftest-aggregate/summary.json`。
- 初步判断：这不是产品打洞失败。按当前 P2 设计，`direct_ipv4` 表示 IPv4 portmap direct candidate 成功；STUN 得到的 mapped address 属于 UDP punching candidate，成功后应记录为 `punching_ipv4`。本 case 期望 `direct_ipv4`，但未启动 NAT-PMP/portmap helper，且 STUN 仍启用，因此实际落到 `punching_ipv4` 并成功是符合当前产品语义的。
- 后续动作：在测试重组中修正该 case 的初始条件：若目标是验证“IPv6 不通后回落到 IPv4 portmap direct”，应启动 NAT-PMP helper，并建议禁用 STUN 以避免 punching 路径干扰；修正后期望 `attempt_path=direct_ipv4`、`hello=ok`、`ping=ok`。
