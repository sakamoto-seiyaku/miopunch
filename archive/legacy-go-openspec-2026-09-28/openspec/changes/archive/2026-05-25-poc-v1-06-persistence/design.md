## Context

06 是整套 `poc-v1` 并行抽离的地基。它没有上游硬依赖，但几乎是所有后续 change 的持久化 authority：

- `02` 写 `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`
- `03` 读 `roster_snapshot`，并在它需要持久化 `last_seen_peers` 时先冻结自己的 object model
- `04` 读 `roster_snapshot + TopicScope`
- `07` 决定 wizard-local `ui_state` 形状，并消费平行 v1 runtime DTO

## Extraction Strategy

- 新实现进入 `internal/pocv1/persist`。
- `internal/pocv1/persist` 只接受 caller-supplied root dir；它不从 legacy `state.json` 或其它 v0 全局路径规则里推导自己的 root。
- legacy `internal/pocstate` 只保留历史参考作用，不再作为当前 v1 state source-of-truth。
- 06 只拥有 layout 与 typed persist API，不拥有 join/enroll/dial/GUI 的业务逻辑。

## Scope

**06 owns:**

- root contract：
  - caller-supplied persist root dir
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
- `broker.json`：当前 v1 恰好一个 runtime broker endpoint
- grouped bootstrap handoff：
  - `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`
  - 同一 joined network 作为单个 success/failure unit 落盘
- `roster_snapshot`：
  - whole-read
  - whole-replace
- `TopicScope`：
  - `net_root`
  - `presence_topic(peer_id)`
  - `inbox_topic(peer_id)`
- 原子写：`tmp + rename`
- 权限：POSIX 目录 `0700`，POSIX 文件 `0600`；Windows 保持 restrictive intent，但权限修复允许 best-effort
- typed store APIs

**06 does not own:**

- global state-path discovery 或 legacy `state.json` root 推导
- bootstrap protocol semantics（`02`）
- presence payload semantics 与 `last_seen_peers` object model / merge policy（`03`）
- punch/session/runtime orchestration（`04/05/07`）
- `ui_state` shape、migration policy 或 desktop config 语义（`07`）
- 加密落盘或解锁 UI

## Owned Paths (planned)

- `internal/pocv1/persist/*`
- `internal/pocv1/persist/*_test.go`

## Task Breakdown

1. 实现 caller-supplied root 下的 `device/` 与 `networks/<network_id>/` layout resolver。
2. 实现第一批 typed stores：device keys、self member credential、mailbox secret、single runtime broker、whole roster snapshot。
3. 实现 `TopicScope`：从 `network_id + mailbox_secret + peer_id` 派生 `net_root`、presence topic 与 inbox topic。
4. 实现 atomic writer、permission enforcer、restart-safe reload，以及 bootstrap grouped write。
5. 为 first-run、single-file rewrite、bootstrap partial-failure、restart reload 与 permission drift 增加测试。

## Acceptance

- 冷启动时能只创建最小 root 与 `device/` 目录，不会伪造网络状态。
- joined bootstrap 任一步写入失败后，重启时不会看到“半 joined”的 network state。
- 任一 state rewrite 失败不会留下半写文件。
- `02` 与 `04` 可通过 06 APIs 或 `TopicScope` 完成 bootstrap handoff、trusted roster 读取与 topic 派生，无需直接操作 legacy `pocstate`。
- `03` 与 `07` 不再要求 06 foundation 先替 `last_seen_peers` 或 `ui_state` 定 schema。
