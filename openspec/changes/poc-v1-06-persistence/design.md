## Context

06 是整套 `poc-v1` 并行抽离的地基。它没有上游硬依赖，但几乎是所有后续 change 的持久化 authority：

- `02` 写 `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`
- `03` 写 `last_seen_peers`
- `04` 读 `roster_snapshot + TopicScope`
- `07` 写 `ui_state`，并消费平行 v1 runtime DTO

## Extraction Strategy

- 新实现进入 `internal/pocv1/persist`。
- legacy `internal/pocstate` 只保留历史参考作用，不再作为当前 v1 state source-of-truth。
- 06 只拥有 layout 与 typed persist API，不拥有 join/enroll/dial/GUI 的业务逻辑。

## Scope

**06 owns:**

- 目录结构：
  - `device/`
  - `networks/<network_id>/`
- 文件职责：
  - `device/ed25519.key`
  - `device/x25519.key`
  - `networks/<id>/member_credential.bin`
  - `networks/<id>/mailbox_secret.bin`
  - `networks/<id>/roster_snapshot.json`
  - `networks/<id>/broker.json`
  - `networks/<id>/last_seen_peers.json`
  - `networks/<id>/ui_state.json`
- `broker.json`：当前 v1 恰好一个 runtime broker endpoint
- `TopicScope`：
  - `net_root`
  - `presence_topic(peer_id)`
  - `inbox_topic(peer_id)`
- 原子写：`tmp + rename`
- 权限：目录 `0700`，文件 `0600`
- typed store APIs

**06 does not own:**

- bootstrap protocol semantics（`02`）
- presence payload semantics（`03`）
- punch/session/runtime orchestration（`04/05/07`）
- 加密落盘或解锁 UI

## Owned Paths (planned)

- `internal/pocv1/persist/*`
- `internal/pocv1/persist/*_test.go`

## Task Breakdown

1. 实现 `device/` 与 `networks/<network_id>/` layout resolver。
2. 实现 typed stores：device keys、self member credential、mailbox secret、single runtime broker、roster snapshot、last seen peers、ui state。
3. 实现 `TopicScope`：从 `network_id + mailbox_secret + peer_id` 派生 `net_root`、presence topic 与 inbox topic。
4. 实现 atomic writer、permission enforcer 与 restart-safe reload。
5. 为 first-run、rewrite、corrupt-file、permission drift 增加测试。

## Acceptance

- 冷启动时能只创建最小目录，不会伪造网络状态。
- 任一 state rewrite 失败不会留下半写文件。
- `02/03/04/07` 都可通过 06 APIs 或 `TopicScope` 完成状态读写与 topic 派生，无需直接操作 legacy `pocstate`。
