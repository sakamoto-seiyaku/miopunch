# Project Context

## Purpose

`miopunch` 当前主线是 `POC v1`：一个面试/demo-ready 的 P2P remote-control POC，通过不可信 MQTT 控制面协助两端发现/协商，并在 punched UDP path 上建立 secure session 和远程 shell。

当前项目目标：

- 让当前 POC v1 形成可解释、可演示、可复测的产品闭环。
- 保持 `control plane`、`UDP path establishment`、`secure session`、`shell/session lifecycle` 的边界清晰。
- 明确当前 gate：host checks + Android/Linux/GUI 真实 demo evidence。
- 保留旧 P0/P1/P2/MNT/XTCP/TCP Door-2 资料为历史/延期参考，但不把它们当作当前 active specs 或当前验证 gate。

## Tech Stack

- `Go`：主语言；CLI、daemon、runtime、tests 默认优先使用 Go。
- `OpenSpec`：用于需求、变更、约束和实施顺序管理。
- `MQTT`：当前 POC v1 控制面/信令承载；broker 不可信，peer-targeted payload 由 v1 wire/security 保护。
- `UDP`：当前 POC v1 唯一 P2P carrier；先 direct reachability，再 UDP punching fallback。
- `KCP + TLS 1.3 + yamux`：当前 POC v1 secure session recipe。
- `Desktop GUI`：当前默认桌面入口，消费 shared daemon LocalAPI/runtime contracts。
- `Android control-lite`：当前 phone-side control-only demo APK，包装 Android arm64 CLI payload。
- `Linux/Windows session bundles`：当前可发布/可演示资产。

## Project Conventions

### Code Style

- 遵循 Go 最佳实践与 idiomatic Go，优先简单、直接、可读的实现。
- 包、类型、函数命名必须贴合网络语义，避免模糊缩写。
- 错误处理必须显式，错误信息必须能帮助定位建链阶段和失败原因。
- 可观测性优先：用户在失败后应能知道失败发生在哪个阶段、看到了什么网络条件、系统做过哪些尝试。
- 涉及超时、取消、重试的接口优先使用 `context.Context`。
- 文档默认使用中文；代码标识符、包名、spec/change ID 使用英文。

### Architecture Patterns

- 当前 POC v1 的事实源在 `internal/pocv1/*`，尤其是 `internal/pocv1/runtime`。
- `internal/localapi` 是 shared daemon IPC/plumbing，不是独立产品语义事实源。
- GUI 和 CLI 是同一 runtime 的不同入口，不得重新定义独立 stage/reason/evidence 模型。
- `connectivity/` 和 `internal/punching/` 可作为 UDP punching 语义来源，但当前 POC v1 不恢复 TCP Door-2 或旧 XTCP gate。
- Runtime-owned UDP socket / owner / demux 是硬边界；punch/session 只能借用，不得关闭 Runtime UDP owner。

### Testing Strategy

- Docs-only / OpenSpec-only 变更至少跑 `openspec validate --all --strict`。
- 当前 POC v1 主线 host checks：
  - `go test ./...`
  - `go vet ./...`
  - `bash scripts/check_no_xtcp_imports.sh`
- 当前 POC v1 真实验证优先使用 Android/Linux/GUI demo evidence：
  - network create/join/approve
  - `ls`
  - `ping`
  - `sh ls`
  - interactive `sh`
  - selected UDP path facts and daemon/app logs
- VM lab gates 暂缓，不作为当前主线必过项；如需恢复，必须先用新的 POC v1 lab OpenSpec change 重定义。

### Git Workflow

- 非平凡能力或验证口径变化优先走 OpenSpec。
- 分支和提交应小而聚焦，不混入无关改动。
- 已完成并同步进 main specs 的 OpenSpec changes 应及时 archive，避免 active changes 与 main specs 分裂。
- 代码影响变更进入 mainline 时按当前 `AGENTS.md` 和 `$dev` 规则执行验证。

## Domain Context

- 当前“打洞”默认指 current POC v1 的 UDP direct-first + UDP punching fallback。
- 当前 POC v1 不承诺 TCP punching、centralized data-plane relay、VPP、完整虚拟局域网或完整产品安装体验。
- `frp xtcp`、P0/P1/P2、MNT-01/02/03、TCP Door-2 是重要历史和参考，但不是当前 active OpenSpec gate。
- MQTT 是控制面/信令，不是数据面 relay。
- Android control-lite 是控制端 demo，不是 Android shell target 产品线。

## Important Constraints

- 当前项目首先服务面试/demo-ready POC，而不是完整终端用户产品。
- 不允许因为实现方便偏离已记录的 POC v1 决策；偏离必须进入 docs/specs 并可追溯。
- 当前 POC v1 pathing 是 UDP-only；`tcp_only` 必须 explicit unsupported，不能静默 UDP fallback。
- 当前验证不运行旧 VM lab gates，直到新的 POC v1 lab spec 定义并恢复它们。

## External Dependencies

- MQTT broker：当前 POC v1 控制面信令入口。
- STUN servers：用于 UDP mapped address discovery。
- Android device + Linux/WSL host：当前真实 demo/evidence 关键环境。
- Desktop runtime/Wails stack：当前 GUI presentation/bridge 入口。
- Historical references: `frp/`, archived OpenSpec specs, and old lab artifacts remain available for comparison and future redesign.
