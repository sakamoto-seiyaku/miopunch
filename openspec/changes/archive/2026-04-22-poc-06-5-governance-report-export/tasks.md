# POC-06.5 Tasks

> 勾选项按“实现 + 可解释性 + 可回归”组织；完成后应能跑通最小闭环：`invite → join → approve → ping → sh`，并支持 `revoke` 后立即拒绝。

## 1. OpenSpec / docs
- [x] 1.1 将本 change 的 delta specs 加入 `openspec/changes/.../specs/`
- [x] 1.2 更新 `docs/roadmap.md`：插入 POC-06.5；POC-07 仅 HTTP 面板

## 2. State dir / identity / hashing
- [x] 2.1 定义 `state_dir`（基于 `state.json` 所在目录）
- [x] 2.2 落盘 identity（Ed25519 + X25519），派生 `peer_id`
- [x] 2.3 落盘 `net.json`（含 `net_id/net_secret/brokers_effective`）

## 3. Governance head + decls
- [x] 3.1 genesis `governance/head_snapshot.json` 生成与验证口径
- [x] 3.2 `decls/decls.json`：approve/revoke decl 结构、签名、set head
- [x] 3.3 `revoke_member`：tombstone 语义与拒绝口径

## 4. invite/join/approve
- [x] 4.1 invite code v0：encode/decode + flags（mode/uses/expires）
- [x] 4.2 join_request：invite_secret AEAD 加密 + reply_topic + 重试
- [x] 4.3 approve：监听 invite_topic，uses/幂等持久化，回包 membership_bundle（X25519+AEAD）

## 5. ping/sh 强制 hello 握手
- [x] 5.1 shellproto：新增 `hello` op 与错误 reason_code
- [x] 5.2 controlled：验证 approve_member + revoke 拒绝；未知成员可接纳落盘
- [x] 5.3 visitor：在 ping/sh/sh_ls/sh_attach 前发送 hello

## 6. report/export
- [x] 6.1 daemon：task 完成报告落盘 + reports_keep 轮转
- [x] 6.2 CLI：`--report <path>` 导出 task report（LocalAPI 拉取）
- [x] 6.3 CLI：`--redact` 最小脱敏规则（默认不影响本机落盘日志）

## 7. 验证
- [x] 7.1 `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 7.2 `go vet ./...`
- [x] 7.3 `bash scripts/check_no_xtcp_imports.sh`

## 8. Review fixes（2026-04-22）
- [x] 8.1 更新 `/docs` 设计稿：治理快照 v1（TUF-style `key_id` + rotation old/new threshold + bootstrap 口径）
- [x] 8.2 更新本 change 的 design + delta spec：治理快照 v1 口径对齐
- [x] 8.3 实现 `governance/head_snapshot.json` v1（结构 + hash + signatures + rotation 校验）
- [x] 8.4 更新 join/bootstrap 与 acceptor：按 v1 口径校验并落盘
- [x] 8.5 补治理快照单元测试（bootstrap / update / no-op / 拒绝分叉）
- [x] 8.6 report 落盘失败可见化（task 输出 fact/suggestion；report markdown 可选附加警告）
- [x] 8.7 统一 atomic write helper（tmp→fsync→rename/replace + dir fsync），并用于 daemon report + `--report`
- [x] 8.8 gofmt（修复 gofmt 触发的 13 个文件）
- [x] 8.9 复跑：`go test ./...` + `go vet ./...` + `bash scripts/check_no_xtcp_imports.sh`
