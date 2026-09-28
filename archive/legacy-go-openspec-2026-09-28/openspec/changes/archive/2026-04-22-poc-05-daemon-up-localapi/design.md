## Context

仓库已完成 POC-01（拆分 lab/product 二进制）以及 POC-02..04 的控制面基础设施（wire format、mailbox/topic 派生、bounded flooding、RPC 时间语义与 invite/approve 幂等持久化等）。但 `cmd/miopunch` 侧仍缺少产品线所需的“常驻进程 + CLI”最小闭环：`miopunch up` 尚未落地，且大部分命令仍是占位返回。

本 change 聚焦把 “daemon `up` + LocalAPI（CLI↔daemon）” 跑通，并冻结可解释性输出契约，使后续：

- POC-06（`sh(tmux)` vertical slice、单写者锁、WS I/O）可以直接复用 LocalAPI/输出契约；
- POC-07（HTTP 面板）可以复用同一套 resources/tasks/SSE/WS，而不再发明新接口。

约束（POC 口径）：

- LocalAPI **只做本机 IPC**（Linux unix socket / Windows named pipe），不暴露为可被外网访问的 TCP listener。
- 安全边界以 OS ACL 为主；不自研“加密落盘/鉴权体系”。
- 输出契约（`stage/reason_code/term_id/exit_code` + `--format json` envelope）POC 内不重命名，只新增；改名必须 alias/deprecated 兼容。
- `sh_ls/sh_attach` 在本 change 只冻结接口/契约并提供 stub；真实远端 tmux/锁/交互在 POC-06 落地。

## Goals / Non-Goals

**Goals:**

- 落地 `miopunch up` 常驻进程：对外提供 LocalAPI v0，并实现“同机同一 operator 单实例”的互斥与可诊断输出。
- 冻结 LocalAPI v0 合同：transport、默认地址、访问控制（OS ACL + 固定 Host）、routes、SSE 事件模型、`sh_attach` WS 子协议。
- 冻结 POC v0 输出契约：稳定字段与 `miopunch.json.v0` 最小 envelope；LocalAPI 的 HTTP 错误模型与 `exit_code -> status` 粗分类映射。
- 纳入 system service 安装/卸载的最小行为契约（稳定路径、权限边界、state 目录形态、升级语义）。

**Non-Goals:**

- 不实现 HTTP 面板 UI/静态资源（POC-07）。
- 不实现真实 `sh_attach` 远端 PTY/ConPTY、tmux 现场与单写者锁（POC-06）。
- 不引入任何中心化数据面 relay；不改变 punching/dataplane 协议与行为。
- 不承诺 facts/suggestions 的完整 schema；只冻结顶层 envelope 与稳定字段命名规则。

## Decisions

### 1) 命令职责边界：CLI 默认是 LocalAPI client

- `miopunch up`：启动 daemon（前台运行，Ctrl-C 退出；不提供 `up -d/down`）。
- `miopunch install-system-daemon/uninstall-system-daemon/reset`：本机管理命令（不依赖 daemon）。
- 其余命令（`ls/invite/approve/join/ping/sh/...`）默认只作为 LocalAPI client：
  - 通过 `POST /api/v0/tasks` 创建 task；
  - 通过 SSE 跟随阶段机/诊断事件；
  - 以同一套输出契约渲染 stdout/stderr（含 `--format json`）。

理由：避免“CLI 与 daemon 两套实现”口径割裂；将可解释性与任务化作为单一事实来源。

### 2) LocalAPI transport 与默认地址：system 优先，user 兜底

- Linux（system service 模式，root 托管）：`/run/miopunch/localapi.sock`。
- Linux（前台开发模式）：`$XDG_RUNTIME_DIR/miopunch/localapi.sock`。
- Windows（system service 模式，LocalSystem 托管）：`\\\\.\\pipe\\miopunch\\localapi-<operator_user_sid>`。
- CLI 探测顺序：先 system LocalAPI；失败再 user LocalAPI。
- 互斥：`miopunch up` 启动时必须探测 system+user 两处地址；任一可达则判定“已在运行”并退出（输出可操作诊断与建议）。
- 遗留 socket/pipe 清理：若发现路径已存在但不可连通，则清理并重建；若可连通则视为已有实例（禁止双起）。

理由：对齐“前台开发 vs system service”两种形态，同时保证用户侧命令行为一致且可诊断。

### 3) 访问控制：OS ACL 为主 + 固定 Host 作为意图校验

- OS ACL（硬边界）：
  - Linux：unix socket 仅允许 `{root} + {operator group}` 访问（目录 0750、socket 0660 等）。
  - Windows：named pipe DACL 仅允许 `{LocalSystem} + {operator user}`。
- 固定 Host（意图校验）：LocalAPI 仅接受 `Host: local-miopunch.localapi`（其余拒绝并给出可解释错误）。

理由：避免引入复杂认证；同时降低“非预期客户端误连/误打到面板 listener”的风险。

### 4) API 形态：resources + tasks；长操作统一 task

