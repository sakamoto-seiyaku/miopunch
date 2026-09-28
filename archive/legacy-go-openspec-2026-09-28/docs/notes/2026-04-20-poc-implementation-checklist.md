# 2026-04-20 POC 实现清单（vertical slice）

> 目标：把 `docs/notes/2026-04-15-alpha-product-discussion.md` 的“已敲定语义”翻译成可执行的实现 checklist。  
> 范围：POC（join → ping → sh），不追求全功能与极致成功率。

## 0. 入口与验收（Freeze）

- 前置：joiner/admin 可出站访问 MQTT；STUN 仅用于提升成功率（缺失不阻断，但会降级诊断置信度）。
- 验收闭环：`join → ping → sh(tmux)` 成功；失败必须输出 `stage + reason_code + facts + suggestions`。
- 无数据面中心 relay（现在与未来都不做）；控制面允许 mesh 转发 + MQTT 兜底。

## 1. 必做 CLI（POC）

- `miopunch up`
- `miopunch ls`
- `miopunch invite [--mode approve|auto] [--uses N] [--expires 15m]`
- `miopunch approve <code>`
- `miopunch join <code-or-url>`
- `miopunch ping <peer> [-4|-6]`
- `miopunch sh <peer> [target] [-s session] [-4|-6]`
- `miopunch sh ls <peer> [target]`
- `miopunch revoke <peer> --dangerous`
- `miopunch install-system-daemon` / `miopunch uninstall-system-daemon`
- `miopunch reset`

约束：

- 除 `up/install-system-daemon/uninstall-system-daemon/reset` 外，其余命令默认只作为 LocalAPI client（不直接跑控制面/网络逻辑）。
- `sh` 依赖 `tmux`；目标侧缺少 `tmux` 必须失败并给可操作提示（不 fallback）。

## 2. up（daemon）职责边界（POC）

- 常驻：control-plane mailbox/presence、targets 探测、LocalAPI、HTTP 面板（可选）。
- 任务化：`invite/join/approve/ping/sh_ls/sh_attach/revoke_member` 走 task，统一阶段机与报告导出。
- 并发：同一 `(peer,target,session)` 单写者锁；锁 TTL 与 WS 活动绑定。

## 3. LocalAPI（CLI↔daemon）最小接口

承载（敲定）：

- Linux：unix socket（system `/run/...`；user `$XDG_RUNTIME_DIR/...`）
- Windows：named pipe（按 operator SID 收敛权限）

协议组合（敲定）：

- HTTP/JSON：资源与 task 创建/查询
- SSE：全局事件流 + task 事件流（统一；不做轮询）
- WebSocket：仅 `sh_attach` 的字节流（`Sec-WebSocket-Protocol: miopunch.sh.v0`）

最小路由（v0）：

- `GET /api/v0/status`
- `GET /api/v0/peers`
- `GET /api/v0/tasks`
- `GET /api/v0/tasks/<task_id>`
- `GET /api/v0/events`（SSE，全局）
- `POST /api/v0/tasks`（创建：`invite/join/approve/ping/sh_ls/sh_attach/revoke_member`）
- `GET /api/v0/tasks/<task_id>/events`（SSE）
- `GET /api/v0/tasks/<task_id>/report`（Markdown）
- `GET /api/v0/tasks/<task_id>/ws`（WS，仅 `sh_attach`）

输出契约（下限）：

- 任意失败：HTTP status 反映成败；响应体至少包含 `stage, reason_code, exit_code, message, facts, suggestions, request_id`。
- `reason_code/stage/term_id/exit_code`：POC 内不重命名，只新增；需要改名用 alias/deprecated。

## 4. 事件模型（SSE）

- 单一 SSE event（JSON body），用 `kind` 区分类型。
- 最小 `kind`：`snapshot` / `stage` / `fact` / `diagnosis` / `report_ready` / `done`。
- 断线重连：不做 `Last-Event-ID` 补发；重连先发 `snapshot`（可含最近少量时间线），再继续推送。

## 5. sh（remote shell）最小实现点

- 目标（Windows 被控端）仅来自：
  - `wsl:<distro>`：ConPTY + `wsl.exe`
  - `ssh:<name>`：ssh connector（VM 内装 `tmux`）
- 现场：`exec tmux new -A -s <session>`。
- 输入/输出：WS 二进制帧字节流透传；resize 走 WS text 控制消息（`winsize{cols,rows}`）。
- Ctrl-C：透传到目标侧；不作为本地强制断开。

## 6. 持久化（最小集 + 原子写）

目标：可恢复 + 幂等 + 可解释，不引入 DB。

- `identity/`：ed25519/x25519/TLS identity（见讨论文档约束）
- `net.json`：`net_id/net_secret/brokers_effective/contact_set/...`
- `governance/head_snapshot.json`
- `decls/decls.json`
- `invites/<invite_id>.json`：`uses_left + handled_requests(request_msg_id->response_ct_b64)`（仅 issuer admin）
- `reports/`：最近 N 次 task 报告（ring buffer）

写入：tmp→fsync→rename 原子更新；`reset` 仅清 `state_dir`。

## 7. install/uninstall system daemon（POC）

- `install-system-daemon`：需要管理员权限；复制二进制到稳定路径；创建 operator 组/绑定 operator SID；安装并启用系统服务。
- `uninstall-system-daemon`：移除 service/unit + 删除稳定二进制；**不清 state**（state 用 `reset`）。
- shell-only：允许普通权限前台 `miopunch up`（未来启用 TUN/组网能力时再要求管理员权限）。

## 8. HTTP 面板（POC，最小交付）

- 默认关闭；启用后只监听 `127.0.0.1`。
- 页面：`Status/Join/Shell` 三块（固定卡片）。
- 刷新：统一 SSE（不做轮询）。
- 写操作白名单：只允许创建 `invite/join/sh_attach` 三类 task；其余提示“请用 CLI”。
- 安全（POC 口径）：同源校验 + 只 loopback；预留 `ui_token` 钩子（未来支持非 loopback/容器访问时启用）。

## 9. 明确不做（POC）

- 文件传输
- 数据面中心 relay（公开/自建都不做）
- net_secret 轮换
- 数据面自动协商/降级（后续链路层大改时再定）

