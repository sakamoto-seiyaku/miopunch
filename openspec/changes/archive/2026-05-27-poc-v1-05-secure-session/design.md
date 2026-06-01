## Context

05 承接的是 04 的 `PathResult`，向上交付的是 `PeerSession`。为了保证这轮 POC v1 真正“能解释”，05 必须只负责 session recipe 与身份 pin，不再回头掺 candidate/punch 或 GUI/runtime。

## Extraction Strategy

- 新实现进入 `internal/pocv1/session`。
- 可窄适配 legacy `dataplane` 与 `internal/tlsutil`，但 v1 的 recipe、pin contract 与 handoff types 由 05 拥有。
- `05` 从 `Store` 读取本机 device keys 与 self member credential，并用 `AuthorityEd25519Pub` 验证本机/远端 credential。
- `PeerSession` 继续作为上层业务边界保留；`sh/ping` 只能看见 `PeerSession/OpenStream/AcceptStream`。

## Scope

**05 owns:**

- `SessionRecipe`
- `PeerSession`
- `OpenStream/AcceptStream`
- 唯一 recipe：`UDP + KCP + TLS1.3 + yamux`
- 6A pin：
  - 对端证书 Ed25519 pub 必须等于 `MemberCredential.subject_ed25519_pub`
  - 该 credential 必须能被 authority 验签
  - 不再复用旧的 `secretKey + sid + role` 生成 pin 的规则作为 05 的事实来源
- 本机使用 Ed25519 自签证书承载公钥

**05 does not own:**

- candidate gather、attempt matrix、selected endpoint（`04`）
- QUIC/TCP/多 recipe 自动协商
- shell/protocol-specific stream framing（上层业务）

## Owned Paths (planned)

- `internal/pocv1/session/*`
- `internal/pocv1/session/*_test.go`

## Task Breakdown

1. 定义 `SessionRecipe`、`PeerSession` 以及 `PathResult -> PeerSession` adapter。
2. 复用或窄适配 legacy KCP/TLS/yamux 实现，建立 v1 单一 recipe。
3. 实现 6A pin 与 authority-backed credential verification。
4. 增加 `OpenStream/AcceptStream`、stream-open metadata 保留、pin fail、credential fail、timeout 等测试。

## Acceptance

- 05 只能从 `PathResult` 开始建会话，不再读取 punch/candidate internals。
- 成功握手后，`PeerSession` 成为上层唯一依赖边界。
- pin 失败或 credential 验签失败时，会话被拒绝，不会默默降级到另一条 recipe。