- 资源接口（短操作）：`/api/v0/status`、`/api/v0/peers`、`/api/v0/tasks`、`/api/v0/tasks/<task_id>`。
- task（长操作）：
  - `POST /api/v0/tasks`：`{kind, args}` 创建；返回 `task_id`。
  - `kind`（POC v0）：`snake_case`；至少支持创建：`invite/join/approve/ping/sh_ls/sh_attach/revoke_member`。
  - `args`：JSON object；仅包含该 task 的最小参数（例如 `code/peer/ip_family/target/session`）。

理由：将阶段机/诊断/报告绑定到 task，便于 CLI/UI 统一消费。

### 5) SSE：单事件形态 + snapshot-first；不做增量补发

- SSE 端点：
  - `GET /api/v0/events`（全局）
  - `GET /api/v0/tasks/<task_id>/events`（单 task）
- 事件格式：单一 SSE event（JSON body），以 `kind` 区分：
  - 最小集合：`snapshot` / `stage` / `fact` / `diagnosis` / `report_ready` / `done`
- 连接建立后必须先发 `snapshot`；不支持 `Last-Event-ID`；断线重连后同样先发 `snapshot`。
- 心跳：使用 SSE 注释行（例如 `: ping`）。

理由：KISS，先冻结可用合同；实现上允许直接复发 `snapshot` 做节流（不要求 diff）。

### 6) WebSocket：仅用于 `sh_attach`，冻结子协议与帧语义（本 change stub）

- `GET /api/v0/tasks/<task_id>/ws` 仅允许 `kind=sh_attach` 的 task。
- `Sec-WebSocket-Protocol` 必须协商为 `miopunch.sh.v0`。
- 帧语义（冻结）：
  - binary frame：字节流透传（stdin/stdout）
  - text frame：控制 JSON（最小 `winsize{cols,rows}`；其余后置扩展）

理由：为 POC-06 的交互实现预留稳定接口；避免 POC-05 过早绑定具体终端实现细节。

### 7) 输出契约：稳定字段 + 最小 JSON envelope + HTTP 错误模型

- 稳定标识（POC Freeze）：`stage/reason_code/term_id/exit_code` 不重命名，只新增；必须改名→alias/deprecated。
- `--format json`：
  - 输出单行 JSON，`format` 固定为 `miopunch.json.v0`。
  - 顶层最小稳定字段：`format, task_id, kind, status, stage, reason_code?, exit_code?, facts, suggestions`（允许新增字段，不承诺完整 schema）。
- LocalAPI 错误响应体（下限）：`stage, reason_code, exit_code, message, facts, suggestions, request_id`。
- HTTP status（按 `exit_code` 粗分类映射，一刀切）：
  - `exit=2 → 400`
  - `exit=3 → 503`
  - `exit=4 → 403`
  - `exit=5 → 504`
  - `exit=6 → 409`
  - `exit=7 → 404`
  - `exit=1 → 500`
- `request_id`（冻结）：base32(raw,no-pad,16B)=26 字符；对 task 相关请求可令 `request_id == task_id` 以简化关联。

理由：让 CLI/UI/测试断言拥有稳定抓手；同时保持 schema 可演进（只冻结顶层下限）。

### 8) state/log 目录与最小 threat model：不自研加密落盘

- 前台开发模式（user state）：state/log 走用户目录；LocalAPI 走 `$XDG_RUNTIME_DIR`（Linux）。
- system service 模式（system state）：state/log 走系统目录（Linux `/var/lib/miopunch` + `/var/log/miopunch`；Windows `%ProgramData%\\miopunch\\<operator_sid>\\...`）。
- 落盘保护口径（冻结）：依赖 OS ACL/目录权限；不自研“加密落盘”。

理由：POC 目标是可用与可解释；落盘安全以最小可操作口径为主。

### 9) install/uninstall system daemon：稳定路径 + 不清 state

- 使用成熟库实现 systemd/Windows Service 安装/卸载（例如 `github.com/kardianos/service`）。
- 安装必须复制二进制到稳定路径后再注册服务：
  - Linux：`/usr/local/bin/miopunch`
  - Windows：`%ProgramFiles%\\miopunch\\miopunch.exe`
- `install-system-daemon`：install + enable + start（安装后应立即可用）；重复执行用于升级（覆盖稳定路径并尽量重启）。
- `uninstall-system-daemon`：stop + disable + uninstall，并删除稳定路径二进制；**不清 state**（state 用 `reset`）。
- operator 模型（冻结）：POC 只支持单 operator 用户（=执行 install 的 OS 用户）；切换 operator 通过 uninstall→reinstall。

理由：把后台化交给系统托管，避免手搓 daemonize；同时明确“服务 vs state”的边界。

## Risks / Trade-offs

- [权限/组未生效导致 LocalAPI 访问失败] → Mitigation：错误输出必须给出可操作建议（重新登录/开新 shell、加入 operator 组、`sudo` 兜底命令等）。
- [遗留 socket/pipe 被误删导致中断] → Mitigation：清理前先尝试连通；仅对“不可连通的遗留文件”执行清理重建。
- [SSE/WS 连接导致内存增长或 goroutine 泄漏] → Mitigation：为每连接设置上限/背压与退出条件；task 结束后主动关闭相关订阅；全局保留最近 N 个 tasks/report。
- [输出契约被冻结后演进困难] → Mitigation：只冻结顶层下限；新增字段用向后兼容扩展；改名必须保留 alias/deprecated。

