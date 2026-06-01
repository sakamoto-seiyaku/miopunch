## Context

POC v1 这一轮不是在旧实现上“继续补齐功能”，而是要从当前混乱的产品栈中抽离出一条作者能完整解释、能逐号验证、最终能在桌面闭环跑通的新主线。

01 是这条主线的第一块地基：先把 peer-targeted 控制面消息的 bytes contract 与安全语义拆出来，否则 `02/03/04/05` 仍会被 legacy JSON/message/task 结构反向污染。

## Extraction Strategy

- 新实现并行放入 `internal/pocv1/wire` 与 `internal/pocv1/peere2e`。
- `internal/controlplane/*` 中现有 message/sign/encoding/msg_id 逻辑只作为行为参考，不再承接新的 v1 领域模型。
- v1 运行时不得再以旧 JSON/AES-GCM message struct 作为当前 source of truth；它们只保留给 archived POC v0。
- 01 只拥有 “顶层 kind 名字 + bytes contract + E2E semantics”，body schema 与流程一律交给后续编号。
- `network_id` / `network_id_bytes` 的双表示边界在这一层先冻结：外部统一用 26 字符 uppercase base32 `network_id`，只有 wire/TLV/crypto 内部才使用 raw 16B `network_id_bytes`。

## Scope

**01 owns:**

- TLV primitive 与 canonical/strict reject 规则。
- outer relay header / inner peer message 两层 envelope。
- 固定 transcript 与 Ed25519 直接签名口径。
- `peer_e2e_v1` 的唯一构造：X25519 + HKDF-SHA256 + XChaCha20-Poly1305。
- `network_id` canonical string 与 raw 16B `network_id_bytes` 的跨 capability 表示边界。
- v1 peer-targeted 顶层 `kind` 名字集合：
  - `join_request`
  - `enroll_response`
  - `dial_offer`
  - `dial_answer`
- replay/expiry admission hook、drop-only 错误语义与 golden vectors。

**01 does not own:**

- `join_request/enroll_response` 的 body 字段集与 authority 流程（`02`）。
- `dial_offer/dial_answer` 的 body 字段集、attempt matrix、`PathResult`（`04`）。
- `SessionRecipe`、`PeerSession`、TLS pin（`05`）。
- presence topic / payload（`03`）。
- authority 侧按 `msg_id` 做 side-effect dedupe / cached response 的业务语义（`02`）。
- GUI reason_code 编排与 stage 状态机（`07`）。
- mesh/relay/bounded flooding/group-scoped message。

## Owned Paths (planned)

- `internal/pocv1/wire/*`
- `internal/pocv1/peere2e/*`
- `internal/pocv1/wire/testdata/*`
- `internal/pocv1/wire/*_test.go`
- `internal/pocv1/peere2e/*_test.go`

## Task Breakdown

1. 在 `internal/pocv1/wire` 中建立 TLV primitive、strict decoder、outer/inner envelope 与 transcript builder。
2. 在 `internal/pocv1/peere2e` 中建立 `peer_e2e_v1` seal/open、AAD 绑定与 ciphertext frame。
3. 为 `join_request/enroll_response/dial_offer/dial_answer` 只暴露顶层 `kind` 常量与 body passthrough，不在 01 提前解析业务 schema。
4. 为 v1 wire/security 提供 golden vectors、tamper fixtures，以及仅限 wire/security admission 的 replay/expiry hook + local failure-kind contract。
5. 证明当前 v1 runtime 不再依赖 legacy JSON/AES-GCM contract 解释 peer-targeted 消息。

## Acceptance

- 单元测试覆盖 canonical TLV、unknown/dup/order/non-canonical reject。
- 固定输入下 outer/inner/transcript/ct bytes 完全稳定，可做 byte-for-byte 回归。
- 改动 outer `src` 不影响安全语义；改动 AAD 或 ciphertext 必定失败。
- v1 代码路径只以 `miopunch-poc-v1-controlplane-wire` 为事实源；旧 `miopunch-poc-control-plane-wire-format` 仅作为 legacy/v0 文档保留。
