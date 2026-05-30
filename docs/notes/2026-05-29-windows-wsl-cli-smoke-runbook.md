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

## 5. 下一步

- 用 Windows + WSL 真实环境跑第一轮正向闭环。
- 如果失败，再按 signaling / broker / membership / session 分层继续拆。
