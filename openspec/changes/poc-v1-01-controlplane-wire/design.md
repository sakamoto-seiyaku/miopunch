## Context

POC v1 的硬约束是：闭环必须可演示、作者能讲清楚、并且 Rule-of-One（每条轴只保留一种实现）。控制面如果不先把 wire 与安全语义定住，后续 join/enroll/punch 很容易又退回“靠约定拼流程”的状态。

## Goals / Non-Goals

**Goals:**
- 冻结 peer-targeted 消息的 outer/inner envelope（outer 明文投递；inner 解密后验签产生安全语义）。
- 冻结 TLV wire（tag/len uvarint + canonical/拒绝规则 + 字段顺序写死）。
- 冻结 transcript（签名输入）为 `domain-sep + TLV(fields...)`（字段顺序写死；不允许 JSON canonicalization）。
- 冻结 `peer_e2e_v1` 为 sign-then-encrypt（recipient-only）：Ed25519 + X25519 + HKDF-SHA256 + XChaCha20-Poly1305。
- 冻结 v1 kind allowlist：仅允许 `join_request/enroll_response/dial_offer/dial_answer`。
- 冻结错误语义：丢弃 + 聚合（用于 GUI reason_code 映射）。
- 冻结 golden vectors（外层/内层/密文/签名输入的字节级 fixtures）。

**Non-Goals:**
- 不实现 join/enroll/dial/punch 的业务流程。
- 不冻结各业务 kind 的具体 body schema（由后续 changes 拥有）。
- 不实现 group-scoped 广播/治理类控制消息。
- 不引入 HPKE 标准化库或多套 E2E scheme。

## Decisions

### 0) v1 kinds（Rule-of-One）

v1 peer-targeted 控制消息的 `kind` 字段只允许以下四种（其它一律拒绝并丢弃）：

- `join_request`
- `enroll_response`
- `dial_offer`
- `dial_answer`

### 1) TLV wire（v1 固定）

所有 v1 peer-targeted 控制消息都编码为二进制 TLV，并作为 MQTT payload 直接传输 bytes。

#### TLV primitive

`TLV = tag(uvarint) || len(uvarint) || value(bytes)`

约束：

- `tag` 与 `len` 的 uvarint 编码必须是 **canonical**（即使用 Go `encoding/binary.PutUvarint` 输出的最短编码；禁止多余的 continuation 字节，例如 `0x80 0x00`）。
- `len` MUST match `value` length exactly；越界、截断、一律拒绝。
- `tag` MUST be non-zero。
- 单个 message 的总长度必须有上限（实现上以常量限制，避免 OOM）；本 change 不冻结具体数值，但必须“有界”。

#### Field encoding

字段类型编码规则（v1 固定）：

- `bytes`: 原样写入 `value`。
- `u64`: 使用 `encoding/binary.PutUvarint` 的 **canonical** uvarint bytes（不是固定 8B little endian）。
- `ascii`: UTF-8 bytes，但要求全部为 ASCII（用于 `kind`、`scheme` 等固定关键字）。

#### Ordering / duplicates / unknown tags

为保证“只有一种正确做法”，同一 message 的 TLV fields 必须满足：

- fields MUST be sorted by increasing `tag`（严格递增）。
- duplicate `tag` MUST be rejected。
- unknown `tag` MUST be rejected（协议冻结期间不做“忽略未知字段”的前向兼容）。

### 2) Outer/Inner envelope（安全语义边界）

v1 peer-targeted 消息统一采用 outer/inner 两层：

- **Outer relay header（明文）**：用于 broker/relay 投递与观测，`src` 不可信。
- **Inner peer message（密文内）**：解密后验签，产生安全语义上的发送者身份与业务输入。

#### Outer relay header（TLV）

outer 必须编码为 TLV message，字段与 tag 表如下（固定）：

| tag | name | type | required | notes |
| --- | --- | --- | --- | --- |
| 1 | `v` | u64 | yes | v1 固定为 `1` |
| 2 | `src_peer_id` | bytes | yes | 16B（不可信，仅路由/调试）；可由实现派生/填充 |
| 3 | `dst_peer_id` | bytes | yes | 16B（收件人 peer_id_raw16） |
| 4 | `msg_id` | bytes | yes | 16B random |
| 5 | `expires_at_unix_ms` | u64 | yes | unix ms |
| 6 | `scheme` | ascii | yes | v1 固定为 `peer_e2e_v1` |
| 7 | `ct` | bytes | yes | ciphertext bytes（见下文） |

outer fields 的 tag 顺序必须严格为 `1..7`（即严格递增）。

#### Inner peer message（TLV）

inner 必须编码为 TLV message，字段与 tag 表如下（固定）：

