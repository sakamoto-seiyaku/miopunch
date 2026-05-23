## Context

重组 POC v1 需要可预测的本地状态结构，否则 join/enroll/dial 的实现很容易把状态散落在各个 task 里。

依赖：无硬依赖（应最先 apply 作为地基；供 enroll/dial/gui 复用统一落盘入口）。

## Scope

- 冻结目录结构：`device/` 一份长期密钥；`networks/<network_id>/` 保存网络相关状态。
- 冻结文件职责：`device/ed25519.key`、`device/x25519.key`、`networks/<id>/member_credential.bin`、`mailbox_secret.bin`、`broker.json`、`last_seen_peers.json`、`ui_state.json`。
- 冻结原子写和权限语义：tmp + rename，目录 0700，文件 0600。
- 本 change 只拥有 persist layout 和 persist API；caller migration 由实际 apply 的业务 change 各自承担。
- 不实现加密落盘或解锁 UI。

## Owned Paths (planned)

- `internal/persist/*`

## Done

- v1 layout、文件职责、原子写、权限语义冻结完成。
- 提供统一 persist API 供后续 changes 接入。
- 不在本 stub 中承接 join/enroll/dial/gui 的调用迁移或 GUI state model 设计。
