# POC-06.5 设计：治理/成员/撤销 + report/export

> 目标：把 `docs/notes/2026-04-15-alpha-product-discussion.md` 中已敲定的治理与可解释性最小闭环落地到可运行实现，并与 POC-06 `sh(tmux)` 纵切片对齐。

## 1. 状态目录（state_dir）与持久化布局

以现有 `state.json` 所在目录作为 `state_dir`（system/user mode 自动落位）。

新增（原子写入）：

- `identity/`：本机长期身份密钥（Ed25519 签名 + X25519 静态 ECDH）
- `net.json`：`net_id/net_secret/brokers_effective/contact_set/...`
- `governance/head_snapshot.json`：当前治理链头（格式 v1；TUF-style `key_id` + 多签；rotation 验证口径见下）
- `decls/decls.json`：声明集合（approve/revoke；set-union 收敛；含 tombstone）
- `invites/<invite_id>.json`：沿用 invite/approve 幂等与 uses 持久化格式（见 `miopunch-poc-control-plane-invite-approve-idempotency`）
- `reports/`：最近 N 次 task 报告（ring buffer；默认 `reports_keep=20`）

## 2. 身份与标识（peer_id / net_id）

- `peer_id = base32(raw,no-pad, sha256(ed25519_sign_pubkey)[:16])`（固定 26 字符，规范输出大写）
- `net_id = base32(raw,no-pad, sha256(net_secret)[:16])`（固定 26 字符，规范输出大写）

## 3. 声明集合（decls）

最小 decl 结构（POC v0）：

```json
{
  "msg_id": "<26ch>",
  "created_at_unix_ms": 0,
  "issuer_peer_id": "<26ch>",
  "kind": "approve_member|revoke_member",
  "body": {},
  "sig_b64": "<ed25519_sig_b64url_no_pad>"
}
```

- `approve_member`：新增/确认成员（携带该成员的 identity pubkeys + 最小 contact）
- `revoke_member`：永久 tombstone，优先级最高

收敛：

- `decls` 以 set-union 收敛（元素为 decl；基于 `msg_id` 去重）
- `revoke_member` 一旦存在则永久拒绝该 peer（需要回来 = 换新 identity 再 join）

## 4. 治理链头（head_snapshot）

POC-06.5 采用“社区最佳实践（TUF root rotation）”的治理快照签名口径（实现为 head snapshot 格式 v1）：

- `snapshot_body`（参与 hash/签名）包含：
  - `net_id`（必需；绑定网络，避免跨网误用/误签）
  - `height`（必需；genesis=0；每次变更 +1）
  - `prev_hash_b64`（必需；genesis=""；非 genesis 必须指向本地链头）
  - `owners/admins`（治理集合；owner 用于签名信任根，admin 用于日常 decl 验签）
- `signatures[]`（owner 多签集合）：
  - `{ key_id, sig_b64 }`，其中 `key_id = hex(sha256(ed25519_pub))`（TUF 风格）
- 校验/应用语义（敲定）：
  - **bootstrap accept（join 初次落盘，本地无 head）**：仅要求“自洽验签”（new-threshold=1）+ `net_id` 一致 + 字段形状合法
  - **apply update（本地已有 head）**：要求同时满足
    - `prev_hash_b64 == local_head_hash`、`height == local_height+1`、`net_id == local_net_id`
    - old-threshold（=1）：至少 1 个签名可被“本地当前 head 的 owners”验签通过
    - new-threshold（=1）：至少 1 个签名可被“候选 snapshot_body 的 owners”验签通过

owner-only 的 propose/sign/apply CLI 仍后置到后续 change（但 hash/签名/threshold 口径固定）。

## 5. invite/join/approve（MQTT mailbox）

- `miopunch invite` 生成 invite code：
  - `invite_topic`：随机高熵（≥128bit）topic 名（不含 peer_id/name）
  - `invite_secret`：随机 32B（用于 join_request 加密）
  - `invite_brokers`：1..2 个 broker 端点（host:port）
  - `expires_at/max_uses/mode`
- `miopunch join <code>`：
  - 生成/复用本机 identity，派生 `peer_id`
  - 发布 `join_request`（invite_secret AEAD；携带 reply_topic + identity pubkeys + contact）
  - 等待 `membership_bundle` 并落盘：`net.json` + `head_snapshot.json` + `decls.json`
- `miopunch approve <code>`：
  - 监听 `invite_topic`，处理 join_request
  - uses/幂等：由 issuer/admin 节点落盘（`invites/<invite_id>.json`）
  - 生成 `approve_member` decl（admin 签名），写入 `decls.json`
  - 回包 membership_bundle（E2E 给 joiner：X25519+HKDF+AEAD）

## 6. 撤销（revoke_member）

- `miopunch revoke <peer> --dangerous`：
  - 生成 `revoke_member` decl（admin 签名），写入 `decls.json`
  - 即时生效：本地拒绝该 peer 的后续请求（含 `ping/sh` 的能力握手）

## 7. POC-06 `ping/sh` 强制能力握手（现场语义 + 可撤销）

在现有 `ping/sh_ls/sh_attach` 之前增加 `hello` 握手：

- visitor 必须发送：
  - `peer_id`
  - `approve_member` decl（首次连接携带；便于对端“未知成员”也可验真接纳）
  - `sig_b64`（用 joiner 的 Ed25519 私钥对握手 transcript 签名，证明持钥）
- controlled 侧必须：
  - 验签：issuer 必须属于当前治理快照 admins
  - 校验：若目标 peer 被 revoke 则拒绝
  - 接纳：未知成员但 decl 验真通过 → 落盘写入 `decls.json`

## 8. report/export

- daemon：task 完成即生成 report markdown 并写入 `reports/<task_id>.md`，按 `reports_keep` 轮转
- CLI：新增
  - `--report <path>`：将本次运行的 task 报告导出为 Markdown
  - `--redact`：对外脱敏（最小规则：不输出 secret/key 的明文值）
