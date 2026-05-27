## Why

`wire / enroll / persist / presence / punch / session` 现在已经基本形成模块级闭环，但产品级闭环仍然卡在 legacy authority 上：`miopunch up`、CLI 命令、LocalAPI 和 desktop 入口仍主要由 `internal/task` 与 `/api/v0/desktop/state` 驱动。这样会让 `POCv1` 看起来像“只差一个 GUI”，实际上却没有一条可独立运行、可验证、可解释的新产品主线。

如果继续把 runtime authority 放在 `07` 里，那么 `07` 就不只是 GUI change，而会再次同时拥有 headless runtime、daemon、LocalAPI 和 desktop 壳职责，重新把 extracted-v1 与 legacy task 编排缠在一起。

## What Changes

- 新增 pre-07 change：`poc-v1-06x-headless-cli-runtime`，专门抽离 `internal/pocv1/runtime`、shared daemon、`localapi` RPC 壳与 CLI 真闭环。
- 定义并后续实现 Linux-first 的 headless runtime：六阶段状态机、`SecureSession -> Shell` gate、structured `Evidence`、用户面 `reason_code`、peer/session/shell lifecycle。
- 保留现有产品 CLI 动词：`up`、`ls`、`init-network`、`invite`、`approve`、`join`、`ping`、`sh ls`、`sh`、`revoke`，但底层 authority 改由 extracted-v1 runtime 拥有。
- `up` 继续保留为显式 daemon 启动/托管命令；其余 CLI 动作通过 `localapi` 连接 shared daemon，并在 daemon 不可达时自动拉起同一用户后台进程。
- 在 `internal/localapi` 中增加基于 Unix socket / named pipe 的 `JSON-RPC` 控制面和独立 events / shell 流通道，供 CLI 和后续 GUI 共同消费。
- 把 `poc-v1-07-gui-wizard` 收缩成只消费 `06x` runtime 的 GUI/desktop change。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-headless-runtime`: 当前 POC v1 的 headless runtime、shared daemon、`localapi` RPC 与 CLI 闭环合同。

### Modified Capabilities

- `miopunch-poc-v1-gui-wizard`: 从 runtime owner 收缩为 runtime consumer。

## Impact

- 计划新增代码：`internal/pocv1/runtime/*`
- 计划重接线的产品入口：`cmd/miopunch/*`
- 计划复用但不再拥有 authority 的壳层：`internal/localapi/*`、`internal/task/*`、`internal/desktopbridge/*`
- 不包含 GUI 呈现细节、桌面包装、system service/tray 安装器或完整 Windows CLI 闭环。
- 不保留长期 HTTP `/api/v0` 兼容层或长期双栈迁移。
