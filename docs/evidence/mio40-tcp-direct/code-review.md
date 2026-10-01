# MIOPUNCH-40 two-axis review

Pinned base: `db292f540d382a9a8e7ae408f2d657afa8fa63f7`. Review snapshot: `0b9df2cc62b9b22fe4b34e10c5b895a067274786`. Both independent reviewers used GPT-6.1 Sol HIGH. Runtime source remained unchanged during review.

## Standards

# MIOPUNCH-40 standards review

Reviewed snapshot `0b9df2cc62b9b22fe4b34e10c5b895a067274786` against
`db292f540d382a9a8e7ae408f2d657afa8fa63f7` using the three-dot diff.
Scope: runtime, fork adaptations, formal TCP tests, runner, and accompanying
validation/evidence documentation.

**Documented-standard breaches: none found.**

Standards consulted: `AGENTS.md`, `/home/js/.codex/RTK.md`,
`docs/agents/{domain,issue-tracker,triage-labels}.md`, `CONTEXT.md`, current
product/scenario and transport notes, and ADRs 0001, 0004 and 0005. Archived
Go/OpenSpec rules were excluded as required by `AGENTS.md`.

The change preserves the documented distinctions between node identity,
membership, direct peer paths and auxiliary resources. New configuration and
wire/lifecycle choices are explicitly recorded in the TCP validation note;
research suggestions are not presented as product decisions. The evidence
index retains acceptance evidence without becoming an open-work ledger, as
required by `docs/agents/issue-tracker.md`.

**Optional baseline smell findings: none warranting a change.**

The shared `ResourceConfig::allows_custom` policy centralizes qualification.
Checks at candidate selection and actual send serve different enforcement
boundaries. The fork wrapper and public adapter helpers provide necessary
policy and encoding seams; their delegation does not warrant a Middle Man
finding. IPv4 compatibility helpers preserve existing callers.

This was an independent static standards review, not a new runtime validation
run. Tooling-enforced formatting/typechecking and unchanged slow NAT matrices
were not rerun or imposed as additional gates.

Total: **0 required violations; 0 optional smell findings.**

## Spec

# MIOPUNCH-40 independent Spec review

Reviewed `db292f540d382a9a8e7ae408f2d657afa8fa63f7...0b9df2cc62b9b22fe4b34e10c5b895a067274786` against the preserved Plane MIOPUNCH-40 description, MIOPUNCH-14 parent specification, completed dependencies 18/38, current context/design notes and relevant ADRs. Standards findings are outside this report.

**Findings: 0.** No missing/partial requirement, unrequested product scope or incorrectly implemented requirement identified within this ticket's explicit reachable-address scope.

- “原生节点通过可达 TCP 地址完成正式准入、控制同步和受控业务，上层继续使用统一连接接口。” Saved startup installs the carrier through the existing node builder and gate (`src/resources.rs:241`; `src/native_network.rs:493`). The real tests complete manual admission, reopen durable state, fetch a newly signed control record absent before synchronization, verify its applied membership effect, and exchange business bytes through ordinary Connection streams (`tests/tcp_direct.rs:87`, `:149`, `:161`).
- “被撤销/错误身份、禁用候选和取消均遵守统一行为，不因 TCP 成功跳过认证。” Expected-identity substitution, valid-but-unapproved identity and member revocation are checked through real formal connections (`tests/tcp_direct.rs:209`, `:318`, `:363`). Candidate permission is checked at final custom send, including raw Endpoint and lookup bypass attempts (`vendor/iroh/src/socket/transports/custom.rs:81`, `:85`; `tests/tcp_direct.rs:262`, `:475`, `:520`). Endpoint cancellation observes actual TCP EOF (`tests/tcp_direct.rs:428`).
- “IPv4 与可用 IPv6 的直接路径有独立证据，连接断开明确结束，不伪装成永续会话。” Separate real IPv4/IPv6 tests disable UDP sockets (`tests/tcp_direct.rs:204`, `:306`). Complete physical path loss ends the original stream/connection; subsequent contact has a different stable ID (`:576`, `:690`).

Existing BBRv3 configuration is reused (`src/resources.rs:263`); retained carrier evidence exercises bounded backpressure. Inspected final receipt36 records 11 passed/0 failed, and affected receipts record 40 additional passed cases. No additional runtime suite was executed for this review. Evidence remains limited to Linux loopback literal IPv4/unscoped IPv6; no NAT, cross-operator, other-platform or universal-timeout claim is inferred. Startup choices and exclusions are explicitly recorded in the TCP validation note.

Standards:0 required /0 optional; Spec:0 findings. Neither axis has a worst issue.
