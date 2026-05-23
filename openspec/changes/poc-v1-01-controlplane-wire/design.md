## Context

POC v1 的硬约束是：闭环必须可演示、作者能讲清楚、并且 Rule-of-One（每条轴只保留一种实现）。控制面如果不先把 wire 与安全语义定住，后续 join/enroll/punch 很容易又退回“靠约定拼流程”的状态。

## Goals / Non-Goals

**Goals:**
- 控制面 peer-targeted 消息统一 TLV wire（outer/inner/body 全部是 bytes）。
- 签名输入（transcript）固定为 `domain-sep + TLV(fields...)`，字段顺序写死。
- `peer_e2e_v1` 固定为 sign-then-encrypt（recipient-only），不提供可配置矩阵。
- 错误语义固定为：丢弃 + 聚合（用于 GUI reason_code 映射）。

**Non-Goals:**
- 不实现 join/enroll/dial/punch 的业务流程。
- 不实现 group-scoped 广播/治理类控制消息。
- 不引入 HPKE 标准化库或多套 E2E scheme。

## Decisions

### 1) Wire encoding = TLV（v1 固定）

- `TLV = tag(uvarint) || len(uvarint) || value(bytes)`。
- MQTT payload 直接传输 bytes；JSON 只用于 GUI/日志，不进入签名输入。

### 2) Transcript（签名输入）固定

- `transcript = "miopunch/v1/transcript/<context>" || TLV(fields...)`。
- `msg_id` 固定 16B rand；时间统一 unix ms。

### 3) peer_e2e_v1 固定加密构造

- X25519(eph, dst) 做共享密钥；HKDF 派生 key；XChaCha20-Poly1305 AEAD；AAD 绑定 outer header。
- v1 只允许 1 种 ct 编码：`"MP1" || eph_pub || nonce24 || aead_seal(inner_bytes)`。

### 4) 错误语义

- decrypt_fail/bad_sig/expired/replay/unsupported/malformed 一律丢弃。
- 仅提供聚合计数与最后一次原因（供 GUI 映射 reason_code）。

## Owned Paths (planned)

- `internal/controlplane/wirev1/*`
- `internal/controlplane/peere2ev1/*`
- `internal/controlplane/wirev1/*_test.go`
