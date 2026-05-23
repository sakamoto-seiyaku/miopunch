## Context

POC v1 的闭环必须能讲清楚“我连到的对端是谁”。因此数据面 secure channel 必须绑定到入网 credential，而不是绑定到一段临时 secret 或日志里才看得懂的 sid。

依赖：`poc-v1-04-dial-punch`（提供 `PathResult` 与对端 `MemberCredential`）。

## Scope

- 本 change 拥有 `SessionRecipe`：消费 `PathResult`，升级为 `PeerSession`，并向上提供 `OpenStream/AcceptStream`。
- recipe 固定为 `UDP + KCP + TLS1.3 + yamux`。
- TLS pin 固定为 6A：对端证书 Ed25519 pub 必须与 `MemberCredential.subject_ed25519_pub` 一致，且 credential 必须可被 authority 验签。
- 证书使用本机 Ed25519 身份自签，仅作为携带公钥的容器。
- 不拥有 candidate gather、attempt matrix、selected endpoint 或任何 punching 策略。

## Owned Paths (planned)

- `dataplane/session_transport.go`
- `dataplane/session_listener_udp.go`
- `internal/tlsutil/*`

## Done

- `PathResult -> SessionRecipe -> PeerSession` 主干中的 `SessionRecipe` 契约冻结完成。
- 6A pin 口径冻结完成，不引入第二套 pin 机制。
- 上层业务只依赖 `PeerSession/OpenStream`，不再知道 KCP/TLS/yamux 细节。
