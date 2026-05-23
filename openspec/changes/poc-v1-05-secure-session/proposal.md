## Why

POC v1 的数据面必须“强身份绑定”，否则面试叙事会变成“只有加密没有认证”。Hard-Min 选择：`UDP -> KCP -> TLS1.3(pinned) -> yamux`，并把 pin 绑定到 MemberCredential 身份。

## What Changes

- 定义并实现 v1 `SessionRecipe`：消费 `PathResult`，升级为 `PeerSession`（KCP + TLS1.3 + yamux）。
- TLS pin 口径固定：对端证书 Ed25519 pub 必须与其在 dial_offer/answer 提供的 MemberCredential.subject_ed25519_pub 一致，且该 credential 必须可被 authority 验签。
- 上层业务只依赖 `PeerSession/OpenStream`（保持主干）。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-secure-session`: v1 数据面 secure session recipe 与 pin 规则。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：`dataplane/` session transport、`internal/tlsutil/` pin 逻辑、以及 `PathResult -> PeerSession` 的 recipe 胶水。
