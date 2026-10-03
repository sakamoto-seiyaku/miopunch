# MIOPUNCH-64 two-axis review

Fixed point: `0332c362fc7d897745a095cac1501a0e9f6a7e6f` (completed browser
MIOPUNCH-65). Original review command:
`git diff 0332c362fc7d897745a095cac1501a0e9f6a7e6f...964e47e0dc02e13b0e285b3f328043e8a569404b`.
Commit: `964e47e0 MIOPUNCH-64: verify public resources on Linux and physical Android`.
Both independent reviewers used GPT-6.1 Sol, high reasoning and actual priority
service tier, verified by the scheduling thread. They read the full Plane
MIOPUNCH-64 and parent specification, current repository rules and accepted
domain decisions. The Standards reviewer also received the full Fowler smell
baseline. They performed read-only review and did not duplicate tests.

## Standards

Reviewed only `0332c362...964e47e0`.

No hard documented coding-standard breaches found. New decisions are explicitly
recorded; archived workflows are not imposed.

Two nonblocking heuristics to park:

- **Possible Duplicated Code — `src/desktop_cli.rs:400`, `:434`, `:461`.** Three
  command branches repeat `if hints_only { tokio::time::timeout(...,
  resources.endpoint().connect(...)).await?? } else {
  resources.connect(...).await? }`. A private connection helper could keep
  timeout and recovery behavior consistent.
- **Possible Data Clumps — `src/mqtt_transport.rs:138`, `src/resources.rs:568`,
  `scripts/check_public_resources.py`.** Diagnostics travel as
  `(String, bool, u64, u64)` and consumers interpret `row[1]`, `row[2]`, `row[3]`.
  Named diagnostic fields would clarify readiness and traffic counters.

Separately referred the TCP observation supersession race to the parent for
Spec-axis evaluation: concurrent queries can let an older completion replace
a newer retained observation.

No edits or tests performed.

## Spec

Spec：1 项发现。

- **P2 — 并发 TCP STUN 查询未正确失效旧结果**（`src/resources.rs:649`）。规格要求：
  “地址观察关联实际承担该传输的 socket/连接，并记录新鲜度、失败和失效。”集成决议
  进一步承诺新查询使旧结果失效。原提交 `964e47e0` 仅在查询开始时移除已保存连接；
  较早的在途查询仍可在同源新查询开始或失败之后完成，将旧结果重新保存并报告
  `fresh=true`。应使用每个来源的查询版本或取消令牌，在完成与发布时拒绝被替代的查询。

其余验收未发现规格偏差。证据支持真实公网中继通信及恢复；UDP STUN 超时、缺少独立
网络和前台 Android 测试壳均有明确限定，未冒充通过。此次审核只读，未重复测试。

## Spec finding resolution

Fix: `4439543d84a11db33e7f798b1673bc487dd37bc4`,
`MIOPUNCH-64: cancel superseded TCP STUN observations`.
Follow-up command: `git diff 964e47e0...4439543d` over the resource observation,
public regression and corresponding note/evidence changes. The same Spec
reviewer completed a fresh follow-up turn with actual Sol 6.1 / high / priority
settings read back. Original failure is in
`failures/tcp-supersession-red.log`; the affected 33-pass result is in
`checks/tcp-supersession-green-affected.log`.

原 P2 已解决。`4439543d` 在同一互斥锁下替换并取消旧查询，发布结果前再次检查取消状态；
已返回观察的 `is_fresh()` 也检查该令牌，因此旧查询不能覆盖新尝试。

RED 记录复现原问题；GREEN 记录显示新增回归及受影响资源测试共 33 项通过、0 失败。
此次修复范围内未发现新的具体规格偏差或回归。

仅进行了只读复核，未重复测试。

The two Standards heuristics remain parked under the user's architecture
instruction; neither is a hard violation or a new refactor assignment.

Standards: 2 nonblocking heuristics, 0 hard breaches (worst: possible duplicated
code). Spec: 1 P2 resolved, 0 remaining findings (worst originally: TCP query
supersession).
