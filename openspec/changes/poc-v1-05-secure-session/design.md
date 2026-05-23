## Context

POC v1 的闭环必须能讲清楚“我连到的对端是谁”。因此数据面 secure channel 必须绑定到入网 credential，而不是绑定到一段临时 secret 或日志里才看得懂的 sid。

依赖：`poc-v1-04-dial-punch`（dial 消息中携带 MemberCredential）。

## Goals / Non-Goals

**Goals:**
- recipe 固定为 `UDP + KCP + TLS1.3 + yamux`。
- TLS pin 固定为 6A：pin 到 MemberCredential 身份。

**Non-Goals:**
- 不实现 QUIC/TCP 方案。
- 不实现数据面 relay fallback。

## Decisions

- 证书：每个 peer 用自身 Ed25519 自签证书，仅用于携带公钥。
- 校验：自定义 verify 读取对端证书公钥，与 dial 消息中对端 credential 绑定的公钥一致；同时验 authority 签名与时间窗。

## Owned Paths (planned)

- `dataplane/session_transport.go`
- `dataplane/session_listener_udp.go`
- `internal/tlsutil/*`