| tag | name | type | required | notes |
| --- | --- | --- | --- | --- |
| 1 | `src_pub_ed25519` | bytes | yes | 32B |
| 2 | `msg_id` | bytes | yes | 16B；必须与 outer 一致 |
| 3 | `created_at_unix_ms` | u64 | yes | unix ms |
| 4 | `expires_at_unix_ms` | u64 | yes | unix ms；必须与 outer 一致 |
| 5 | `kind` | ascii | yes | 只允许 v1 allowlist |
| 6 | `in_reply_to` | bytes | no | 16B msg_id |
| 7 | `body_bytes` | bytes | yes | 业务负载（由后续 changes 冻结具体 schema） |
| 8 | `sig_ed25519` | bytes | yes | 64B；签名输入见 transcript |

inner fields 的 tag 顺序必须严格为 `1..8`（即严格递增）；`in_reply_to` 缺省时跳过 tag 6（不允许以 `len=0` 表示缺省）。

### 3) Transcript（签名输入，字段顺序写死）

签名输入（transcript）固定为：

`transcript = domain_sep || tlv_transcript_fields`

其中：

- `domain_sep`：ASCII bytes `"miopunch/v1/transcript/peer_message"`。
- `tlv_transcript_fields`：按固定 tag 表编码的 TLV（不是 outer/inner 的完整 TLV；只包含被签名覆盖的字段）。

#### Transcript fields（fixed）

`tlv_transcript_fields` 的 tag 表与顺序固定为：

| tag | name | type | required |
| --- | --- | --- | --- |
| 1 | `msg_id` | bytes | yes |
| 2 | `created_at_unix_ms` | u64 | yes |
| 3 | `expires_at_unix_ms` | u64 | yes |
| 4 | `kind` | ascii | yes |
| 5 | `src_pub_ed25519` | bytes | yes |
| 6 | `body_bytes` | bytes | yes |

注意：

- transcript 直接对 `transcript` bytes 进行 Ed25519 签名（不做 SHA256 预哈希）。
- outer 的 `src_peer_id` 不进入 transcript（outer src 不可信）。

### 4) peer_e2e_v1（sign-then-encrypt recipient-only）

peer_e2e_v1 固定为 sign-then-encrypt（recipient-only），且只允许单一构造：

1. 构造 inner（不含 `sig_ed25519`）。
2. 构造 transcript（见上文），并使用发送者 Ed25519 对 transcript bytes 直接签名写入 `sig_ed25519`。
3. 将完整 inner TLV bytes 加密为 `ct`（sealed-box 语义）。

#### Key agreement

- 发送者每条消息生成一次性 `eph_x25519` keypair。
- `shared = X25519(eph_priv, dst_x25519_pub)`。
- `salt = sha256(msg_id || dst_x25519_pub)`（32B）。
- `key32 = HKDF-SHA256(ikm=shared, salt=salt, info="miopunch/peer_e2e_v1/aead", L=32)`。

#### AEAD

- AEAD：`XChaCha20-Poly1305`。
- `nonce24 = random(24B)`。

#### AAD binding

AAD 固定绑定到 outer header 的下列字段（按顺序，以 TLV 编码）：

| tag | name | type |
| --- | --- | --- |
| 1 | `v` | u64 |
| 2 | `dst_peer_id` | bytes |
| 3 | `msg_id` | bytes |
| 4 | `expires_at_unix_ms` | u64 |
| 5 | `scheme` | ascii |

AAD 不包含 `ct` 本身，也不包含不可信的 `src_peer_id`。

#### Ciphertext frame

`ct` bytes 的编码固定为：

`ct = magic(3B) || eph_pub_x25519(32B) || nonce24(24B) || aead_ct(bytes)`

其中 `magic = "MP1"`（ASCII）。

### 5) Error semantics（drop-only）

解密失败、验签失败、过期、重放、版本不支持、格式错误等一律：

- 本地丢弃，不产生任何网络回包；
- 仅为 GUI/Evidence 提供 reason 聚合（分类建议：`decrypt_fail/bad_sig/expired/replay/unsupported_version/malformed`）。

### 6) Golden vectors（byte-level determinism）

本 change 要求提供可回归的 golden vectors（hex fixtures），至少覆盖：

- outer header TLV bytes（给定字段值，bytes 必须完全一致）。
- inner message TLV bytes（含签名；并提供验签通过样例）。
- transcript bytes（inner_without_sig 的签名输入 bytes）。
- ciphertext `ct` frame bytes（固定 `eph_priv` 与 `nonce` 时输出必须一致；tamper/AAD 改动必须失败）。

golden vectors 的承载形式由实现决定（例如 testdata/*.hex + JSON 元数据），但必须可被单测消费并逐字节比对。

## Owned Paths (planned)

- `internal/controlplane/wirev1/*`
- `internal/controlplane/peere2ev1/*`
- `internal/controlplane/wirev1/*_test.go`
