## Context

现有仓库已经有 Linux-only 的 `poc-v1-cli-smoke`，也有 Windows/WSL 真机排查的长文档，但没有一条把“Windows 与 WSL 直接用 CLI 互测建网/入网”收拢成正式 smoke 的路径。用户当前不再接受 GUI 作为主验证方式，因此需要把验证焦点移到可复跑、可取证的 CLI 闭环上。

## Goals / Non-Goals

**Goals:**
- 用 CLI 直接验证 Windows/WSL 双向建网与 join。
- 复用现有 session bundle、`labctl` 和 `miopunch` CLI 语义。
- 对每次失败保留足够证据，便于后续按阶段拆解。

**Non-Goals:**
- 不扩展 GUI 测试。
- 不在这轮补完整失败矩阵。
- 不改变 runtime/join 协议本身。

## Decisions

### 1) Reuse the existing smoke execution model

新 change 不重新发明一套执行器，而是复用现有 session bundle + `labctl` 的产物/目录约定。这样可以直接使用现成的 `miopunch` 可执行文件、日志目录和 state 目录，减少额外变量。

### 2) Keep the smoke CLI-only and bidirectional

只测两条正向链路：
- Windows `init-network -> invite -> approve -> join`
- WSL `init-network -> invite -> approve -> join`

每条链路都要求另一侧完成 join。这样能同时覆盖两边的 LocalAPI、broker、state 和 report 路径。

### 3) Make diagnostics a first-class smoke output

每次运行都必须保存：
- CLI stdout/stderr
- `--report` 输出
- daemon log
- state snapshot / runtime snapshot
- run metadata

失败时以 `reason_code` / `facts` / `suggestions` 为主，不只看 exit code。

### 4) Use isolated bundle directories per side

Windows 和 WSL 各自使用独立的 extracted bundle / state 目录，避免旧进程、旧 state 或旧 report 污染交叉验证结果。

## Risks / Trade-offs

- [Risk] Windows/WSL 路径和 shell 转义容易出错。→ Mitigation: 统一在文档里列出原始命令模板，Windows 路径一律用引号保护。
- [Risk] 双向正向闭环在同一次 smoke 中会增加等待时间。→ Mitigation: smoke 保持只做最小正向闭环，不扩展失败矩阵。
- [Risk] 若 broker 不可达，整个 smoke 会失败得很早。→ Mitigation: 这是预期行为，必须保留清晰的 broker 失败事实。
