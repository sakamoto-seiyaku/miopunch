## Context

重组 POC v1 需要可预测的本地状态结构，否则 join/enroll/dial 的实现很容易把状态散落在各个 task 里。

依赖：无硬依赖（可最先 apply 作为地基；供 enroll/dial/gui 复用统一落盘入口）。

## Goals / Non-Goals

**Goals:**
- 目录结构固定：`device/` 一份长期密钥；`networks/<network_id>/` 保存网络相关状态。
- 原子写：写临时文件再 rename。
- 权限收紧：文件 0600、目录 0700。

**Non-Goals:**
- 不实现加密落盘/解锁 UI。

## Decisions

- `device/`：`ed25519.key`、`x25519.key`。
- `networks/<id>/`：`member_credential.bin`、`mailbox_secret.bin`、`broker.json`、`last_seen_peers.json`、`ui_state.json`。

## Owned Paths (planned)

- `internal/persist/*`
- `internal/task/desktop_state.go`（迁移 state 到 persist）
