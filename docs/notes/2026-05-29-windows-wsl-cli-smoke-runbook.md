# Windows/WSL CLI Smoke Runbook

日期：2026-05-29

状态：计划文档。用于后续直接执行 Windows 与 WSL 的 CLI-only 建网/入网验证，不依赖 GUI。

## 1. 目标

- Windows 侧用 session bundle 的 `miopunch` CLI 建网并生成 invite。
- WSL 侧用 session bundle 的 `miopunch` CLI join 同一个 invite。
- 再反向跑一遍：WSL 建网，Windows join。
- 每次运行都保留 stdout、stderr、`--report`、daemon log、state snapshot 和 run metadata。

## 2. 运行约定

- 使用独立的 Windows bundle root 和 WSL bundle root。
- 每侧单独保存：
  - `logs/miopunch.log`
  - `logs/miopunch-desktop.log`（若存在）
  - `data/state.json`
  - `data/runtime_v1.json`（若存在）
  - CLI `--report`
  - stdout / stderr

## 3. 正向命令顺序

1. `up`
2. `init-network`
3. `invite --mode approve --uses 1 --expires 15m`
4. `approve <invite_code>`
5. `join <invite_code>`

## 4. 失败判定

- 只要 `join` 失败，就必须记录：
  - `stage`
  - `reason_code`
  - `facts`
  - `suggestions`
  - 相关 CLI stderr
- 不允许只用“return code != 0”结束排查。

## 5. 可执行性验证步骤

在真实 Windows/WSL 环境确认这个 smoke 可执行时，按以下顺序记录证据：

1. 分别确认 Windows bundle root 和 WSL bundle root 是独立目录，且每侧 `data/`、`logs/`、artifacts 目录可写。
2. 分别启动两侧 `miopunch up --session`，记录 daemon stdout/stderr 和 `logs/miopunch.log`。
3. 对 Windows -> WSL 方向运行 `init-network -> invite --mode approve --uses 1 --expires 15m -> approve <invite_code> -> join <invite_code>`，每一步保存 stdout、stderr 和 `--report`。
4. 对 WSL -> Windows 方向重复同一序列，使用新的隔离 state root，避免复用上一轮 membership 状态。
5. 每轮结束后保存 `data/state.json`、`data/runtime_v1.json`（若存在）和 run metadata，记录命令、时间、bundle commit、host 标识。
6. 如果任一 `join` 失败，按本文失败判定记录 `stage`、`reason_code`、`facts`、`suggestions`，然后按 signaling / broker / membership / session 分层继续拆。

## 6. 下一步

- 用 Windows + WSL 真实环境跑第一轮正向闭环。
- 如果失败，再按 signaling / broker / membership / session 分层继续拆。
