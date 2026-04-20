# 2026-04-15 Alpha/POC 讨论记录（临时）

> 状态：临时讨论记录（非最终 spec / 非 roadmap 承诺）。
>
> 目标：把本次讨论中**最终敲定**的方向与规则沉淀为可执行口径；后续实现以 change 为准。
>
> 约束：本文只写**语义/流程/规则/验收口径**，不写具体数据结构/字段/函数签名。

## 0. POC Freeze（实现前收口清单，已定稿）

- A1 成功标准/假设：以“场景+前置条件”验收；不写成功率 KPI（见第 5/9 章）
  - 前置：joiner/admin 可出站访问 MQTT；STUN 仅用于提升成功率（缺失不阻断）
  - 验收闭环：`join → ping → sh(tmux)`；失败必须给出 `stage + reason_code + facts + suggestions`
- A2 CLI↔daemon：LocalAPI=HTTP/JSON + SSE + WS；除少数命令外 CLI 只创建 task 并渲染 events（见 3.2）
- A3 持久化：文件树 + 原子写（tmp→fsync→rename）；无 DB；invite uses/幂等必须持久化（见 3.1.2/5）
- A4 密钥落盘：POC threat model=本机失陷=该节点失陷；只做权限/账号隔离（建议全盘加密）；keystore 后置（见 6.6）
- A5 daemon/service：`up` 前台；系统服务托管后台；`install-system-daemon` 需要管理员权限；`uninstall` 不清 state（state 用 `reset`）（见 3.2）
- B6 面板（POC）：默认关闭；启用后只监听 `127.0.0.1`；只做必要卡片；写操作白名单 `invite/join/sh_attach`；刷新统一 SSE（见 3.2）
- B7 稳定字段：冻结 `stage/reason_code/term_id/exit_code` 与 `--format json` 顶层字段；不重命名只新增；改名用 alias/deprecated（见 9.2.1/术语词典）
- B8 invite/join/approve UX：`invite` 默认只输出 code 并退出；`approve <code>` 监听并交付；默认非交互自动批准；issuer admin 负责持久化 `uses + handled_requests`（见第 5 章）

## 1. 产品定位（最终口径）

- `miopunch`：面向个人/小团队的、跨平台（Windows / Linux / Android）的**私有直连组网工具**。
- 核心差异点：**高度可解释性**（用户一眼能看懂“现在在做什么、为什么通/不通、当前走哪条路径”）。
- POC 不再是“打洞演示”；必须提供一个真实可用能力。
- `web` 只能作为辅助面（控制台/配对/状态展示）；**不能替代客户端**。
- 加密遵循业界最佳实践（复用标准库/成熟方案），不自创密码学。

## 2. Alpha/POC 首个能力：远程 Shell（敲定）

- 首个 POC 能力：**远程 Shell（PTY 透传）**。
- 不把“访问对端 HTTP 服务”作为首个主能力；文件传输不进入 POC 范围（后置）。
- 必须支持**纯 CLI** 使用方式（任意终端可用）；UI/web 仅作为辅助入口与解释面。

### 2.1 平台角色（敲定）

- 被控端（提供 shell 目标的一侧）：**Windows host、Linux**。
- 控制端（发起连接的一侧）：**Windows、Linux、Android**。
- Android：**仅控制端**（POC 不做 Android 被控常驻）。

### 2.2 Windows 被控端：目标与连接器（敲定）

- `miopunch up` 常驻运行在 Windows host（真网卡上），**不要求**在每个 WSL/VM 里跑 agent。
- Windows 对外暴露的 shell 目标仅来自 Linux 环境：
  - `wsl:<distro>`：WSL/WSL2 发行版
  - `ssh:<name>`：本机 VM（通过 SSH）
- 连接器：
  - WSL：**ConPTY + `wsl.exe`**（不要求 WSL 内启用 `sshd`）
  - VM：`ssh` 连接器（VM 内只需 `tmux`）
- 不支持（POC）：Windows 原生 `powershell/cmd` 作为 target。

### 2.3 会话语义：现场锚定在 tmux（敲定）

- “现场”定义：目标侧的 **`tmux session`**。
- 进入/恢复语义固定为：`exec tmux new -A -s <session>`（存在即 attach，不存在即创建；detach/exit 会结束本次 PTY，使 `miopunch sh` 干净返回）。
- 不支持“接管一个已经在跑的任意终端 tab/PTY”；只管理 `miopunch` 创建/管理的会话（即上述 tmux 现场）。
- 断线/控制端重启/被控端 `miopunch` agent 重启：都等价于 `tmux client` 断开；**重新 attach 即恢复现场**。
- 被控机系统重启：tmux 进程消失；**不保证现场仍在**（POC 不引入更重的恢复方案）。
- `miopunch` **不实现自己的 multiplexer**；直接依赖 `tmux`。
- 依赖（POC，敲定）：`tmux` 为硬依赖；目标侧缺少 `tmux` 时 `sh` 必须失败并给出可操作的安装/排查提示（不 fallback 到 screen/裸 shell）。
- Shell 交互语义（POC 暂定，KISS）：
  - 控制端进入 `miopunch sh ...` 后：本地终端切 raw 模式，stdin/stdout 以字节流透传
  - Ctrl-C 语义（POC，敲定）：Ctrl-C 透传到目标侧；不作为“本地强制断开”手势（避免改变远端程序语义）
    - `miopunch sh` 进入后只提示一次“如何退出/断开”（建议提示 tmux 默认 detach：`Ctrl+b d`；或在会话内 `exit`）
  - 必需：窗口大小变化（resize）同步到目标侧 PTY（保证 `vim/top/tmux` 等可用）
  - attach 成功后：默认不再向 stdout 打印诊断进度（避免污染交互）；诊断只写入日志/报告（若启用）
  - 断线语义：断线仅等价于 tmux detach；不杀 session；用户重跑 `miopunch sh ...` 即可恢复现场（POC 不做自动重连）
  - `sh_attach` 生命周期（POC，敲定，KISS）：
    - 一条 WebSocket attach 对应一个 `sh_attach` task；WS 关闭即 task 结束
    - 重连/刷新/再次进入：创建新的 `sh_attach` task（现场由 tmux 保证，不要求 task 可续）

### 2.4 单写者（敲定）

- 默认单写者：同一 `(peer,target,session)` 同时只允许一个控制端 attach。
- 其他 attach 默认报错 `in use`；`--steal/--force` 作为后续能力（POC 先不做）。
- 锁释放（POC 必需）：
  - 锁保活（POC，敲定）：以 WS 存活为准（任意 WS 帧/WS ping-pong 都算保活；空闲不释放）；超过 TTL 未见任何 WS 活动则自动释放（避免控制端崩溃/断网导致永久占用）
  - 建议：`interval=15s`，`ttl=60s`（具体值可后续调优）

### 2.5 运行模式：常驻 vs 临时（敲定）

- 常驻（owned devices）：
  - 被控端长期运行 `miopunch up`（开机自启），维护 targets / 会话列表 / mailbox。
- 临时（temporary devices）：
  - 临时机器下载二进制后用一次性授权加入，退出即清理；不要求长期驻留。

## 3. CLI 与配置（敲定）

### 3.1 配置自动加载（可省略）

- 配置是可选的；存在则自动加载。
- 配置文件格式（POC，敲定）：TOML（仅支持一种格式，避免多格式并存）。
- 命名风格（POC，敲定）：统一 `snake_case`；CLI flags 与 config key 使用同一概念命名（允许提供短别名，但不引入第二套语义）。
- 加载顺序：**先匹配先用（不 merge）**
  1. `--config <path>`
  2. `$MIOPUNCH_CONFIG`
  3. 二进制同目录：`miopunch.toml` 或 `config.toml`
  4. 默认路径：`os.UserConfigDir()/miopunch/config.toml`
- system service 模式（由 `install-system-daemon` 安装）：
  - service/unit 必须显式传入 `--config <system_config_path>`，避免依赖 root 的 user config dir（细节落地时实测校准）
  - system config 的建议位置：
    - Linux：`/etc/miopunch/config.toml`
    - Windows：`%ProgramData%\\miopunch\\<operator_sid>\\config.toml`

### 3.1.1 配置 schema（POC 最小，敲定）

目标：覆盖 POC 必需的“可用性 + 可解释性”，不引入过度配置；未配置的部分使用内置默认。

建议分组（TOML table）：

- `[defaults]`：人机交互默认值
  - `default_target`：当未显式指定 target 且存在多个时的默认 target（例如 `wsl:ubuntu` / `ssh:home`）
  - `default_session`：未显式 `-s` 时的默认 session（默认 `main`）
- `[resolver]`：**仅用于 STUN/MQTT 的域名解析**（不扩展到其它用途）
  - `mode = "auto"|"system"|"builtin"`：
    - `auto`（默认）：先用系统 DNS，失败再 fallback 内置 DNS
    - `builtin`：强制使用内置 DNS
    - `system`：强制只用系统 DNS（不 fallback）
  - `protocol = "tcp"`（POC 固定；不做 DoT/DoH；`udp53` 在部分网络环境下易被 RST/污染）
  - `servers = ["1.1.1.1:53","8.8.8.8:53","223.5.5.5:53","119.29.29.29:53"]`（默认值；可覆盖）
- `[control_plane]`：控制面入口与行为
  - `broker_profile = "global"|"cn"`：内置 MQTT broker 优先级（默认 `global`；仅在未显式配置 `brokers` 时生效）
    - 语义：`global/cn` 使用同一套内置 broker 集合，但优先级顺序不同（更贴合不同网络环境）
  - `brokers`：MQTT broker 列表（可省略→用内置默认；显式配置则忽略 `broker_profile`）
    - POC 内置默认 broker 列表见：`docs/config/miopunch.example.toml`
  - `stun_servers_global` / `stun_servers_cn`：内置 STUN 名单（可省略→用代码内置默认；用户配置则覆盖对应 bucket）
    - 说明：`cn/global` 仅作用于 STUN 观测到的公网映射视图（用于“选哪个公网映射更容易打洞/更低 RTT”），不影响 LAN/local candidates 的收集与交换
    - 若只想使用“单列表 STUN”（不关心 `cn/global`）：可只配置 `stun_servers_global` 并让 `stun_servers_cn=[]`（或两桶配置同一份列表）
- `[data_plane]`：数据面传输选择（不做自动协商/降级）
  - `data_proto = "kcp"|"quic"`（默认 `quic`）
  - `quic_cc = "bbr"|"brutal"`（仅 `data_proto="quic"` 生效；默认 `bbr`）
  - `brutal_up_bps` / `brutal_down_bps`：仅 `quic_cc="brutal"` 必填（否则视为配置错误）
- `[http_panel]`：本机面板
  - `enabled = true|false`（默认 `false`）
  - `listen_addr = "127.0.0.1:27400"`（默认值；仅允许 `127.0.0.1`）
  - `reports_keep = 20`（默认值；最近 N 次 task 报告）
- `[targets]`：目标连接器（被控端）
  - WSL：可同时存在多个 distro（等价于多个 targets）
  - VM/其它：通过 ssh shortcut 显式配置
  - `[targets.wsl.<name>]`：WSL target（可选，用于别名/过滤）
    - `distro`：实际 WSL distro 名（来自 `wsl.exe -l -q`）
    - 说明（POC）：默认仍支持直接使用 `wsl:<distro>`；配置主要用于“更短的别名/更友好展示”
  - `[targets.ssh.<name>]`：SSH target（必需显式配置）
    - `host` / `port` / `user` / （认证细节落地时按最佳实践收敛；POC 至少支持 key/agent）

补充（POC，敲定）：

- 仓库提供可直接修改的示例配置：`docs/config/miopunch.example.toml`（含 POC 内置的 broker 与 STUN `cn/global` 默认名单）。
- 用户未配置的字段仍以代码内置默认值为准；用户配置后以用户配置为准。

### 3.1.2 本地 state/log 目录布局（POC 建议，KISS）

目标：用最少的文件/目录满足“可恢复 + 可解释 + 幂等持久化”；不引入 DB 依赖（后置再评估）。

- `state_dir`（必须可写；权限收敛见 6.6）建议包含：
  - `identity/`：本机身份材料（POC 暂定文件格式；实现可调整但语义必须一致）
    - `ed25519_seed.b64`：Ed25519 私钥种子（`32B`）的 `base64url(no-pad)`（末尾允许换行）
    - `x25519_sk.b64`：X25519 静态私钥（`32B`）的 `base64url(no-pad)`（末尾允许换行）
    - `tls_cert.pem`：TLS 自签证书（X.509 PEM）；其证书公钥必须与 `ed25519_seed` 派生的 Ed25519 公钥一致（见 4.3 的“身份绑定”）
  - `net.json`：`net_id/net_secret/brokers_effective/contact_set/...`（最小可恢复集合）
  - `governance/head_snapshot.json`：当前治理链头 snapshot（owner-signed）
  - `decls/decls.json`：声明集合（approve/revoke…；全量 union；含 tombstone）
  - `invites/`：本机签发的 invite 的最小持久化（覆盖 invite 有效期；用于 `uses` 扣减与幂等 response）
    - `invite_id`（建议）：`base32(raw,no-pad, sha256(invite_topic)[:16])`（26 字符；仅用于文件名/索引；不泄露语义）
    - `invites/<invite_id>.json`（建议字段）：`{ invite_topic, expires_at, max_uses, uses_left, handled_requests{request_msg_id->response_ct_b64} }`
  - `reports/`：最近 N 次 task 的报告（ring buffer；按 `task_id` 文件名保存；N 由 `http_panel.reports_keep` 控制）
- `log_dir`（可选）：
  - `up.log`（+ 最多 1 个旧文件轮转；详见第 9 章“事件留存与日志”）

写入语义（POC 建议）：

- 所有关键状态写入应使用“写临时文件 → fsync → rename 覆盖”的原子更新模式（避免半写入导致无法恢复）。
- `reset` 只清理 `state_dir`（不清理二进制/服务/配置；卸载服务见 `uninstall-system-daemon`）。

### 3.2 最小命令集合（短、可记）

- 列 peer：`miopunch ls`
- 发码（admin）：`miopunch invite [--mode approve|auto] [--uses N] [--expires 15m]`
- 审批（admin，approve 模式）：`miopunch approve <code>`
- 入网（joiner）：`miopunch join <code-or-url>`
- 连通性：`miopunch ping <peer> [-4|-6]`
- 远程 Shell：`miopunch sh <peer> [target] [-s session] [-4|-6]`
- 列 targets/sessions：`miopunch sh ls <peer> [target]`
- 撤销成员（核按钮，admin）：`miopunch revoke <peer> --dangerous`
- 安装/卸载系统服务：`miopunch install-system-daemon` / `miopunch uninstall-system-daemon`
- 重置本机（变新节点）：`miopunch reset`
- 后置（语义已定义）：`miopunch gov propose/sign/apply`（owner-signed snapshot）
- 常驻运行（被控端/长期在线设备）：`miopunch up`
  - 语义：启动本机控制面 mailbox/presence/连接器探测等常驻能力；前台运行，Ctrl-C 退出（POC 不实现 `up -d` / `down` 这类自带 daemonize；后台化交给系统服务托管）
  - 权限（POC，敲定）：纯 shell/控制面模式允许以普通权限前台运行；未来启用 TUN/组网能力时再要求管理员权限
  - 进程模型（POC 暂定，KISS）：
    - 被控端要“可被连接/可被发现”，必须有常驻的 `miopunch up`
    - 其它命令（`ls/ping/sh/invite/join/approve/...`）永远保留 CLI 形态：默认作为 client 连接本机正在运行的 `miopunch up`
      - 实现约束（POC，推荐）：除 `up/install-system-daemon/uninstall-system-daemon/reset` 外，CLI 不直接执行控制面/网络逻辑；统一通过 LocalAPI 创建 task + 订阅 events + 渲染输出
      - 若未检测到 up：默认报错并提示先运行 `miopunch up`（与 TS 类似）
  - CLI ⇄ up 通信（LocalAPI，敲定）：
    - 只做本机 IPC，不暴露为可被外网访问的 TCP 端口
    - 形态：HTTP/JSON（便于跨平台/便于复用 `--format json`）
    - 承载：
      - Linux：unix socket
      - Windows：named pipe
      - Android：POC 不支持常驻 `up`（仅控制端 one-shot）
    - 默认地址（POC，敲定）：
      - Linux（systemd system service，root 托管）：
        - `/run/miopunch/localapi.sock`（或发行版等价的 `/var/run/...`）
      - Linux（前台开发模式：用户直接 `miopunch up`，不走 systemd）：
        - `$XDG_RUNTIME_DIR/miopunch/localapi.sock`
      - Windows（Windows Service，LocalSystem 托管）：`\\\\.\\pipe\\miopunch\\localapi-<operator_user_sid>`（POC 单 operator 用户）
    - 权限边界（POC，必须）：
      - Linux：unix socket 必须只允许 `{root} + {operator user}` 访问（例如目录 `0750` / socket `0660` + 组/ACL 收敛）
      - Windows：named pipe DACL 只允许 `{LocalSystem} + {operator user}` 访问
    - operator（POC，敲定）：
      - Linux：通过 operator 组授予“无需 sudo 管理 miopunch”的权限（例如 `miopunch-operators`）；LocalAPI socket 的 group/ACL 收敛到该组
      - Windows：operator 固定为“执行 `install-system-daemon` 的 OS 用户”（POC 单 operator 用户）
    - 请求级防护（POC，敲定）：
      - `Host`（意图校验，不作为主要安全边界）：
        - LocalAPI（unix socket / named pipe）：仅允许固定 Host（例如 `local-miopunch.localapi`）
        - HTTP 面板（`127.0.0.1`）：仅允许 `127.0.0.1`/`localhost`（与配置端口匹配）
      - `Origin`/`Referer`（HTTP 面板 listener 才需要；LocalAPI 主要依赖 OS 权限边界）：
        - 写操作与 WebSocket：必须携带 `Origin`，且必须同源（仅允许本机面板 origin）
        - 只读请求：若携带则必须同源；若不携带则允许
      - 面板写操作白名单（POC，敲定）：
        - HTTP 面板 listener：只允许创建 `invite/join/sh_attach` 三类 task；其余一律拒绝并提示使用 CLI
        - `GET /api/v0/tasks/<task_id>/ws` 仅允许用于 `sh_attach`
    - API 形态（POC，敲定）：
      - 资源 + task：
        - 短操作走资源接口（例如读取状态/列表）
        - 长操作统一以 task 承载（`join/ping/approve/sh_attach` 等）
        - 解释（更贴近直觉）：需要阶段机/诊断/报告的动作一律 task + SSE；只读快照用资源接口更直观（UI/CLI 直接 `GET`）
      - task_id（POC，敲定）：
        - 形态：随机不透明 ID（高熵随机；不编码时间/顺序）；用于引用/下载报告/文件名
        - 格式（POC v0，敲定）：`16B` 随机 → RFC4648 base32(raw,no-pad)；规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）
        - 输入容错：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符
        - 排序：task 列表按 `created_at` 排序；不依赖 task_id 可排序
      - 事件流（POC，敲定）：
        - 承载：统一使用 SSE（`text/event-stream`）推送 task 阶段机与诊断事件（POC 不做轮询）
        - 事件形态：单一 SSE event + JSON 体；用 `kind` 区分事件类型（便于扩展、避免多 event 名兼容问题）
        - 最小 `kind` 集合：`snapshot`（首次必发）/`stage`/`fact`/`diagnosis`/`report_ready`/`done`
        - 心跳：使用 SSE 注释行（例如 `: ping`），不引入额外事件类型
        - 断线重连（POC，敲定）：不做 `Last-Event-ID` 增量补发；重连后先发 `snapshot`（可包含最近少量时间线），再继续推送后续事件
      - LocalAPI 路由（POC 最小，敲定到语义）：
        - 统一前缀：`/api/v0/...` 用于 JSON/SSE；HTTP 面板页面走 `/...`（避免混杂）
        - 读接口（短操作，资源型）：
          - `GET /api/v0/status`：本机状态快照（含 `stage/reason_code` 摘要）
          - `GET /api/v0/peers`：已知 membership 列表（含 `last_seen/role/hints`）
          - `GET /api/v0/tasks`：最近 task 列表（按 `created_at`）
            - 返回最小字段（POC，敲定）：`task_id, kind, created_at, status(running|done), stage, reason_code?, exit_code?, peer?, report_ready`
          - `GET /api/v0/tasks/<task_id>`：单个 task 的当前状态（`running|done` + 摘要）
            - 返回字段：至少包含上述最小字段；done 时可附 `result`（按 kind 返回最小结果摘要）
        - 全局事件流（POC，敲定；面板 `Status` 用）：
          - `GET /api/v0/events`：SSE（`text/event-stream`）
            - 首次必发 `snapshot`（含 `status/peers/tasks` 三块摘要）
            - 后续增量：POC 允许直接再发 `snapshot`（不要求细粒度 diff；实现可做节流）
      - task（长操作）：
        - `POST /api/v0/tasks`：创建 task（`kind + args`），返回 `task_id`
          - `kind`（POC，敲定）：`snake_case`；与 CLI 命令语义对齐
            - POC 必做：`invite/join/approve/ping/sh_ls/sh_attach/revoke_member`
            - 后置预留：`gov_propose/gov_apply`
          - `args`（POC，敲定）：JSON object；仅包含该 task 所需的最小参数（例如 `code`/`peer`/`ip_family`/`target`/`session`）
        - `GET /api/v0/tasks/<task_id>/events`：SSE 事件流
        - `GET /api/v0/tasks/<task_id>/report`：Markdown 报告下载（仅在 `report_ready` 后可用）
          - 报告生成时机（POC，敲定）：task 结束时生成（包含握手/路径/端点对/transport/诊断；不包含交互内容）
        - shell I/O（WebSocket）：
          - `GET /api/v0/tasks/<task_id>/ws`：仅用于 `sh_attach`；WebSocket 双向字节流（用于浏览器内置终端与 CLI attach）
            - 协议（POC v0，敲定）：`Sec-WebSocket-Protocol: miopunch.sh.v0`
            - binary frame：PTY/ConPTY 原始字节流（client→server=stdin，server→client=stdout）
            - text frame：控制消息 JSON（最小：`winsize{cols,rows}`；其余后置扩展）
      - shell I/O：不复用 JSON/SSE；单独走 WebSocket “字节流”通道（PTY/ConPTY），但其阶段/诊断仍通过 task/SSE 给出
    - 错误模型（POC，敲定）：
      - 语义：HTTP status 必须反映成败；同时响应体必须携带 `stage/reason_code/exit_code`（便于 CLI/UI 同口径解释）
      - task：
        - 创建：`201 Created`（返回 `task_id`）
        - 运行中：`202 Accepted`
        - 成功：`200 OK`
      - 失败：按 `exit_code` 粗分类映射到非 2xx（KISS，一刀切）：
        - `exit=2` → `400`
        - `exit=3` → `503`
        - `exit=4` → `403`
        - `exit=5` → `504`
        - `exit=6` → `409`
        - `exit=7` → `404`
        - `exit=1` → `500`
    - LocalAPI 探测与互斥（POC，敲定）：
      - 探测顺序：CLI 先尝试连接 system LocalAPI；失败再尝试 user LocalAPI
      - 互斥：同机同一 operator 只允许 1 个 `up` 在跑
        - `miopunch up` 启动时必须探测 system+user LocalAPI：任意可达则判定“已在运行”并退出（并提示用哪个实例）
        - `install-system-daemon` 若检测到前台 `up` 正在运行应拒绝并提示先停止（避免双 daemon）
    - 实例与清理（POC，KISS）：
      - 同一用户下只允许 1 个 `miopunch up` 实例
      - 启动时若发现遗留 socket/pipe：先尝试连通；连通则判定“已在运行”；不可连通则清理并重建
    - 覆盖（开发/调试后置但预留）：
      - 允许用 `--localapi <addr>` 或环境变量覆盖默认地址（生产/普通用户不建议）
  - 命令执行模式（POC，敲定）：
    - Windows/Linux（支持 `up` 的平台）：
      - 基线：除 `up` 外的命令都要求本机 `up` 正在运行（LocalAPI 可达）
        - 若 `up` 不在运行/不可达：报错并提示先运行 `miopunch up`（或后续用 systemd/Windows Service 托管）
      - `reset`：允许在无 `up` 时执行；若检测到 `up` 正在运行则拒绝并提示先停止（避免并发踩状态）
      - `sh`：交互仍由 CLI 承担；`up` 负责连接与会话生命周期（锁/targets/sessions），CLI 负责渲染/交互
    - Android（POC）：
      - 不支持常驻 `up`
      - 命令一律 one-shot（控制端）；不承担被控端职责（因此不提供“长期在线/可被发现/可被连接”）
  - 系统服务托管（POC 必做，需管理员权限）：
    - 目的：让被控端长期在线；提供开机自启/崩溃拉起能力。托管的进程仍是 `miopunch up`（不引入另一套 daemon 入口）。
    - 命令（POC，必须）：
      - `miopunch install-system-daemon`：在本机注册系统托管（systemd / Windows Service），用于托管 `miopunch up`
      - `miopunch uninstall-system-daemon`：移除系统托管
    - 实现约束（POC，必须）：
      - 不手搓 systemd/Windows Service；依赖成熟开源库完成安装/卸载（例如 `github.com/kardianos/service`）
      - 安装时必须把二进制复制到稳定路径，再用稳定路径注册 unit/service（避免用户移动/覆盖原文件导致服务失效）
        - Linux：例如 `/usr/local/bin/miopunch`
        - Windows：例如 `%ProgramFiles%\\miopunch\\miopunch.exe`
      - 具体 unit/service 的细节（路径/权限/启动策略/日志）以最佳实践暂定，落地时以实测校准为准
      - `miopunch up` 始终保持“前台运行”的含义；POC 不提供 `miopunch service start|stop|restart|status`
        - 日常启动/停止/重启由平台原生命令完成（`systemctl` / `sc` / `launchctl`）
    - install/uninstall 的行为（POC，敲定）：
      - `install-system-daemon`：安装 + enable + start（安装后应立即可用）
      - 重复执行 `install-system-daemon`（用于升级）：
        - 覆盖稳定路径的二进制
        - 尽量重启 service 让新版本生效；若无法重启则明确打印平台命令让用户手动重启
      - `uninstall-system-daemon`：
        - stop + disable + uninstall
        - 删除稳定路径的二进制（只移除服务与稳定入口，不清理 state）
        - state 目录保持不动；若要清理 state，使用 `miopunch reset`
    - operator 组与权限（POC，敲定）：
      - Linux：安装时创建 operator 组（例如 `miopunch-operators`），并把“执行安装的用户”加入该组（使其无需 sudo 即可调用 LocalAPI）
        - 若当前终端未立刻获得新组权限：提示用户重新登录/开新 shell；并提供 `sudo` 作为兜底路径
    - Linux：systemd system service（默认，root；为未来 TUN/组网预留权限模型）
    - Windows：Windows Service（默认，管理员安装；服务以 LocalSystem 运行，但 LocalAPI 权限按 operator user SID 收敛）
  - state 目录（POC，敲定；细节实测校准）：
    - 前台开发模式（用户直接 `miopunch up`）：使用 user state（XDG/`os.UserConfigDir()`）
    - system service 模式（`install-system-daemon` 安装）：使用 system state（例如 Linux `/var/lib/miopunch`、Windows `%ProgramData%\\miopunch\\<operator_sid>\\`）
    - 安装时迁移（KISS）：仅当 system state 为空时，允许从 user state 迁移/复制一次；否则不自动覆盖
  - reset（POC，敲定）：
    - `reset` 必须在 `up` 未运行时执行（已在运行则拒绝）
    - `reset` 作用域按“当前有效 state”自动选择：
      - 若检测到 system state / system service 形态：重置 system state（需管理员权限）
      - 否则：重置 user state（普通权限即可）
  - HTTP 面板（POC，敲定）：
    - 启用方式：由配置显式开启（system service 模式下通过配置控制是否启动）；前台开发模式也允许用命令行参数覆盖（细节落地时实测校准）
    - 仅本机辅助 UI，不替代 CLI（跨平台一致：浏览器打开本机页面即可）
    - 监听范围：仅 `127.0.0.1`（POC 不支持对外监听）
    - 对外监听与认证（后置，留口子）：
      - 若未来需要容器/局域网访问：必须显式开启“非 loopback 监听”并引入 `ui_token`（或等价机制）保护所有写接口与 WebSocket
    - 端口（POC，敲定）：固定默认端口（例如 `127.0.0.1:27400`）；若端口占用则报错并提示在 config 中修改（不做自动换端口）
    - 启用提示（POC，敲定）：若面板已启用，`miopunch up` 启动时在 stdout 打印一次面板 URL（不自动打开浏览器）
    - 前端交付（POC，敲定）：内置最小静态页面（HTML+CSS+少量 JS），以 `go:embed` 随二进制分发；不引入 SPA 框架
      - 页面形态（POC，敲定）：单页 + 顶部 4 个 Tab：`Status / Invite / Join / Shell`
      - 导航（POC，敲定）：不做多页面路由；Tab 切换只做前端显隐
      - 终端渲染（POC，敲定）：使用 `xterm.js`（浏览器侧渲染器），只做最薄 glue（WS 连接 + resize + 断线提示）
      - 二维码（POC，敲定）：由前端 JS 生成（内置极小 qrcode 库），不依赖外网 CDN
    - 实时更新（POC，敲定）：统一使用 SSE（不做周期轮询）
      - `Status`：通过 `GET /api/v0/events` 获取全局快照/更新
      - `Join/Shell`：通过 `GET /api/v0/tasks/<task_id>/events` 跟随阶段机与诊断
      - 若 SSE 不可用：直接提示用户刷新/重试（轮询后置，不在 POC 实现）
    - 写操作防护（POC，敲定）：仅允许创建 `invite/join/sh_attach` 三类 task（面板 UI 按钮仍显示为 `invite/join/sh`）；其余写操作一律拒绝并提示使用 CLI；写操作与 WebSocket 必须携带 `Origin` 且同源（不做登录体系；CSRF token 后置）
    - Join 执行模型（POC，敲定）：异步 task
      - 点击 join 后立即返回“已开始”，页面通过 SSE 观察阶段机推进；断线/刷新可继续观察同一 task
      - join 成功/失败都有明确的 `reason_code` 与诊断摘要（与 CLI 同口径）
    - 报告下载（POC，敲定）：保留最近 N 次 task 的 Markdown 报告（ring buffer；N 先小值例如 20，可后续调优），仅提供报告下载入口；不提供日志文件下载
      - 索引：按 `task_id` 索引
      - 清理：写入新报告/启动时做 prune（超出 N 则删除最旧）
    - 功能边界（POC，敲定：只读 + 配对 + 终端）：
      - 只读：状态/peers/链路/targets/sessions/诊断摘要与报告下载入口
      - ID 展示（POC，敲定）：默认短显（前 8），并提供“一键复制全量”（`peer_id/net_id/task_id/...`）
      - 可操作：`invite/join`（用于配对/入网）
      - 可操作（POC 最小）：浏览器内置终端（`sh` 的一种渲染器/入口）
        - 不做复杂管理：不在面板里实现 `approve/ping`；`sh` 的 target/session 选择规则仍与 CLI 完全一致
        - 交互（POC，敲定）：不做 targets/sessions 下拉；只提供“打开 Shell”按钮 + 可选输入框（peer/target/session）
          - 若缺少 target 且发生歧义/失败：直接失败，并在诊断里展示可选 targets/sessions 与下一步（补全后重试）
    - 最小页面清单（POC，敲定）：
      - `Status`：
        - 本机摘要：`peer_id/net_id`（短显+复制全量）、当前 stage、active brokers、本机公网映射 v4/v6
        - peers 列表：online/last_seen/role/hints（默认不展开端点全集）
        - 最近 tasks：最近 N 个 task（running/done + reason_code）；提供 report 下载入口
        - 更新：SSE（`GET /api/v0/events`）
      - `Invite`：
        - 一键生成（默认值）；高级选项折叠：`mode/uses/expires`
        - 展示：`code` + `miopunch://join/<code>` + 二维码 + 一键复制
        - 若 `mode=approve`：明确提示下一步“在发码端运行 `miopunch approve <code>`”
      - `Join`：
        - 输入：`code-or-url`（支持粘贴 `miopunch://join/...`）
        - 执行：创建 task 并展示阶段机进度；失败展示诊断树摘要 + reason_code + 下一步
        - 更新：SSE（`GET /api/v0/tasks/<task_id>/events`）
      - `Shell`：
        - 输入：`peer`（必填），`target/session`（可选）；打开后进入 xterm（WS attach）
        - 断线提示：WS 断开=detach；重试即可恢复现场（tmux session）
        - 更新：SSE + WS（先 SSE 跟随阶段机，确认就绪后再 WS attach）
    - 可解释性呈现（POC，敲定；面板硬规则）：
      - 阶段机：固定 8 阶段列表（done/current/pending）+ 当前一句白话摘要；可选显示每阶段耗时
      - 失败呈现：固定诊断卡片结构
        - 一句白话结论 + `reason_code` + 置信度
        - 关键证据 3–6 条（facts）
        - 建议动作 ≤3（suggestions）
        - 一键复制（摘要/全量）
      - 报告交付：提供下载 `report.md` + “复制报告文本”（不做 Markdown 渲染器）
      - Web 终端键位（xterm.js，POC 口径）：
        - `Ctrl-C` 必须透传到远端（不作为复制快捷键）
        - 复制：选中/右键菜单；粘贴：右键/`Ctrl-Shift-V`
    - 输入与错误契约（POC，敲定；面板硬规则）：
      - peer 输入（Shell）：
        - 接受 `name` 或 `peer_id` 前缀（≥8，大小写不敏感）
        - 若歧义/过短：直接报错并提示去 `Status` 页复制全量 `peer_id`
      - target/session（Shell）：
        - 允许输入；target 支持 `wsl:<distro>`/`ssh:<name>`
        - 省略时按 CLI 同规则选择；歧义/不存在直接失败，并在诊断 facts 附“可选 targets/sessions”
      - join code（Join）：
        - 接受纯 code 或 `miopunch://join/<code>`
        - 解析失败必须提示常见原因（多余空格/分隔符、粘贴了错误类型的 code 等）
      - 错误展示层级：
        - 默认展示“可修复提示”（下一步动作）
        - 展开详情后展示：stage + facts + reason_code（不默认铺开底层原始错误）
    - 与 CLI 一致性与互操作（POC，敲定；面板硬规则）：
      - 每个可操作动作都提供“一键复制等价 CLI 命令”（`invite/join/sh`）
        - 失败时也必须提供“重试命令 + 推荐 flags/参数”的复制入口（与诊断树建议动作一致）
      - 配置/日志：仅展示路径并提供复制（不尝试自动打开目录/文件）
      - Shell：允许多个浏览器 tab 并存；每个 tab 都创建独立 `sh_attach` task
        - 若遇到单写者锁冲突：明确提示 `in use` + 建议（换 session/稍后重试）
      - 权限与危险操作：面板不提供 `reset`/治理变更等破坏性操作；仅保留 `invite/join/sh_attach`
    - invite 展示（POC，敲定）：二维码 + 明文 code + 一键复制（覆盖“扫码/粘贴”两种交付场景）
      - 二维码：面板在生成/展示 invite 时应提供 `miopunch://join/<code>` 的二维码（便于手机扫码）
      - code 分组展示（POC，敲定）：为便于人工核对，展示时对 code 做固定分组并加分隔符（例如每 4–8 个字符一组）；复制时输出“无分隔符的原始 code”
    - 展示形态（POC，敲定）：固定卡片 + 阶段机进度 + `reason_code` + 诊断树摘要；必要时提供报告下载（见“可解释性/报告导出”章节）
  - 日志与可观测性（POC，敲定）：
    - 统一策略：所有运行形态（前台 / system service）都写入文件日志；stdout 只输出摘要/必要提示
    - 日志位置（随 state 形态变化）：
      - 前台开发模式：user log（XDG/`os.UserConfigDir()` 下）
      - system service 模式：system log（Linux `/var/log/miopunch`；Windows `%ProgramData%\\miopunch\\<operator_sid>\\logs\\`）
    - 轮转：单文件 `10MB`，最多 1 个备份，覆盖最旧（POC 先按此固定值）
  - install-system-daemon 写入最小默认配置（POC，敲定）：
    - 仅写“必须持久化/平台相关”的项：`state_dir` / `log_dir` / `localapi_addr` / `operator` / service 运行参数
    - 网络相关参数（STUN/MQTT/DNS）仍允许使用内置默认；用户需要时再在 config 中覆盖
    - HTTP 面板默认不启用；用户按需在 config 中显式开启（仅支持本机 `127.0.0.1`）
  - operator 组未生效的错误提示（POC，敲定）：
    - 当 CLI 调用 LocalAPI 遭遇权限错误时，必须输出“可操作的诊断与修复步骤”
      - 例如提示加入 `miopunch-operators`、重新登录/开新 shell，并提供 `sudo ...` 兜底命令
  - 多用户与多 operator（POC，敲定）：
    - POC 只支持单 operator 用户（=执行 `install-system-daemon` 的 OS 用户）
    - 需要切换 operator：`uninstall-system-daemon` 后重新安装
- 生成入网材料：`miopunch invite [--mode approve|auto] [--uses N] [--expires D]`
  - 默认：`--mode approve --uses 1 --expires 15m`
- 入网：`miopunch join <code>`
  - 收到 `membership bundle` 后自动 bootstrap（按 6.4 的“2 个推荐 + 最多 2 轮 bootstrap_more”）
  - bootstrap 连上任意 1 个邻居即入网成功；若最终失败则仍落盘保存（便于后续重试）
- 审批（仅 `mode=approve`）：`miopunch approve <code>`
  - `miopunch invite` 默认只输出 code 并退出；由 `miopunch approve <code>` 在有效期内监听并交付；`uses` 用完或过期即退出
  - 默认非交互：收到 `join_request` 后展示 joiner 关键信息并直接交付（无需 `y/N` 二次确认）
  - 可选：`--interactive` 进入交互确认模式（POC 默认仍为非交互）
- 连通性验证：`miopunch ping <peer> [-4|-6]`
  - 验收口径：默认跑到 `transport.payload_exchanged` 即算成功；不进入 shell
  - 行为：尽量对齐标准 `ping`（POC 默认单次请求-响应，显示 RTT；连续多次后置）
  - （后置）连续多次：预留 `-c N`（POC 先不实现）
  - 传输栈选择（POC，敲定）：
    - 默认从 config 决定（例如 `data_proto=kcp|quic`、`quic_cc=bbr|brutal`）
    - 允许命令行临时覆盖（例如 `--data-proto/--quic-cc`）
    - CLI 友好别名（POC，敲定）：提供糖参数 `--kcp/--quic`、`--bbr/--brutal`（冲突时报错；本质等价于写 `--data-proto/--quic-cc`）
  - 超时预算（POC）：`3s`
- 重置本地状态（危险）：`miopunch reset`
  - 语义：清理本地持久化状态（含 identity key）→ 等同“新节点”
  - 确认：需要二次确认（例如 `--dangerous` + 确认短语）
  - 说明：这是**本机自毁式重置**；不涉及对其它 peer 的管理权限
- 进入/恢复 shell：`miopunch sh <peer> [target] [-s session] [-4|-6]`
- 列 target / session：
  - `miopunch sh ls <peer>`
    - 0 个 target：报错 + 提示先发现/配置 target
    - 1 个 target：直接列该 target 的 tmux sessions（输出头部显示 `target=<name>`）
    - ≥2 个 target：列 targets，并提示 `miopunch sh ls <peer> <target>` 查看 sessions
  - `miopunch sh ls <peer> <target>`：列该 target 的 tmux sessions
  - 实现与元数据边界（POC，敲定）：
    - `targets/sessions` 不在全网广播；不进入 `hello/capabilities`；仅在需要时由控制端对该 peer 发起点对点查询获取
    - `miopunch sh ls` 通过 LocalAPI 创建 `sh_ls` task（仅 CLI 可用；HTTP 面板不开放该写操作）
    - 结果允许缓存（TTL：后续实测定值）；查询失败时允许展示“上次缓存（可能过期）”作为提示
    - HTTP 面板不做 targets/sessions 的预拉取下拉；当 `sh_attach` 因 target 歧义/错误失败时，诊断 facts 中应附“可选 targets/sessions”与下一步指引（让用户补全再重试）

### 3.3 target 选择规则（隐藏类型、歧义才显式）

- target 来源：
  - WSL：distro 名（`wsl.exe -l -q`）
  - VM：config 中定义的 ssh shortcut 名
- 显式指定 target：
  1. 写了前缀 `wsl:<name>`/`ssh:<name>`：强制按该类型匹配
  2. 否则：按名字在全部 targets 里匹配（建议大小写不敏感）
     - 命中 0：报错 + 提示 `miopunch sh ls <peer>`
     - 命中 1：选中
     - 命中 ≥2：报错 + 要求用前缀消歧
- 省略 target：
  - 只有 1 个：直接选
  - ≥2：
    - config 有 `default_target` 且存在：选它
    - 否则：TTY 下交互选择；非 TTY 报错要求显式给 target

### 3.4 session 默认规则

- `-s <session>`：用户显式指定（对齐 tmux）
- 不带 `-s`：
  - 优先 config `default_session`（可按 peer/target 覆盖）
  - 否则默认 `main`

### 3.5 免手敲与桌面端 UX（建议）

- 可通过终端 profile/快捷方式把常用命令固化，实现“一键恢复到同一会话”（例如 Windows Terminal profile 直接跑 `miopunch sh <peer> ...`）。
- 桌面端 UI 若提供“进入/恢复会话”入口，优先拉起用户常用的终端软件；不强制要求内嵌终端。
- 桌面端 UI 应优先展示“当前可恢复/可进入”的 target 与 session，再由用户点选进入。
- 重命名语义：
  - session 重命名：沿用 `tmux` 原生命令/快捷键（POC 不额外发明 rename 子命令）
  - peer 别名 / VM shortcut / target 别名：通过 config 修改

### 3.6 peer 标识与发现（敲定）

- 每个节点同时具备：
  - 稳定 `peer_id`（POC v0，敲定）：
    - 语义：由身份签名公钥派生的短码（用于索引/路由；**非密钥**）
    - 生成：`peer_id = base32(raw,no-pad, sha256(ed25519_sign_pubkey)[:16])`
    - wire：规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）
    - 输入容错（POC v0，敲定）：解析时忽略大小写，并允许空格/短横线分组；输出一律规范化为大写无分隔符
    - 因此：重置 identity key = 新 `peer_id`（POC 不支持“原地换 key”）
    - 想回到同一网络：按新节点重新走 `invite/join/approve`
  - 人类可读 `name`（可在本机 config 中设置）
- `miopunch ls` 默认展示：`name` + 在线状态；`-v` 才展示 `peer_id`。
- CLI 中的 `<peer>` 解析规则：
  - 优先按 `name` 匹配
  - 若重名/歧义：要求使用 `peer_id`（全量或前缀）
    - 前缀规则（POC，敲定）：允许输入 `peer_id` 前缀（≥`8` 字符，大小写不敏感），且必须唯一命中
    - 若前缀过短或命中多条：报错并提示 `miopunch ls -v` 获取全量 `peer_id`
- 发现 vs 配置：
  - `miopunch ls` 以“在线发现”为主（来自 control-plane 的 presence/hello）
  - config 只承担本机参数 + 可选别名映射；不要求用户预先配置全网 peers
- `miopunch ls` 展示口径（POC）：
  - 默认展示全量 membership：online 在上，offline/unknown 在下
  - offline 显示 `last_seen` 相对时间（例如 `last_seen=3m ago`）
  - `-v` 才展开更多字段（`peer_id/role/hints`）

### 3.7 CLI 输出契约与退出码（POC，敲定）

输出总原则：

- 默认输出面向人（极度友好、可解释）；失败自动展开（见第 9 章）
- 以 `reason_code` 为主（可解释词典），退出码只做**粗分类**
- 不搞“两套 CLI”；仅提供输出格式开关（同一命令树）

输出约定：

- 正常（默认）：
  - 1 行摘要：`成功/进行中 + 当前阶段 + 目标`
  - 3–6 条关键事实（只给用户“能用来判断”的信息）
- 失败（默认自动展开）：
  - 1 行摘要：`卡在 <阶段>：<白话原因>`
  - 输出：`reason_code=<...>` + 关键证据 + ≤3 条建议动作
- 详细程度：
  - `--log-level info`（默认）：满足“10 秒内看懂”
  - `--log-level debug`：追加更多原始观测/事件（仍保持可解释结构，不倒一屏无结构日志）
- ID 展示（POC，敲定）：
  - CLI 默认短显（前 `8` 字符）：`peer_id/net_id/msg_id/task_id`（例如 `peer_id=ABCDEFGH…`）
  - 需要全量：使用 `miopunch ls -v`（查看全量 `peer_id`）或 `--log-level debug`（追加全量 ID）
  - HTTP 面板：默认短显，但提供“一键复制全量”（见 3.1 的面板约定）
  - 若报错提示“请使用 `peer_id` 消歧”：必须同时提示运行 `miopunch ls -v` 获取全量 `peer_id`
- 机器可读（POC，敲定；最小稳定 envelope）：`--format json` 输出单条 JSON（1 行）
  - 顶层字段（POC v0，敲定）：`format, task_id, kind, status, stage, reason_code?, exit_code?, facts, suggestions`
    - `format`：固定字符串（例如 `miopunch.json.v0`），便于演进
    - 允许新增字段；但上述字段名与类型不变
    - `facts/suggestions` 为数组；元素允许新增字段（不承诺完整 schema）
- 报告导出（敲定）：`--report <path>` 导出本次运行的 Markdown 诊断报告（详见第 9 章）
- 安全分享（敲定）：`--redact` 用于对外分享时脱敏（详见第 9 章）
  - 脱敏范围（POC）：隐藏所有 `IP:port`（包括本机映射、端点对）、以及 broker/STUN 地址；仅保留摘要与 reason_code/阶段/耗时等
  - 作用范围（POC，敲定）：影响默认文本输出、`--report`、以及 `--format json`；**不影响**本机日志落盘（外发请用 `--report --redact`）

各命令最小输出契约（文本默认）：

- `miopunch invite`：尽量多种可交付形式（附 `expires/uses/mode`），然后退出
  - 前置（POC，Win/Linux）：要求本机已运行 `miopunch up`；若未运行则报错并提示先启动 `up`
  - 纯 code（便于粘贴）
  - `miopunch://join/<code>`（便于点击/二维码）
  - `invite_brokers`（1–2 个，供用户确认本次入网将使用哪些 broker）
    - 若存在未固定到 `ip:port` 的 hostname：必须输出 WARNING（跨网络/DNS 解析差异可能导致入网失败；建议改用 `ip:port` 或显式 `control_plane.brokers`）
  - HTTP 面板：若启用则可展示二维码（CLI 不做 ASCII QR）
- `miopunch approve <code>`：
  - 前置（POC，Win/Linux）：要求本机已运行 `miopunch up`；若未运行则报错并提示先启动 `up`
  - 启动时打印：监听中 + `invite_brokers` + 剩余有效期 + 剩余 uses
    - 若 `invite_brokers` 含未固定 hostname：必须输出 WARNING（提示可能失败与建议）
  - 收到 `join_request`：打印 joiner 关键信息（name/peer_id 前 8/platform）+ “已批准/已交付”
  - 结束时打印：到期/uses 用尽/手动退出
- `miopunch join <code-or-url>`（接受纯 code 或 `miopunch://join/...`）：
  - 进度按阶段机推进（至少输出：已发请求/已收 bundle/正在 bootstrap/成功第一跳）
  - 成功：打印“已入网 + 首个邻居 + 当前 peer_id（前 8）”（并包含所用端点对/传输栈等关键事实）
  - 失败：自动展开并给出下一步建议（重试/换网络/检查 invite 是否过期等）
  - 可选：`--report <path>` 导出本次诊断报告；配合 `--redact` 可对外分享
  - 前置（POC，Win/Linux）：要求本机已运行 `miopunch up`；若未运行则报错并提示先启动 `up`
- 落盘时机（POC）：一旦收到并验证通过 `membership bundle` 就立刻落盘（便于后续 bootstrap 重试与恢复）
- `miopunch ping <peer>`：
  - 行为：尽量对齐标准 `ping`（单次请求-响应，显示 RTT；是否连续多次后置再加）
  - 成功：打印“payload_exchanged 成功”+ 家族/路径/所用端点对/RTT
  - 失败：输出卡住阶段 + reason_code + 建议
  - 可选：`--report <path>` 导出本次诊断报告；配合 `--redact` 可对外分享
- `miopunch sh <peer> ...`：
  - 在 attach 前按阶段机输出进度（简短）
  - 成功 attach：打印 `target/session` + 单写者状态 + 已选定端点对/传输栈，然后进入交互
  - 锁占用：输出 `in use` + 建议（换 session/稍后重试；`--force` 后置）
  - 可选：`--report <path>` 导出“建链 + attach”诊断报告（不含交互内容）；配合 `--redact` 可对外分享
- `miopunch revoke <peer>`（POC 最小治理，可用）：
  - 语义：发布一条 `revoke_member` 声明（永久 tombstone），使全网将该身份视为不可信（之后想回来只能换新 identity key 再 join）
  - 前置（POC，Win/Linux）：要求本机已运行 `miopunch up`；若未运行则报错并提示先启动 `up`
  - 权限（POC）：本机必须是 admin；否则直接失败并提示“该操作需要 admin”
  - 护栏：核按钮，必须二次确认（例如 `--dangerous` + 确认短语）
  - 约束（POC，KISS）：只允许撤销普通 member；若目标是 admin/owner 或无法确认其不是 admin/owner → 直接失败并提示“需要 owner-signed 治理快照”（POC 不实现该流程）
  - 成功：打印“已撤销 + peer 摘要（name/peer_id 前 8）”；说明“最终一致，离线节点通过后续 state_pull 收敛”
  - 失败：输出卡住阶段 + reason_code + 建议（例如先 `state_pull` 刷新视图；或确认 peer 是否为 admin）
- `miopunch gov propose ...`（后置实现，但输出契约先敲定）：
  - 语义：仅生成 `proposal.json`（不改变网络治理状态）
  - 前置：本机 `up` 正在运行 + 本机为 admin（否则 `LOCALAPI_UNAVAILABLE/GOV_PROPOSE_NOT_ADMIN`）
  - 输出：默认 stdout 输出 `proposal.json`（纯 JSON）；可 `--out` 写文件（用于交付离线 owner）
  - 失败：输出 reason_code + 建议（例如先 `state_pull` 刷新治理链头；或改用 `peer_id` 消歧）
- `miopunch gov sign ...`（后置实现，但输出契约先敲定）：
  - 语义：离线签名 `snapshot_body`，输出 `snapshot.json`（不需要 `up`；不产生网络副作用）
  - 护栏：必须二次确认（见 7.2 的 `SIGN <hash8>` 规则）
  - 输出：默认 stdout 输出 `snapshot.json`（纯 JSON）；可 `--out` 写文件（交付给在线 admin 代发）
  - 失败：输出 reason_code + 建议（例如 proposal 过旧需重新发起；或 `net_id` 不一致拒签）
- `miopunch gov apply ...`（后置实现，但输出契约先敲定）：
  - 语义：应用 `snapshot.json` 为新链头（owner 验签 + `prev_hash_b64` 对齐），并通过 `state_head` 变化触发最终一致收敛
  - 前置：本机 `up` 正在运行 + 本机为 admin（否则 `LOCALAPI_UNAVAILABLE/GOV_APPLY_NOT_ADMIN`）
  - 成功：打印新 `governance_head_b64`（短显+可复制全量）+ `height/owners/admins` 数量摘要
  - 失败：输出 reason_code + 建议（例如 `prev` 不匹配先 `state_pull`，再重新 apply）
- `miopunch reset`：
  - 输出：提示将清理本地状态并变成新节点；要求二次确认；成功后打印新的“下一步”（重新 join）

退出码（粗分类，跨平台稳定）：

- `0`：成功
- `1`：内部错误/未分类失败（bug 或未覆盖的错误路径）
- `2`：用法/配置错误（参数缺失、歧义 target、配置解析失败等）
- `3`：网络不可用（DNS/broker 连接、STUN 不可达等）
- `4`：认证/信任失败（解密失败、验签失败、已被 revoke、invite 失效等）
- `5`：超时（在预算内未得到所需回应/确认）
- `6`：对端拒绝/占用（被拒绝、单写者锁占用、对端不支持该能力等）
- `7`：对象不存在（peer/target/session 不存在或不可见）

`reason_code` → 退出码映射（POC，敲定）：

- 若该 `reason_code` 在文档中显式标注了 `exit=<N>`：以标注为准
- 否则按类别一刀切：
  - 用法/配置错误 → `2`
  - 网络不可用（DNS/broker/STUN）→ `3`
  - 认证/信任失败（解密/验签/revoke/invite 失效）→ `4`
  - 超时 → `5`
  - 对端拒绝/占用 → `6`
  - 对象不存在 → `7`
  - 其余未分类失败（bug/实现缺口）→ `1`

入网/状态同步最小 `reason_code`（POC 先覆盖高频）：

- `JOIN_CODE_INVALID`（exit=2）：入网码无法解析/格式错误
- `JOIN_INVITE_EXPIRED`（exit=4）：入网码已过期
- `JOIN_INVITE_USES_EXHAUSTED`（exit=4）：入网码已被用完（uses 用尽）
- `JOIN_REQUEST_TIMEOUT`（exit=5）：`join_request` 在 `expires` 窗口内未获回应
- `JOIN_BUNDLE_INVALID`（exit=4）：收到 `membership bundle` 但解密/验签失败
- `JOIN_BOOTSTRAP_FAILED`（exit=5）：拿到 `membership bundle` 后仍无法完成第一跳 bootstrap
- `STATE_PULL_TIMEOUT`（exit=5）：`state_pull` 超时（无法拉到最新长期态视图）
- `STATE_PULL_SIGNATURE_INVALID`（exit=4）：`state_pull` 回包不可验真（解密成功但验签失败/信任根不匹配）
- `STATE_PULL_GOVERNANCE_INVALID`（exit=4）：治理快照链不自洽（owner 签名链头/`prev_hash_b64` 不匹配）
- `STATE_PULL_TOO_LARGE`（exit=2）：`state_pull` 回包过大，超出 POC 控制面单消息上限（需要减少网络规模或重建网络）
- `STATE_PULL_NO_UPDATE`（exit=0）：`state_pull` 成功但无更新（拉到的视图不比本地新；仅用于解释/调试）
- `BOOTSTRAP_MORE_TIMEOUT`（exit=5）：`bootstrap_more` 请求超时（无法获得新的推荐 peers）

治理最小 `reason_code`（POC v0，先覆盖高频）：

- `REVOKE_NEEDS_DANGEROUS`（exit=2）：未提供二次确认（缺少 `--dangerous` 或确认短语不匹配）
- `REVOKE_NOT_ADMIN`（exit=4）：本机不是 admin（无权限执行该操作）
- `REVOKE_PEER_NOT_FOUND`（exit=7）：peer 不存在/不可见（本地视图找不到该 peer）
- `REVOKE_NEEDS_OWNER_SIGNATURE`（exit=4）：目标为 admin/owner，或无法确认其不是 admin/owner（需要 owner-signed 治理快照；POC 不实现）
- `REVOKE_NOOP_ALREADY_REVOKED`（exit=0）：该 peer 已被撤销（幂等；不重复产生副作用）

治理（owner-signed snapshot，后置实现但先预留 `reason_code`）：

- `GOV_PROPOSE_NOT_ADMIN`（exit=4）：本机不是 admin（无权限发起 proposal）
- `GOV_PROPOSE_CHANGE_INVALID`（exit=2）：proposal 变更非法（例如 owners 为空、试图移除最后一个 owner、未知 change_kind、参数不完整）
- `GOV_PROPOSE_PEER_NOT_FOUND`（exit=7）：proposal 引用的 peer 不存在/不可见（无法解析到公钥）
- `GOV_SIGN_PROPOSAL_INVALID`（exit=2）：proposal 文件无法解析/字段缺失/版本不支持
- `GOV_SIGN_PREV_MISMATCH`（exit=4）：`prev_snapshot_body` 校验失败（其 hash 与 `prev_hash_b64` 不一致；拒绝签名）
- `GOV_SIGN_NET_ID_MISMATCH`（exit=4）：proposal 的 `net_id` 与签名器本地 `net_id` 不一致（未显式 `--force`）
- `GOV_SIGN_NEEDS_CONFIRMATION`（exit=2）：缺少二次确认（确认短语不匹配；防盲签）
- `GOV_APPLY_NOT_ADMIN`（exit=4）：本机不是 admin（无权限 apply snapshot）
- `GOV_APPLY_SNAPSHOT_INVALID`（exit=2）：snapshot 文件无法解析/字段缺失/版本不支持
- `GOV_APPLY_SIGNATURE_INVALID`（exit=4）：owner 验签失败（不可信 snapshot）
- `GOV_APPLY_PREV_MISMATCH`（exit=4）：`prev_hash_b64` 与本地 `governance_head_b64` 不一致（拒绝，避免分叉；建议先 `state_pull` 或重新发起）
- `GOV_APPLY_NOOP_ALREADY_APPLIED`（exit=0）：snapshot 已是当前链头（幂等；不重复产生副作用）

本机/LocalAPI 最小 `reason_code`（POC 先覆盖高频）：

- `LOCALAPI_UNAVAILABLE`（exit=2）：本机 `up` 未运行或 LocalAPI 不可达（先启动 `miopunch up` 或检查 system service）
- `LOCALAPI_PERMISSION_DENIED`（exit=2）：无权限访问 LocalAPI（检查 operator 组/Windows DACL；必要时用管理员安装）
- `LOCALAPI_PROTOCOL_MISMATCH`（exit=2）：CLI 与 `up` 的 LocalAPI 版本不兼容（需要升级/重启到同版本）

## 4. 组网/控制平面（Alpha/POC 口径，敲定到事件与规则）

### 4.1 目标/边界

- 目标：形成可用的私有网络，让多个 peer 能互相发现、入网、恢复、管理。
- 不做（POC）：
  - 完整 ACL 体系/策略语言
  - 中心化公共数据面 relay（控制面可中继；后续最多做“网内数据面中继/转发”）
  - `net_secret` 轮换（P4 再做）
- 设计原则：
  - 无中心控制器/无官方托管/不要求自建 server
  - 入口（MQTT 等）**不可信**，只做 rendezvous/mailbox
  - 信令介质可替换（MQTT/其它）；POC 先以 MQTT 为默认实现
  - 语义 KISS，可收敛，可解释

### 4.2 信令入口（MQTT）与“不可枚举”

- 必然存在“网外可达入口”（新节点首次入网、完全失联恢复都需要）。
- 防扫描策略：
  - topic/收件箱命名空间从 secret 派生为**高熵随机**（建议 ≥128bit 有效熵）
  - 外界无法枚举 topic；只能猜，猜中概率近乎 0
- 常态控制面不依赖“全网广播大 topic”，而是 **peer inbox（点对点投递）**：
  - 每个 peer 仅订阅自己的 inbox（由 `net_secret` 派生命名空间 + `peer_id` 派生）
  - A→B：发布到 B 的 inbox
- inbox topic 派生（POC，敲定）：
  - 目标：无需中心分配 topic；网络内任意节点仅凭 `net_secret + peer_id` 即可推导出对端 inbox topic（便于恢复/换 broker）
  - `net_id`（POC v0，敲定）：`net_id = base32(raw,no-pad, sha256(net_secret)[:16])`
    - wire：规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）
    - 输入容错：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符
  - 方法（POC v0，敲定）：
    - 使用 `HKDF(net_secret, salt=net_id, info=...purpose...)` 派生 `16B` “名字材料”
    - 编码：RFC4648 base32(raw, no-pad)；写入 MQTT topic 时统一用小写（便于人工输入/一致性）
    - topic 中不明文包含 `peer_id`
  - 约束：
    - topic 命名必须具备 ≥128bit 有效熵（不可枚举）
    - 不同用途必须域分离（inbox/state/invite 等不得复用同一派生 info）
- MQTT 使用约定（POC，KISS）：
  - RPC 类消息：QoS1（至少一次；配合幂等去重）
  - best-effort 消息：QoS0（可丢、可延迟）
  - 不使用 retained（避免“老消息复活/误导”）
  - 不依赖 broker 的离线队列语义（POC 默认 clean session）
  - 传输层（POC，敲定）：默认使用 `tcp://host:port`（常见 `:1883`），**不强推** MQTT over TLS/WSS；需要时由用户显式配置（不改变控制面 E2E 安全语义）
- broker profile（POC，敲定）：
  - 内置：`global` / `cn`（同一套内置 broker 集合，不同优先级顺序）
  - 默认：`global`
  - POC 内置默认 broker 集合（示例/不保证长期可用；可能限流/变更/不可达）：
    - `mqtt.eclipseprojects.io:1883`
    - `broker.emqx.io:1883`
    - `broker.hivemq.com:1883`
  - 优先级（POC，敲定）：
    - `global`：`mqtt.eclipseprojects.io` → `broker.emqx.io` → `broker.hivemq.com`
    - `cn`：`broker.emqx.io` → `mqtt.eclipseprojects.io` → `broker.hivemq.com`
  - 对稳定性敏感：建议显式配置 `control_plane.brokers`（优先写 `ip:port` 或你自己可控的 broker）
- 多 broker 连接策略（POC，敲定；受限多活）：
  - 常驻 `up`：同时连接优先级最高的前 `N=2` 个 broker（不足则只连 1 个）
  - 入网（invite/join）：approver 与 joiner 对 code 内的 `invite_brokers`（最多 `2` 个）并发收发（同样依赖 `msg_id` 去重保证幂等）
  - publish/subscribe：对每个已连接 broker 都执行；靠 `msg_id` 去重窗口保证“重复投递不产生重复副作用”
  - 输出口径（POC，敲定）：
    - 成功态默认展示：active brokers（最多 2 个；`host:port`）+ `broker_profile`（若有）+ 连接状态摘要
    - 失败或 `--log-level debug`：展示完整尝试顺序 + 具体错误（含回退/补位原因）
  - 轮换（POC，KISS）：
    - 若 active broker 断开：按优先级从剩余候选中补位，维持“最多 2 个 active”
    - 若所有 broker 都不可达：控制面进入 `CP_CONNECT_FAIL`（可解释性建议用户更换 broker/网络）
- 可接受的元数据：
  - broker 仍可能看到连接层元数据（连接 IP、时间/频率/密文大小、订阅数量等）
  - POC 不做 cover traffic / 代理 / Tor 等强隐匿

### 4.3 控制面消息安全（必须）

- 所有控制面消息：**端到端加密（AEAD）+ 签名认证**。
- 允许使用的标准方案：
  - identity key（POC v0，敲定）：Ed25519（签名）+ X25519（静态 ECDH）
  - 签名：Ed25519
  - ECDH：X25519 + HKDF
  - AEAD（POC，敲定）：固定 `AES-256-GCM`（不做算法协商）
- 派生与域分离（POC，敲定）：
  - 不直接复用 `net_secret` 作为任何 AEAD key；所有用途通过 HKDF 域分离派生（以固定 `info` 前缀区分用途与版本）
  - `info` 命名约定（POC v0，敲定）：`miopunch/v0/<purpose>`（ASCII；purpose 用点号分层；必须稳定）
    - 示例（非穷举）：`topic.inbox`、`topic.presence`、`topic.state`、`aead.ctrl.group`、`aead.ctrl.pairwise`、`aead.ctrl.invite`
  - 示例（概念级）：`ctrl_group_aead_key`（全网可读）、`ctrl_pairwise_*`（点对点私密）、`topic_name_key`（topic 命名）
- 点对点私密消息（仅收件人可读，POC，敲定）：
  - 使用 X25519 **ephemeral-static（每条消息）** 做 ECDH，HKDF 后得到一次性 AEAD key
  - payload 仍需 Ed25519 签名以绑定发送者身份（避免“能解密但不知是谁发的”）
- nonce/抗重放（POC，敲定）：
  - AEAD nonce 使用随机值（不引入单调计数器）
    - 长度（POC v0，敲定）：AES-GCM 统一使用 `12B` nonce
    - 若需要进入 JSON：字段名 `nonce_b64`，编码为 `base64url(no-pad)`
  - 抗重放依赖：`msg_id` 去重窗口 + `created_at_unix_ms` 过旧丢弃（见 4.7 的去重规则）
- broker/中继只能重放/丢包/DoS，不能伪造“有效管理动作”。
- 数据面（payload）加密口径（POC，敲定）：
  - QUIC：使用 QUIC 内建 TLS 1.3（端到端）
  - KCP：在 KCP 上跑 TLS 1.3（端到端）
  - 身份绑定（POC，敲定；两者一致）：TLS 使用自签证书做双向认证；对端证书公钥必须与 control-plane membership 中该 peer 的 identity 公钥匹配（防 MITM）
  - 生命周期（POC，敲定；两者一致）：证书首次生成即落盘复用；reset=新 identity=新证书

### 4.4 “全网可读” vs “仅收件人可读”（敲定）

> “全网可读”不等于明文；含义是“所有成员可解密”（对外仍是密文传输）。

- 全网可读（成员可解密）：用于长期态信息
  - membership/声明集/治理快照头/`presence`/reachability hints 等
- 仅收件人可读：用于敏感短期态
  - STUN 结果、candidate 集、端点/端口、打洞证据等
- 转发不需要解密敏感载荷（pairwise 内层）：
  - 转发节点只需要读取外层路由头（`dst_peer_id`、`msg_id`、`hop_limit` 等）即可完成转发
  - 外层路由头由“成员可解密”的 group AEAD 包裹；转发节点不需要解密内层 pairwise 载荷

### 4.5 传输策略：mesh 优先 + MQTT 兜底（敲定）

- 只要已有任意 dataplane 邻居链路：控制面消息优先走网内转发。
- 仅在以下情况下使用 MQTT：
  - 未入网/无任何邻居
  - 直连/邻居转发超时（POC 阈值：`1s`；仅对 RPC 做 MQTT 兜底）
  - bootstrap/recover 的 request-response 类消息

POC 路由策略（最小）：

- 受限泛洪（bounded flooding，POC 固定 `H=3`）：
  - `hop_limit ∈ [0,3]`；若收到 `hop_limit>H` 则直接丢弃（debug 记录）
  - 发送方：当需要网内转发时，设置 `hop_limit=H` 并发给所有邻居（邻居数由 6.5 的 k 约束）
  - 转发方：仅当 `hop_limit>0` 才允许转发；转发前必须先 `hop_limit--`；不得回传给来源邻居；不得修改 `dst_peer_id`
  - `msg_id` 去重（带 TTL 窗口）
  - 适用范围（POC）：仅用于“已入网消息”（外层 group wrapper）；invite 阶段消息不转发
    - 是否转发只由 `hop_limit` 决定（presence/keepalive/解释性 sidecar 默认 `hop_limit=0`）
- 限流与队列上限（POC，敲定到最小口径；只为防放大/DoS，正常情况下几乎不触发）：
  - 转发永远是 best-effort：不得因为“转发堆积”阻塞本机发起的 RPC（`join/state_pull/bootstrap_more/ping/sh`）
  - 发送队列必须有硬上限（避免内存放大）：
    - `forward_queue_max = 1024`（条；仅统计“需要转发的消息”）
    - 超过上限：直接丢弃新到的“待转发消息”，并在 debug 记录 drop 计数（不回传错误，不改变安全语义）
  - 丢弃优先级（KISS）：
    1) 先丢弃 best-effort（presence/解释性 sidecar）
    2) 再丢弃“转发的 RPC”
    3) 本机发起的 RPC 保留最小发送预算（实现可用独立队列/独立 token bucket）
  - 可解释性（建议最小 facts）：若在一个 task 的预算窗口内发生过转发丢弃，应在报告 facts 里附 `mesh_forward_drops=<n>`（默认不在 stdout 刷屏）
  - 配置（POC，敲定）：上述上限暂不提供可配置项（避免配置面膨胀）；若未来压测发现需要调参，再引入显式 config key

### 4.6 密钥分层（最终口径）

- `invite_secret`（PSK/邀请码）：
  - 仅用于派生临时入网入口（invite topic/mailbox）与 join 阶段加密/认证
  - 可设 expires/max_uses；轮换＝生成新 invite
  - **永远不等于 `net_secret`**
- `net_secret`：
  - 网络根 secret，用于派生入网后稳定的控制面命名空间与加密 key
  - **仅在入网批准（或 PSK 自动批准）后交付给 joiner**

### 4.7 控制面协议 v0（消息族 + 可靠性，敲定）

目标：

- 让 join/bootstrap/presence/state-sync/打洞协调都能在“mesh 优先 + MQTT 兜底”下工作
- 入口/中继不可信：只能丢包/重放/DoS，不能伪造“有效动作”
- “最终一致”：长期态靠 push+pull 收敛，不追求强一致

通用包络（概念级）：

- 线格式（POC v0，敲定）：
  - MQTT/mesh 搬运的都是 **bytes**（密文）；broker/中继只见密文，不见明文结构
  - 外层密文 framing（POC v0，敲定）：
    - `v(1B) || nonce(12B) || ct`
    - `v=0`：AES-256-GCM；`ct` 含认证 tag
  - 解密后明文统一使用 **UTF-8 JSON**（KISS；便于 debug 与可解释性）
  - JSON 包络（POC v0，敲定到字段名；以下均为“解密后明文”）：
    - 已入网消息（group wrapper 明文）：
      - 顶层：`{ proto_version, route, signed }`
      - `proto_version`（int，必需；POC v0 固定为 `0`）
      - `route`（转发可变；仅 `hop_limit` 允许被转发节点递减）：
        - `dst_peer_id`（string，必需）
        - `msg_id`（string，必需）
        - `hop_limit`（int，`0..3`，必需；POC 固定 `H=3`；超过 `H` 丢弃）
        - `created_at_unix_ms`（int64，必需）
        - `expires_at_unix_ms`（int64，可选；仅 RPC request 必需）
      - `signed`（签名绑定 sender 与动作）：
        - `sender_peer_id`（string，必需）
        - `kind`（string，必需；命名约定为 `snake_case`）
        - `in_reply_to`（string，可选；仅 RPC response 必需）
        - `body`（object，必需；pairwise 时承载 `pairwise` 载荷）
        - `sig_b64`（string，必需；Ed25519 signature，base64url no-pad）
      - pairwise 内层（仅收件人可读，放在 `signed.body.pairwise`）：
        - `epk_b64`（string，必需；X25519 ephemeral public key）
        - `nonce_b64`（string，必需；AEAD nonce；AES-GCM 固定 `12B` 随机；base64url no-pad）
        - `ct_b64`（string，必需；inner ciphertext）
    - invite 阶段消息（POC v0，敲定）：
      - topic 即路由；不引入 `route/hop_limit`
      - 顶层：`{ proto_version, msg_id, created_at_unix_ms, expires_at_unix_ms?, kind, body, sig_b64? }`
      - `proto_version`（int，必需；POC v0 固定为 `0`）
  - 编码约定（POC v0，敲定）：
    - 所有以 `_b64` 结尾的字段统一使用 `base64url(no-pad)`（例如 `sig_b64/epk_b64/nonce_b64/ct_b64/...`）
    - 解析容错：允许带 padding；允许 standard base64（`+/`）与 base64url（`-_`）
    - 输出规范化：统一写 `base64url(no-pad)`（避免同义字符串造成比较/签名差异）
  - 签名覆盖（POC，敲定）：`dst_peer_id + msg_id + created_at_unix_ms + expires_at_unix_ms? + sender_peer_id + kind + in_reply_to? + body`
    - 覆盖 `dst_peer_id`：防止成员“重定向/改收件人”（不改签名即可改投递目标），保证审计/解释一致
    - 不覆盖 `hop_limit`：转发节点需要递减并重封装外层 wrapper
    - 投递方式可变（mesh / MQTT 兜底）不依赖路由字段变更
    - 签名输入字节（POC v0，敲定）：
      - 把上述“签名覆盖字段”组成固定结构体（字段顺序固定；禁止 `map[string]any`；`body` 也必须是固定 struct/slice）
      - `transcript = json.Marshal(该结构体)`
      - `sig = Ed25519.Sign(priv, sha256(transcript))`；写入 `sig_b64`
	  - `canonical_json`（POC v0，敲定）：
	    - 用途：`state_head`、`governance snapshot_body hash`、声明集合 head 等所有“要做 sha256 的结构体”都必须可确定性编码
	    - 定义：`canonical_json(x) = json.Marshal(x)`（UTF-8；固定 struct 字段顺序；禁止 `map` 参与哈希输入）
	    - 集合语义（set）必须先排序：
	      - `elem_hash = sha256(canonical_json(elem))`（32B）
	      - 对 `elem_hash` 按字节字典序排序
	      - `set_head_b64 = base64url(no-pad, sha256(concat(elem_hashes)))`
	    - 声明集合（`decls`，POC v0，敲定）：
	      - `decl` 元素是长期态：**不包含**路由字段（`dst_peer_id/hop_limit/...`）
	      - 最小结构：`{ msg_id, created_at_unix_ms, issuer_peer_id, kind, body, sig_b64 }`
	      - 权限约束（POC，必须）：
	        - `approve_member` / `revoke_member` 的 `issuer_peer_id` 必须属于当前治理快照中的 admin 集合；否则视为无效并忽略
	        - `revoke_member` 不允许目标为 admin/owner；若目标在 admin/owner 集合中，视为无效并忽略（应改走 owner-signed snapshot 链做 demote/remove）
	      - `decls_head_b64`：对 `decl` 做 set head（见上）
	      - kind（POC）：
	        - `approve_member`：增加/确认成员
	          - `body` 最小：`{ member_peer_id, member_name, ed25519_pub_b64, x25519_pub_b64, v4_hint, v6_hint }`
	        - `revoke_member`：永久 tombstone（优先级最高）
	          - `body` 最小：`{ member_peer_id, reason? }`
	      - 收敛（POC）：按 set-union 合并；`revoke_member` 永久不可逆（需要回来＝换新 identity 再 join）
	  - 演进：依赖 `proto_version` 进行兼容；后续可将 v1 切到 protobuf/CBOR（不影响 v0）
- 路由头（外层，POC，敲定到语义）：
  - 最小集合：`dst_peer_id`、`msg_id`、`hop_limit`、`created_at_unix_ms`（必需）；`expires_at_unix_ms`（仅当需要）
  - sender 身份不放在路由头里；sender 必须在可验真的载荷中以签名表达（避免“路由头泄露更多元数据”）
  - 路由头位于“成员可解密”的外层 wrapper 中（用于转发递减 `hop_limit`），不对 broker 明文暴露
  - 接收方规则（必须）：验签通过后必须检查 `dst_peer_id == self_peer_id`；不匹配则丢弃（防误投递/重定向）
  - 转发方规则（必须）：不得修改 `dst_peer_id`；只允许递减 `hop_limit` 并重封装外层 wrapper；不得改动被签名的载荷
- 版本：携带 `proto_version`；未知版本/未知 kind → 安全忽略（不阻断主流程）
- 加密层级（POC，敲定到语义）：控制面消息采用“外层 group wrapper + 可选内层 pairwise”的两层模型（invite 阶段例外）
  - 外层（成员可解密）：`net_secret` 派生 group AEAD，承载路由头与转发所需的最小信息（并允许转发节点递减 `hop_limit` 后重封装）
  - 内层（仅收件人可读）：对敏感短期态，再用双方 identity 的 ECDH（X25519+HKDF）派生 pairwise AEAD 加密；转发节点不需要解密内层
  - 全网可读（成员可解密）：不再额外加 pairwise 内层；载荷内仍必须带 sender 签名（防成员伪造）
  - invite 阶段：未持有 `net_secret` 时，用 `invite_secret` 做 AEAD（`membership_bundle` 仍端到端加密给 joiner）；invite 消息不走 mesh 转发
- 大小上限（POC，敲定）：单条控制面消息的解密后载荷硬上限 `256KiB`；超过视为异常并丢弃（防 DoS/避免内存放大）
- `hop_limit`（POC，敲定）：
  - 取值：`0..3`（POC 固定 `H=3`）；若收到 `hop_limit>H` 则直接丢弃（debug 记录）
  - `0`：不得转发（只允许直达投递/或走 MQTT 兜底）
  - `>=1`：允许转发；每转发一次必须先 `hop_limit--`，直到减为 `0` 为止（需要重封装外层 group wrapper）
- 允许转发的消息范围（POC，敲定，KISS）：
  - 仅“已入网消息”（外层 group wrapper）可被转发
  - invite 阶段（`join_request/membership_bundle`）不得转发（只走 MQTT）
  - 默认不转发：`presence/keepalive/解释性 sidecar`（`hop_limit=0`）
  - 其余点对点 RPC/协调消息：当“未直达 dst”时默认使用 `hop_limit=H` 做受限泛洪；RPC 超时再走 MQTT 兜底
- 可验真条件（统一口径）：
  - 能解密（满足对应的可读范围）
  - 且签名验证通过（绑定到 sender 的 identity；invite 阶段是“证明自己持有该 key”，不代表已入网）

传输与路由（POC 最小）：

- mesh-first：若存在任意 dataplane 邻居链路，控制面消息优先走 mesh
  - 语义澄清：`dst_peer_id` 是“最终收件人（ultimate destination）”，不是“下一跳（next hop）”
    - 下一跳由本机投递策略决定（直达/邻居转发/MQTT 兜底），但 `dst_peer_id` 不变（已被签名绑定）
  - 直达优先：若已存在到 `dst_peer_id` 的 dataplane 邻居链路，则直接投递给对端
  - 否则：按 4.5 的受限泛洪进行网内转发尝试
  - RPC 兜底：若 `1s` 内未获得可验真回应，则再通过 MQTT 投递到对端 inbox（A→B 发布到 B 的 inbox topic）
- 无邻居：直接通过 MQTT 投递到对端 inbox
- 网内转发（POC）：受限泛洪（见 4.5），仅依赖外层路由头；无需解密内层敏感载荷即可转发
  - 转发节点必须先成功解密外层 group wrapper，才能读取/递减 `hop_limit` 并继续转发；解密失败则丢弃（避免转发“不可验真”的垃圾数据）
  - 转发节点只做“路由/去重/限流”，不解析内层载荷、不做 sender 验签、不刷新 `last_seen`（避免过度耦合与额外泄露）

去重与顺序：

- `msg_id` 全局唯一（POC v0，敲定）：
  - 本体：`16B` 高熵随机（`crypto/rand`）
  - wire：RFC4648 base32(raw, no-pad)，规范输出大写；解析时大小写不敏感
  - 长度：固定 `26`；字符集 `[A-Z2-7]`
  - 输入容错（POC v0，敲定）：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符
- 接收端维护去重窗口（LRU + TTL）
  - best-effort：重复消息直接丢弃
  - RPC request：重复消息**不得**直接丢弃（通常表示“上次 response 丢了”）；应按“请求-响应幂等规则”重发 response（见下节）
  - 窗口参数（POC v0，敲定）：
    - seen：容量 `8192`，TTL `10m`（重启清空）
    - handled RPC requests：容量 `1024`，TTL ≥ request 有效期窗口，且最小 `10m`
      - POC：`invite/approve/join` 相关的 handled 记录与 `uses` 扣减必须最小持久化（由签发者 admin 节点负责；覆盖 invite 有效期），避免重启导致重复扣 `uses`/重复交付 `membership_bundle`
      - 其它 RPC：允许重启清空（KISS）
  - 日志：默认静默（避免刷屏），`--log-level debug` 可记录“duplicate dropped”
- 不保证顺序：所有消息必须幂等/可重复处理
  - 时钟偏差校验（POC v0，敲定）：
    - 若 `abs(now_unix_ms - created_at_unix_ms) > 10m`：丢弃，并在 debug 记录（防重放/脏消息扰乱解释）
    - 超时/重试预算只用本地 monotonic 计时；不依赖对端时钟
    - 重试允许刷新 `created_at_unix_ms`；但 request 必须复用同一个 `request_msg_id`
  - 不可接受消息：`debug` 记录后丢弃（例如 `net_id` 不匹配/已被 revoke/不在 membership）

请求-响应（需要可靠性）的统一规则：

- 用于：`join`、`state_pull`、`bootstrap_more`、（可选）打洞协调中的 request/response
- request 过期（POC，敲定）：RPC request 必须携带 `expires_at_unix_ms`；接收端若发现已过期则直接丢弃（并在 debug 记录）
- 关联（POC，敲定）：RPC response 必须携带 `in_reply_to=<request_msg_id>`（便于重试/解释/排障）
- 发起方：指数退避重试（上限 10s），直到自身 deadline/`expires_at_unix_ms`（重试应复用同一个 `request_msg_id`，便于接收端幂等）
- 响应方：对重复请求必须幂等（KISS）
  - 维护“已处理请求”缓存（key=`request_msg_id`；value=最终 response 或“已处理”状态），TTL 至少覆盖该 request 的有效期窗口
  - 重复 request 到达：重发缓存中的最终 response（不得重复计数/重复副作用；例如 `uses` 只能扣一次）
- 超时预算（POC 建议）：
   - `state_pull`：`5s`
   - `bootstrap_more`：`5s`
   - `sh_ls`：`3s`

控制面消息调度（POC，KISS）：

- 只分两类：
  - RPC（需要可靠性）：`join/state_pull/bootstrap_more/...`（走上述重试/超时/幂等）
  - best-effort：`presence/keepalive/解释性回报`（可丢、可延迟、不阻断主流程）
- best-effort 不得挤占 RPC 的发送预算（实现可用“单独队列/限流”即可）

消息族（v0，列“语义”而非字段）：

- 入网（invite 阶段）：
  - `join_request`：joiner → `invite_topic`（`invite_secret` 加密；内含 joiner `peer_id` + identity 公钥 + `reply_topic`）
  - `membership_bundle`：approver → `reply_topic`（端到端加密给 joiner；交付 `net_secret` 与长期态视图/声明集/seed）
  - `approve_member`：approver/admin 生成的“成员加入声明”（admin 签名；全网成员可解密；用于长期态传播）
    - joiner 首次握手携带该声明作为入网凭证（见 5.5）
- 状态同步（长期态收敛）：
  - `state_pull_request`：向任一在线 admin 拉取最新长期态（治理快照链头/快照 + 声明集合）
  - `state_pull_response`：返回最新视图（增量或全量皆可；POC 先允许全量；至少包含最新 governance snapshot 与 decls）
  - POC 策略（KISS）：
    - 触发：入网后首次恢复/启动时拉 1 次；以及“自动自愈”触发时拉 1 次
    - 触发（补充）：收到邻居 `hello/presence`，若 `state_head` 显示本地可能落后，则 best-effort 自动拉 1 次（限流）
    - 内容：优先全量（网络规模小，简单可靠）；后续再做增量
    - 目标：优先 approver；若仍验签失败则换一个在线 admin 再拉 1 次；再失败就停止并报错
  - wire（POC v0，敲定到最小字段）：
    - `state_pull_request.body`：`{ have_state_head?: { governance_head_b64, decls_head_b64 } }`（可选；用于解释/调试）
    - `state_pull_response.body`（全量，POC 口径）：`{ state_head, governance_snapshot, decls }`
      - `governance_snapshot`：`snapshot` 对象 `{ snapshot_body, owner_sig_b64, hash_b64? }`（`hash_b64` 仅展示；接收端必须复算）
      - `decls`：声明集合全量（含 tombstone）；`decl` 结构见 4.7
      - 可选：`peers`（派生视图，仅用于 UI/调试；权威仍是 `decls`）
    - 约束：响应解密后载荷仍受 `256KiB` 上限约束；若超过则以 `STATE_PULL_TOO_LARGE` 失败
- bootstrap 辅助：
  - `bootstrap_more_request`：joiner 在初始 seed 失败后向 approver 请求新的候选（不含端点/IP/端口）
  - `bootstrap_more_response`：返回 2 个新 peer（按 6.4 规则选取）
- 在线与发现：
  - `hello`：新建链路后的第一条“我是谁/我支持什么版本”的可验真消息（用于快速刷新 `last_seen`）
    - 最小语义（POC）：`identity + proto_version + capabilities 摘要 + state_head 摘要`
    - `capabilities`（摘要，POC，敲定）：只做能力摘要，不携带 targets/sessions 全量
      - 建议形态：`{ cmd:["ping","sh"], data_proto:["kcp","quic"], quic_cc:["bbr","brutal"], connectors:["wsl","ssh"] }`
    - `state_head`（摘要，POC，敲定）：`{ governance_head_b64, decls_head_b64 }`
      - `governance_head_b64`：治理 snapshot 链头 snapshot 的 `hash_b64`（base64url no-pad；32B）
      - `decls_head_b64`：声明集合 head（`set_head_b64`；计算见 4.7 的 `canonical_json`）
  - `presence`：邻居保活（按 6.2；无邻居时可 MQTT 定向发给 approver 兜底）
- Shell 发现（点对点，recipient-only）：
    - `sh_ls_request`：请求对端列出 targets，或列出某 target 下的 sessions（只发给收件人；不做全网广播）
    - `sh_ls_response`：返回列表（允许缓存 TTL）；用于 `miopunch sh ls`，以及 `sh_attach` 在 target 歧义/失败时附带“可选项”
    - 通道（POC，敲定）：走控制面 RPC（mesh-first + MQTT 兜底），不要求先打洞/建链
	- 打洞/建链协调（两端点对点，敏感，recipient-only）：
	  - `connect_request`：发起方请求与目标建立直连（携带家族约束 `auto/-4/-6` 等）
	  - `candidates`：交换候选（STUN 结果/candidate 集/端点证据等；只对收件人可读）
	    - body 形态（POC v0，敲定）：
	      - 两者都使用 pairwise 内层：`signed.body = { pairwise:{ epk_b64, nonce_b64, ct_b64 } }`
	      - `connect_request` inner plaintext 最小：`{ cmd, ip_family }`
	        - `cmd`：`"ping" | "sh"`
	        - `ip_family`：`"auto" | "4" | "6"`（对应 CLI 默认/-4/-6；仅约束 punching）
	      - `candidates` inner plaintext 最小：`{ ttl_ms, stun_views, selected_view, local_candidates }`
	        - `ttl_ms`：`30000`（对齐 candidates 集有效期）
	        - `stun_views`：每桶一份 `{ bucket, available, nat_difficulty, rtt_ms, ok_count, mapped_addrs }`
	        - `selected_view`：仲裁结果（唯一桶 + 映射列表）
	        - `local_candidates`：LAN/local 端点列表（不受 cn/global 影响）
	      - `endpoint` 建议结构：`{ family:4|6, ip, port }`（避免 IPv6 字符串歧义）
	    - 内置 STUN（`cn/global` 分桶）时：
	      - 双方都上报 `cn/global` 两桶的 STUN 观测（仅作用于“公网映射视图”）
	      - exchange 必须仲裁出**唯一的**“最终选定 view”，并只用该 view 的公网映射参与候选排序/打洞
	      - LAN/local candidates 的收集与交换不受 `cn/global` 影响（避免误伤同局域网直连）
    - 仲裁顺序（POC，敲定；对齐当前实现）：`availability → nat_difficulty → stun_rtt(阈值 30ms) → ok_count → default_global`
  - `connect_confirm`：确认进入打洞/建链阶段（可选；用于更快收敛时序；缺失不阻断）
  - 并发发起（KISS）：
    - A、B 同时点 `ping/sh` 时，不做复杂仲裁：视为同一次 connect
    - 本地对同一 `dst_peer_id` 只维护一个 active connect（single-flight）；重复 `connect_request` 只回送“当前最新 candidates/状态”
    - 若已存在可用 dataplane 链路：直接回“already connected”并短路（避免重复打洞）
  - 预算（POC 先定死，后续再细化）：
    - 单轮 connect attempt 固定预算：`30s`
    - 超时后视为失败；用户重试或自动重试会开启新一轮 attempt
  - 短期态缓存/有效期（POC 固定建议，KISS）：
    - STUN 自发现映射缓存：`ttl=2m`（网络变化/连接失败触发重测）
    - candidates 集有效期：`ttl=30s`（过期视为陈旧；需重新 exchange 才能继续）
    - 端点漂移（endpoint drift）：
      - 若收到“可验真”的 dataplane 包，且其 UDP 源 `ip:port` ≠ 当前 active remote endpoint：记录 `PU_ENDPOINT_DRIFT`，并将 active endpoint 切换到观测值后继续尝试
      - POC 不因 drift 立即强制 STUN 重测/全量重交换；仅在“切换后仍失败”时再进入新一轮 attempt
    - `-4/-6` 强制：仅生成/交换该地址族的 candidates；默认不展示另一族映射（`--log-level debug` 可额外记录）
  - 自动自愈（best-effort，不改变安全语义）：
    - 触发：收到对端消息但“可验真失败”（例如解密成功但验签失败/不可验真回包）
    - 动作：自动执行一次 `state_pull` 拉取最新长期态视图/声明集，然后将当前操作重试 1 次
    - 约束：同一 peer/同一时间窗口最多触发 1 次（限流）；失败则按原错误输出（可解释性提示“已尝试自愈”）
    - 不触发：解密失败（通常是 invite/net_secret 不一致/用错网络/错误配置），应直接失败并提示重新入网/清理状态
  - POC 不要求一次协商就成功：允许重试/换 peer；可解释性只观察，不改变流程

## 5. 入网（join/approve）语义（敲定）

### 5.1 两种入网模式

- `mode=approve`：需要审批
- `mode=auto`：自动批准（纯 PSK 模式）

共同规则：入网入口都基于 `invite_secret` 的临时入口；不复用 `net_secret`。

### 5.2 入网材料（invite）包含什么（概念级）

- `invite_brokers`（1–2 个；`host:port`；可为域名或 IP；POC 默认语义为 `tcp://`）
  - 约束（POC，敲定）：`invite_brokers` 必须是“approver 与 joiner 都会使用的**同一组** broker 端点”，避免因解析差异落到不同 broker 实例导致入网失败
- 人类可见摘要（POC，敲定，便于用户“知道我在连谁/连哪”）：
  - `issuer`：发码者短标识（优先 name；否则 `peer_id` 前 8）
  - `mode/expires/max_uses`：用于 UI/CLI 展示与用户心智模型
- `invite_topic`（POC v0，敲定）：
  - 必须具备 ≥128bit 有效熵，且不包含 `peer_id/name`
  - 建议形态：`invite_topic=base32(raw,no-pad, random16B)`（写入 topic 时用小写；不要带固定前缀/路径段）
- `invite_secret`
- `max_uses`（POC 默认 `1`）
- `expires`（可选；duration；可很长）
- `mode=approve|auto`
- 对外交付形式（POC，敲定；尽量多支持一点）：
  - 纯 code（文本粘贴）
  - `miopunch://join/<code>`（用于点击/二维码；`join` 需支持直接粘贴该 URL）
  - 二维码：仅由 HTTP 面板展示（CLI 不输出 ASCII QR）
  - code 编码与容错（POC，敲定）：
  - 编码：`bech32m`（输出全小写；强校验，便于人工输入/扫码）
    - HRP（前缀，POC，敲定）：`miopunch`（code 单独出现也能自描述其归属）
  - 类型与版本（POC，敲定）：
    - code 必须携带 `code_type + version`（用于演进与避免不同 code 类型混用）
    - `miopunch join` 必须校验 `code_type=join`，否则视为无效
  - broker 指示（POC，敲定）：
    - code 内必须包含 `invite_brokers`（1–2 个）：本次入网用于 MQTT 投递/回包的 broker 端点
      - 端点形态（POC）：`host:port`（可为域名或 IP；默认语义为 `tcp://`）
        - POC 不在 code 中编码 broker 的用户名/密码/证书等材料；需要鉴权/TLS 的场景后置再做
      - 取值规则（POC，敲定）：
        - 若本机 `miopunch up` 正在运行：优先取其 active brokers（最多 `2` 个）
        - 否则：由“生成 invite 时最终生效的 broker 列表”选出前 `N=2`
          - 若配置了显式 `control_plane.brokers`：以其为准
          - 否则：使用 POC 内置默认 broker 集合（由 `control_plane.broker_profile=global|cn` 决定优先级）
      - 规范化（POC，敲定）：`invite` 在写入 code 前应将 broker 端点规范化为**确定性的可连接地址**
        - 若来自 `up` 的 active brokers：直接写入当前已连接的端点（优先为 `ip:port`）
        - 若为 hostname：用 `[resolver]` 解析 A 记录并固定为**一个** `ip:port`（取 resolver 返回的第 1 个）
        - 若无法解析：保留 hostname 原样写入 code，但 `invite/approve/join` 必须输出**强警告**（该 code 的成功将依赖双方 DNS 结果一致；建议改用 `ip:port` 或显式 `control_plane.brokers`）
        - 目的：避免 joiner/approver 因 DNS/geo 分流解析到不同实例而“看不到彼此消息”
      - 目的：确保 joiner 无需预置任何 broker 配置，也能与 approver 命中同一组入口完成入网
      - 自定义 brokers（POC 建议）：admin 在生成 invite 前先配置 `control_plane.brokers`；`invite` 会把“最终生效”的前 `1–2` 个端点写入 code；joiner 不需要同步配置
    - 可选：code 可额外携带 `broker_profile`（仅用于解释/展示；不作为 join 成功的硬前置）
  - 输入：`join` 必须支持纯 code 与 `miopunch://join/<code>`；允许 code 中带空格/短横线分组（解析前去掉分隔符并统一转小写再校验）
  - 长度上限（POC，敲定）：code（含分隔符）硬上限 `1024` 字符；超过则判定无效
  - 解析失败：一律 `JOIN_CODE_INVALID`

默认口径（POC）：

- `mode=approve`
- `max_uses=1`
- `expires=15m`
- `max_uses=0` / `expires=0` 不赋予特殊含义（POC 直接视为非法参数并报错）
- 该 invite 的“发码 admin”必须在线并负责交付 `membership bundle`（POC 不做多 admin 自动接管）。

### 5.3 membership bundle（批准后下发，端到端给 joiner）

- `net_id`/`net_secret`
- 最新长期态视图（建议与 `state_pull_response.body` 同构，KISS）：
  - `state_head={governance_head_b64,decls_head_b64}`
  - `governance_snapshot`（head snapshot；含 owners/admins/recovery…）
  - `decls`（声明集合全量；含 tombstone；逐条验签；长期态权威来源）
- 可选（便于 UI/调试）：可额外携带派生后的 `peers` 列表（从 `decls+governance_snapshot` 计算得到）；但 `decls` 仍是权威来源
- peers 长期态边界（概念级）：
  - 包含：`peer_id`/`name`/公钥（Ed25519 sign + X25519 kex）/角色（owner/admin/member）/`v4_hint`/`v6_hint`
  - 不包含：STUN/candidates/公网端点列表/端口映射细节
- seed peers（2–3，含 approver/admin）
- bootstrap 推荐（初始 2 个）

约束：

- 不包含 STUN/candidates 等短期元数据；这类永远点对点加密。

### 5.4 入网握手（事件序列）

1. 管理节点生成 `invite_topic + invite_secret`，并生成包含 `invite_brokers` 的入网 code；在有效期内监听 `invite_topic`
   - `invite_topic`（POC v0）：建议 `invite_topic=base32(raw,no-pad, random16B)`（≥128bit 熵；小写；不含 `peer_id/name`；不要带固定前缀/路径段）
   - broker 策略（POC，敲定）：`miopunch approve <code>` 对 code 中的 `invite_brokers`（最多 `2` 个）并行监听/收发（不足则 1 个）
   - CLI 建议（POC）：管理员先运行 `miopunch invite` 生成 code；再运行 `miopunch approve <code>` 在有效期内监听并交付；`uses` 用完或过期即退出
2. 新节点拿到 invite（扫码/粘贴），连接 MQTT
   - broker 策略（POC，敲定）：joiner 同时连接 code 中的 `invite_brokers`（最多 `2` 个；不足则 1 个）
3. 新节点生成 identity key；再生成随机 `reply_topic` 并订阅
   - `reply_topic`（POC v0，敲定）：必须具备 ≥128bit 有效熵，且不包含 `peer_id/name`
     - 生成：`reply_topic=base32(raw,no-pad, random16B)`，写入 topic 时使用小写
     - 约束：不使用可预测前缀/固定路径段（避免被 wildcard 订阅轻易筛到）
   - 订阅策略（POC，敲定）：对每个已连接 broker 都订阅 `reply_topic`（避免“选错 broker 导致回包收不到”）
4. 新节点向 `invite_topic` 发送 `join_request`（用 `invite_secret` AEAD 加密；携带 `reply_topic` + 自己的 `peer_id` + identity 公钥）
   - identity 公钥（POC v0）：Ed25519 签名公钥 + X25519 静态 ECDH 公钥（便于后续验签/点对点加密/数据面认证绑定）
   - 发布策略（POC，敲定）：对每个已连接 broker 都发布同一条 `join_request`（靠 `msg_id` 幂等去重）
   - join_request 建议携带（仅展示用）：`joiner_name`、`platform`（信任以签名/公钥为准；name 仅用于展示）
     - `joiner_name` 默认取本机 hostname；允许在 config/命令行覆盖（用于用户友好展示与管理）
   - 重试：若无回应，joiner 在 `expires` 窗口内指数退避重发（上限 10s）；协议需幂等（重复 request 不应导致重复计数/重复入网）
5. 管理节点处理：
   - `approve`：由 admin 显式运行 `miopunch approve <code>` 承接请求（POC 默认不再二次确认），继续交付
   - `auto`：直接批准
   - `approve` 默认输出（POC，敲定）：必须打印 joiner 摘要（name/peer_id 前 8/platform）+ 本次 code 的 `issuer/mode/expires/uses` 摘要 + “如何撤销/如何重置”的下一步提示（例如：`miopunch revoke <peer>` / `miopunch reset`）
6. 管理节点向 `reply_topic` 回 `membership bundle`（端到端加密给新节点）
   - 同时（长期态传播）：管理节点生成 `approve_member` 声明（admin 签名；全网可读长期态），并最佳努力分发给既有 peers
     - 推送策略（POC）：优先推给 seeds（`seed_peers`/`bootstrap_recommendations`）与当前在线邻居；不强制逐个推全网
     - 不要求先全网收敛：joiner 在与任意 peer 的首次握手中携带该声明作为“可验真入网凭证”，对端可验签后立即落盘接纳
     - 离线/漏收节点：通过后续 `state_pull` 补齐
     - 去重：`approve_member` 作为长期态声明必须幂等；重复到达默认静默去重（`--log-level debug` 可记录）
7. 新节点收到后开始 bootstrap（见 6.4），连上 1 个邻居即可视为入网成功；随后补齐邻居
8. 新节点一旦拿到 `membership bundle`，后续控制面即切到由 `net_secret` 派生的正常 inbox / mailbox / control-plane；不再依赖 invite 入口

补充：

- Joiner 在 join 成功后不再监听/使用 `invite_topic`
- 管理节点是否继续监听该 `invite_topic`，取决于该 invite 的 `expires/max_uses`；达到条件后停止监听

入网成功口径：

- 按 6.4 的 bootstrap/retry 规则，在预算内与任意 1 个 peer 建立可用通道并完成确认（最小 payload 交换/确认事件）。

### 5.5 入网硬规则（POC 验收口径，KISS）

- `approve_member` 作为“入网凭证”：
  - joiner 在与任意 peer 的**首次**可验真握手/控制消息中携带 `approve_member` 声明
  - 只要对端已落盘接纳成功，后续连接不再需要重复携带
- 对端处理 `approve_member`：
  - 验签通过：立即把该成员写入本地长期态缓存（随后用 `state_pull` 补齐全量声明集）
  - 验签失败：直接拒绝（视为错误入网材料或攻击），并输出可解释的原因/建议重入网
- 幂等与 `uses`：
  - joiner 的 `join_request` 允许重发（指数退避 ≤10s，直到 `expires`）
  - 重发不应消耗 `uses`；`uses` 只在 admin 实际“批准并交付 `membership bundle`”时消耗
  - 实现要求（POC，最小）：签发者 admin 必须对该 invite 做最小落盘（覆盖 invite 有效期）
    - `uses` 扣减与 `handled_requests[request_msg_id] -> response` 必须可恢复（避免重启导致重复扣 `uses`/重复交付）
      - 建议（POC，KISS）：把“最终 response（已加密的 bytes）”直接落盘缓存，重复 request 到达时原样重发（不重算、不重复副作用）
    - 顺序：先落盘（并确保落盘成功）→ 再发布 `membership_bundle`
  - 对同一 joiner 的重复 `join_request`，admin 应返回同一结果或显式提示“已处理”（不得重复入网/重复计数）
- 入网成功口径：
  - joiner 与任意 1 个 seed/peer 完成“可验真握手 + 最小 payload exchange”即可视为成功
  - 全网成员视图的收敛是后台最终一致（mesh 优先 / MQTT 兜底），不阻断入网成功
- identity 重置：
  - `peer_id` 由 identity key 派生；重置 identity key = 新 `peer_id`
  - 想回到同一网络：按新节点重新走 `invite/join/approve`
- `approve` 展示要求：
  - 必须展示 joiner 关键信息：`joiner_name` + `peer_id（前 8）` + `platform`
  - 默认不要求人工确认（POC 仍允许后置加 `--interactive`/策略开关）

## 6. bootstrap/在线/恢复（敲定）

### 6.1 `last_seen` / online 窗口（POC 固定建议）

- `last_seen` 刷新条件：收到任意一条“可验真”的控制面消息（解密成功 + 签名验证通过）
- online 窗口建议：`2m`
  - 超过窗口视为“可能离线”，默认不进优选池
  - 若在线池凑不满 2 个：放宽 online 过滤（宁可给候选让 joiner 试）

### 6.2 presence（POC 规则）

- 目的：避免 `last_seen` 自然过期
- 入网后：仅发给**邻居**（不全网广播、不转发）
- 建议间隔：`30s`
- 无邻居/未入网阶段：允许通过 MQTT **定向发给 approver** 兜底（E2E + 签名）
- 安全层次：外层使用由 `net_secret` 派生的 AEAD；内含发送者签名，避免共享 `net_secret` 后可伪造来源
- 承载（POC，敲定）：presence 必须携带 `state_head={governance_head_b64,decls_head_b64}`（仅摘要 hash），用于传播“治理/声明集合是否变化”并触发对端 best-effort `state_pull`
- 重放/去重：presence 也带唯一消息 ID，并按去重 TTL 丢弃重复消息
- 链路保活（POC 必需）：
  - 已建立直连 dataplane 的邻居链路，在无业务时也必须周期性发送小包保活，用于维持 NAT/防火墙的 conntrack（CT）表
  - 建议：`interval=15s`（具体值后续可调优）；保活可由 transport 自身实现，或由 control-plane 在 dataplane 上发送轻量 keepalive
  - 断链判定与自愈（POC）：
    - 若连续 `ttl=60s` 未观测到来自该邻居的任意 dataplane 包，则认为该直连链路失活
    - 动作：后台触发一次新的 connect attempt（预算 `30s`）；若仍失败则按“邻居维护”规则更换邻居（见 6.5）

### 6.3 reachability hints（仅用于排序）

- 每个 peer 提供两个 hint：`v4_hint`、`v6_hint`；不包含端点信息
- 仅用于：approver 选 bootstrap 候选、rejoin 选邻居排序
- 更新触发：
  - 网络变化
  - 连接失败
  - TTL 过期（建议 `10m`；过期视为 `unknown`）

分级：

- `v4_hint`：`direct > easy > hard1(端口增长可预测) > hard2(端口增长不可预测) > unknown > none`
- `v6_hint`：`direct > easy > hard1(入站受限/需先出站建表) > unknown > none`（POC 不引入 hard2）

地址族：

- 默认 `auto`：同一 peer 尝试顺序为“先 v6 后 v4”
- `-4/-6`：仅约束 p2p punching 路径，不改变 rendezvous 的联网行为

### 6.4 bootstrap 推荐 + 重试（敲定）

- 初始：membership 附带 `bootstrap_recommendations`（2 个 peer）
- Joiner 依次尝试这 2 个
- 两个都失败：
  - Joiner 通过 MQTT 发 `bootstrap_more_request`（携带“已尝试 peer 列表 + 极粗失败摘要”；不含 IP/端口）
  - 优先发给本次入网的 approver；若超时可按 admin 列表顺序兜底尝试其它在线 admin
  - 对端回 `bootstrap_more_response`（2 个新 peer）
- 最多重复 2 次（总计最多尝试 6 个 peer）；仍失败则 join 失败并提示“环境过硬/节点不在线/需要一个网内可达的中继节点（由用户自持；POC 不提供中心化公共 relay）”。

Approver 选 peer 的最小规则：

- 先按桶逐级放宽（direct/easy → hard1 → hard2/unknown）
- 尽量从 online 池取；不够则放宽
- 桶内随机/轮换
- 必须去重（不得重复 joiner 已尝试的 peer）

### 6.5 入网后的邻居维护（避免单点）

- bootstrap 只负责“第一跳”
- 入网后：按 `k = max(2, ceil(ln(n)))` 维持邻居数（n 为已知 peer 数）
- 选择策略：桶内随机，逐级放宽，避免所有节点都只连“最好连的那台”
- 触发换邻居（POC，KISS）：
  - 邻居直连链路失活（见 6.2）→ 尝试重连；若连续 2 次 attempt 仍失败 → 换下一个邻居

### 6.6 Rejoin/Recover（重启/换网/长期离线统一流程）

本地持久化（最小）：

- identity key（首次生成即落盘复用；若要重置，等同新节点重新入网）
- `net_id`/`net_secret`
- 落盘保护（POC 最低口径，KISS）：
  - 威胁模型：本机被攻破/磁盘被读取 = 该 peer 身份与网络 secret 被攻破（需要 `reset` + 重新入网；必要时 admin `revoke`）
  - 不做：自研“加密落盘”/自定义口令/KMS 集成（后置）
  - 必做：`state_dir` 权限收敛 + 日志/报告永不输出明文 secret
    - Linux（system service）：默认 `root:miopunch-operators`；目录 `0750`；文件 `0640`
    - Linux（user mode）：目录 `0700`；文件 `0600`
    - Windows：`ProgramData` 下目录/文件 ACL 仅 `{LocalSystem}+{operator user}`
  - 可选（后置）：Windows DPAPI / macOS Keychain / Linux Secret Service
- broker 入口（POC，敲定）：
  - `brokers_effective`：本机实际用于 control-plane 的 broker 端点列表（1–2 个；可为 IP 或域名；建议为 `ip:port`）
    - joiner：来自 join code 的 `invite_brokers`（并原样持久化）
    - 初始 admin（第一台机器）：来自本机配置的 `control_plane.brokers`（若配置）或内置默认 broker（由 `broker_profile` 决定优先级）
  - `broker_profile` 仅用于“无显式 brokers 且尚未持久化 brokers_effective”时选择内置默认；一旦形成 `brokers_effective`，后续优先使用它
- approver/admin contact
- `contact_set`（seed + 最近成功邻居滚动更新）

恢复流程（高层）：

1. 连接 MQTT，订阅自身 mailbox
2. presence 定向发：approver + contact_set
3. `state_pull`：向 approver 或任一在线 admin 拉取最新长期态视图/声明集合
4. 计算目标邻居数 k，按桶随机重建邻居并重打洞
5. 全失败：复用 `bootstrap_more` 机制

合并口径（POC）：

- membership/peers 声明集合：按 **set-union** 合并收敛（`revoke` 作为永久 tombstone）
- admin/owner/config：不走 union，走 owner-signed snapshot 链（见 7.1）

`contact_set` 初始来源（敲定）：

- membership 交付时给 2–3 个 seed（含 approver/admin），本地持久化
- 运行中用“最近成功连过的邻居”滚动更新

## 7. 治理（owner/admin/recovery）（敲定）

POC 范围（写操作，明确可用入口）：

- 入网：`approve_member` 由 `miopunch approve` 产生（见第 5 章）
- 治理核按钮：只实现 `revoke member`（`miopunch revoke <peer>`）
- 其余 owner-only 变更（admin/owner 集合、recovery codes、`net_secret_rotate` 等）：仅写语义（后置）

### 7.1 状态收敛：owner 签名 snapshot 链

- admin/owner/config 这类“带删除语义”的状态，使用 **owner 签名 snapshot 链**：
  - snapshot 由两部分构成：
    - `snapshot_body`：可哈希、可复制的“治理快照本体”（不含签名字段）
    - `owner_sig_b64`：owner 对该 `snapshot_body` 的签名
  - `snapshot_body`（POC 语义，字段可演进）：
    - `prev_hash_b64`（base64url no-pad；32B sha256；genesis 为空字符串）
    - `height`（int；genesis=0；每次变更 +1）
    - `owners`（[]Ed25519 pub；base64url no-pad）
    - `admins`（[]Ed25519 pub；base64url no-pad）
    - `recovery_allowlist`（[]Ed25519 pub；base64url no-pad；一次性使用后移除）
    - `config_digest?`（可选；用于未来把“网络配置”纳入治理）
  - hash 与签名（POC，敲定）：
    - `hash_b64 = base64url(no-pad, sha256(canonical_json(snapshot_body)))`
    - `owner_sig_b64 = base64url(no-pad, Ed25519.Sign(owner_priv, sha256(canonical_json(snapshot_body))))`
  - 验证规则（必须）：
    - 以“当前已知 snapshot”中的 `owners` 集合作为验签信任根（允许 snapshot 变更 owners/admins）
    - `prev_hash_b64` 必须指向本地链头（避免产生分叉；见 7.2 的流程）
  - 所有节点以链头为准（通过 `governance_head_b64` 对齐）
  - 分叉属于异常/误操作；实现应尽量避免（优先拒绝 stale proposal；见 7.2）

### 7.2 角色与权限边界

- Owner（super signing key）：
  - 可离线保存；离线签名即可，由任意在线节点代发广播
  - 负责：admin/owner 集合变更、recovery codes 下发、（未来）`net_secret_rotate`
- Admin/Approver：
  - 日常动作：approve join、响应 `bootstrap_more`、响应 `state_pull`、presence/last_seen 等

Owner-only 生效（必须 owner 签名的动作）：

- 变更 admin 集合（grant/demote）
- 变更 owner 集合（rotate/remove/add；但不能移除最后一个 owner）
- 签发/更新 recovery codes
- （后置）`net_secret_rotate`

UX 口径（不改变安全语义）：

- 允许在任意 admin 节点“发起”上述动作，但**只有 owner 签名后才生效**（类似 sudo/2FA）。
- owner 临时离线不影响网络运行；仅 owner-only 动作暂不可执行。

Admin 可执行（不需要 owner 的日常动作）：

- approve join
- 响应 `bootstrap_more` / `state_pull`
- 对普通 member 执行 `revoke member`（核按钮，仍需二次确认）
- 不允许对 admin/owner 执行 `revoke member`：应改走 owner-signed snapshot 链 demote/remove（POC 不实现）

危险动作 UX 护栏（敲定）：

- `revoke member` 必须二次确认（例如 `--dangerous` + 确认短语），并明确提示“该 key 永久不可恢复（需换新 key）”

#### owner-signed snapshot 的“发起/签名/代发”流程（后置实现，但流程敲定）

目标：

- owner key 可离线保存；owner 不必在线即可完成 owner-only 变更。
- 任意在线 admin 节点都能“发起”；但只有 owner 签名后才生效。
- 不引入中心化控制器；传播最终一致（mesh 优先 / MQTT 兜底）。

建议三步（KISS，偏文本/文件交付；UI/二维码后置）：

1) 发起（admin 在线节点）：

- admin 生成“签名请求（proposal）”，用于把“要签什么”交付给 owner（proposal 本身不需要签名；最终只签 `snapshot_body`）。
- proposal 最小字段（JSON；POC 口径，字段可演进）：
  - `proposal_version`（int，必需；POC 固定 `0`）
  - `proposal_id`（string，必需；`16B` 随机 → base32(raw,no-pad)；`26` 字符）
  - `created_at_unix_ms`（int64，必需）
  - `net_id`（string，建议；用于“避免 owner 跨网误签”）
  - `initiator_peer_id`（string，必需；发起该 proposal 的 admin peer）
  - `initiator_name`（string，可选；仅用于展示）
  - `snapshot_body`（object，必需；见 7.1；**必须**包含 `prev_hash_b64/height/owners/admins/...`）
  - `prev_snapshot_body`（object，可选但强烈建议；用于签名端做 diff 展示）
    - 若提供：签名端必须校验 `sha256(canonical_json(prev_snapshot_body)) == snapshot_body.prev_hash_b64`
  - `summary`（object，建议；用于展示/解释；**非权威**，签名端必须复算/校验）
  - `note`（string，可选；纯备注）
- 展示摘要（proposal 生成端必须提供；签名端必须复算/校验，禁止“只展示一段不可验证文案”）：
  - 最少要能让 owner 读懂：这次到底改了什么（owners/admins/recovery/config）
  - 建议结构（非权威）：`summary = { owners_added, owners_removed, admins_added, admins_removed, recovery_added, recovery_removed, config_digest_changed? }`
- 交付形式（建议）：可复制的文本 blob 或文件（例如 `proposal.json`）。

2) 签名（owner 离线设备/离线 key）：

- owner 侧签名器必须只对 `snapshot_body` 签名（见 7.1），并在签名前做“可解释、可验证”的展示：
  - 重新计算并展示：
    - `prev_hash_b64`（短显+可复制全量）
    - `height`
    - `hash_b64`（本次 `snapshot_body` 的 hash；短显+可复制全量；确认短语建议绑定它）
  - 若 `prev_snapshot_body` 存在且校验通过：展示“已验证 prev_snapshot_body 与 prev_hash_b64 一致”；并展示 diff（added/removed）
  - 若 `prev_snapshot_body` 缺失：必须明确提示“无法自动生成 diff，只能展示新快照的全量集合”（降低置信度）
  - 必须展示的摘要（最低口径）：
    - owners/admins/recovery_allowlist 的新增/移除列表：
      - 优先展示 `peer_id`（由 Ed25519 pub 派生）+（若可得）`name`
      - 否则展示 pubkey 指纹（短显+可复制全量）
    - owners/admins 的最终集合与数量
    - `config_digest` 变化（若有；展示 old/new）
  - 护栏（必须）：
    - 二次确认（确认短语必须包含 `hash_b64` 前缀，例如 `SIGN <hash8>`；防盲签）
    - 若 proposal 的 `net_id` 与签名器本地 net_id 可得且不一致：必须拒绝（或要求 `--force` 明确绕过）
- 输出“已签名的 snapshot”（例如 `snapshot.json`），交回给任意在线 admin 代发。

3) 代发/应用（任意在线 admin 节点）：

- admin 先本地验证再应用：
  - `owner_sig_b64` 可用“当前 owners 集合”验签通过
  - `prev_hash_b64 == 本地 governance_head_b64`（若不匹配：拒绝并提示重新发起；避免 fork）
- 应用成功后：
  - 本地持久化新链头
  - 通过正常机制传播（最终一致）：
    - `hello`（或等价可验真 keepalive）携带最新 `state_head`（含 `governance_head_b64`）
    - 其他节点见到 head 变化后 best-effort 触发一次 `state_pull` 拉取最新治理快照与声明集合（见 4.7）
    - 离线/漏收节点通过后续 `state_pull` 补齐

备注（为何不使用“全网广播 topic”）：

- 不引入全网大 topic；避免可枚举入口与不必要的元数据暴露。
- snapshot 的传播以“点对点投递 + 自动 pull 收敛”为主，简单且足够可靠（网络规模小）。

##### `proposal.json` / `snapshot.json` 最小输出格式（POC v0，敲定）

通用约定：

- 编码：UTF-8 JSON（建议 pretty-print；字段名固定；允许新增字段）
- 所有 `*_b64`：`base64url(no-pad)`
- unknown 字段：必须安全忽略（向前兼容）
- **安全提醒**：proposal/snapshot 文件不得包含任何私钥/`net_secret`/broker 密码等敏感材料；只包含公钥与摘要。

`proposal.json`（发起端 → 签名端）：

- 语义：签名端只信任 `snapshot_body`（其余字段仅用于展示/辅助校验）。
- 必需字段：`proposal_version, proposal_id, created_at_unix_ms, initiator_peer_id, snapshot_body`
- 建议字段：`net_id, initiator_name, prev_snapshot_body, summary, summary_text, note`
- `summary_text`（可选）：纯展示文本（多行允许）；**非权威**，签名端必须复算/校验摘要后再展示。

最小示例（v0）：

```json
{
  "proposal_version": 0,
  "proposal_id": "<proposal_id_26ch>",
  "created_at_unix_ms": 0,
  "initiator_peer_id": "<peer_id_26ch>",
  "snapshot_body": {
    "prev_hash_b64": "",
    "height": 0,
    "owners": [],
    "admins": [],
    "recovery_allowlist": []
  }
}
```

`snapshot.json`（签名端 → 代发端）：

- 语义：这是“可被全网验真并应用”的治理快照载体。
- 必需字段：`snapshot_body, owner_sig_b64`
- 可选字段：`hash_b64`（展示用；接收端必须复算并校验一致）

最小示例（v0）：

```json
{
  "snapshot_body": {
    "prev_hash_b64": "",
    "height": 0,
    "owners": [],
    "admins": [],
    "recovery_allowlist": []
  },
  "owner_sig_b64": "<ed25519_sig_b64_86ch>"
}
```

##### CLI / LocalAPI 映射（后置实现，但命名与语义先敲定）

目标：把 owner-only 变更做成“可离线签名 + 任意在线 admin 代发”的统一流程。

- `miopunch gov propose ...`（在线 admin 发起）：
  - 作用：基于当前 `governance_head_b64` 生成 `proposal.json`（含 `snapshot_body` 与可解释摘要）。
  - 前置：需要本机 `miopunch up` 正在运行（读取当前治理链头/快照）。
  - LocalAPI（建议）：创建 task `kind=gov_propose`，`result` 产出 `proposal.json`（或 `proposal` 对象）。
  - 变更类型（建议先从“全量 set”开始，KISS；后置再加 add/remove sugar）：
    - `admins_set`：设置 admin 集合
    - `owners_set`：设置 owner 集合（不得为空；不得移除最后一个 owner）
    - `recovery_allowlist_set`：设置 recovery allowlist（一次性 keys）
    - `config_digest_set`：（后置）设置 `config_digest`
  - 命令形态（A，敲定；最贴近直觉、也最贴近 LocalAPI args）：
    - `miopunch gov propose --change <change_kind> [--peer <peer>...] [--recovery-pub <ed25519_pub_b64>...] [--config-digest-b64 <b64>] [--out <proposal.json>] [--note <text>]`
      - `--peer`（可重复；用于 `admins_set/owners_set`）：使用 `<peer>` 选择器（name 或 peer_id 前缀）；由发起端从本地视图解析为 pubkeys 写入 `snapshot_body`
      - `--recovery-pub`（可重复；用于 `recovery_allowlist_set`，后置）：明确指定一次性 recovery key 的 Ed25519 公钥（base64url）
      - `--config-digest-b64`（用于 `config_digest_set`，后置）：明确指定新 `config_digest`（base64url）
      - set 语义：你传的就是“最终集合”（不是 add/remove）
      - 输出：默认 stdout 输出 `proposal.json`（纯 JSON）；`--out` 可写文件（便于交付给离线 owner）
  - 默认生成规则（敲定；缺失也不致命，但会降低“可解释/可验证”程度）：
    - 必须写入 `net_id`（用于避免 owner 跨网误签）
    - 应尽量写入 `prev_snapshot_body`（本地当前链头的 `snapshot_body`），用于签名端生成可验证 diff
  - task args（建议最小；字段可演进；与上述 CLI 对齐）：
    - `{ change_kind, peers?, recovery_allowlist_pubs_b64?, config_digest_b64?, note?, out_path? }`
- `miopunch gov sign <proposal.json> --owner-key <path> [--out <snapshot.json>]`（离线 owner 签名）：
  - 作用：读取 `proposal.json`，按 7.1 对 `snapshot_body` 签名，输出 `snapshot.json`。
  - 前置：不需要 `miopunch up`；这是纯本地签名操作（可在离线设备执行）。
  - `--owner-key <path>`（POC 暂定文件格式）：
    - 文件内容：Ed25519 私钥种子（`32B`）的 `base64url(no-pad)`（末尾允许换行）
    - POC：不支持口令加密私钥文件；离线保存需自行做好 OS 权限/物理隔离（后置再扩展）
  - 输出：默认输出 `snapshot.json`（stdout 或 `--out` 文件）；不得写入任何私钥材料到输出。
  - 交互与确认（A，敲定；人用安全、脚本可用）：
    - TTY：默认交互展示（摘要 + diff + `hash_b64`），并要求输入确认短语 `SIGN <hash8>` 才允许签名（缺失则 `GOV_SIGN_NEEDS_CONFIRMATION`）
    - 非 TTY：必须显式提供 `--dangerous "SIGN <hash8>"`（同样绑定 hash 前缀），否则拒绝
    - `net_id` 不一致默认拒绝（`GOV_SIGN_NET_ID_MISMATCH`）；可 `--force` 明确绕过（不推荐）
    - 若 proposal 缺少 `net_id` 或 `prev_snapshot_body`：必须明确提示“无法做跨网校验/无法做可验证 diff（降级展示）”，但仍允许继续签名（KISS）
- `miopunch gov apply <snapshot.json>`（在线 admin 代发/应用）：
  - 作用：校验 `snapshot.json`（owner 验签 + `prev_hash_b64` 对齐），应用为新链头，并 best-effort 传播。
  - 前置：需要本机 `miopunch up` 正在运行。
  - LocalAPI（建议）：创建 task `kind=gov_apply`，输入为 `snapshot.json`（或 `snapshot` 对象）。
  - 传播（A，敲定；不引入全网广播 topic / 不逐个 inbox 推送）：
    - 只更新本地链头；不等待全网确认
    - 通过 `hello/presence.state_head` 的变化触发其它节点 best-effort `state_pull` 补齐（最终一致）
    - apply 成功后可 best-effort 立刻对“当前邻居”发一次 `presence`（加速对端察觉 head 变化）
    - POC 不做“逐个 seed/逐个 peer inbox 推送 snapshot/head”（避免放大与复杂排障）
  - task result（建议最小）：
    - `{ governance_head_b64, height, owners_count, admins_count }`

### 7.3 revoke 语义（核按钮，敲定）

- `revoke member`：永久拉黑该身份公钥（不可恢复）
- 重新回来：必须换新 identity key 再 join
- 不用 revoke 来降权：降权走 admin 集合变更（owner-signed snapshot）
- 入口（POC，可用）：
  - CLI：`miopunch revoke <peer>`（必须二次确认）
  - LocalAPI：创建 `revoke_member` task（并生成 `revoke_member` decl 写入声明集合）
  - HTTP 面板：不开放
- 即时效果（必须）：
  - 一旦本地写入 `revoke_member` tombstone：立即拒绝该 peer 的后续控制面消息与 dataplane 握手（视为不可信）
  - 对既有连接：允许实现选择“尽快断开/等待自然断开”；但安全语义上不得再继续信任该 peer
- 传播（最终一致，KISS）：
  - 产生者应最佳努力推送给在线邻居/seed；离线/漏收节点通过后续 `state_pull` 补齐
- 约束（POC，KISS，安全优先）：
  - 只允许撤销普通 member
  - 若目标为 admin/owner，或无法确认其不是 admin/owner：拒绝并提示需要 owner-signed 治理快照（POC 不实现）

### 7.4 owner 集合与灾备（最终口径）

- owner 集合允许轮换/移除（应对泄露/被盗）
- 硬规则：**不能移除最后一个 owner key**
- 推荐：至少 2 把 owner key（`owner_primary` + `owner_backup`），分地离线保存

#### Recovery Codes（一次性 owner 恢复钥匙）

- backup code = 一次性 recovery owner key（高熵随机，等价 root key 的私钥种子）
- 网络中记录其公钥（在 snapshot 里）
- 使用某个 recovery code 进行 owner 更新签名后：
  - 该 recovery 公钥从 allowlist 移除（一次性消耗）
- recovery codes 快用完时：任意现存 owner 可签发新一批
- 若 **所有 owner key + 所有 recovery codes 都丢失**：
  - owner-only 能力永久丢失（admin 集合变更/密钥轮换等不可做）
  - 只能新建网络

## 8. `net_secret` 轮换（明确后置）

- POC：不支持轮换
- 若 `net_secret` 泄露：攻击者可持续订阅/投递到该网络命名空间；没有中心控制器可踢人
  - 干净恢复方式：rekey/new network（新 `net_id + net_secret`，所有设备重新入网）
- P4：加入 `net_secret_rotate`
  - owner-signed snapshot 宣告轮换
  - 新 `net_secret` 逐个端到端加密交付给成员
  - 成员切换到新派生的 topic/key

## 9. 可解释性（POC 口径，敲定）

目标（对用户极度友好）：

- 用户在 10 秒内能回答：我在连谁、现在到哪一步、成功走哪条路径、失败卡在哪一步、最可能原因是什么、下一步怎么做。
- 同一套“阶段/事件”同时驱动 CLI 与桌面 UI（CLI=文本渲染器，UI=时间线/卡片渲染器），避免口径割裂。
- 面向“具备一些背景知识”的用户：用接近白话的方式讲清楚“事实→推断→建议”，并在必要处给出关键名词解释。
- 术语口径：`docs/notes/2026-04-16-alpha-glossary.md`（Alpha/POC 术语词典，非正式规范）。

### 9.1 展示边界（信息一览无余，但不泛滥）

- 本机（默认展示）：
  - 本机公网映射：`v4/v6 ip:port`（若存在）
  - `v4_hint/v6_hint`
- 点对点详情（仅当“我正在连这个 peer / 已直连”时默认展示）：
  - 当前实际使用的端点对：`local ip:port → remote ip:port`（两边都展示）
  - 当前实际传输栈：`kcp` 或 `quic`（若为 `quic`，显示 `cc=bbr|brutal`）
  - 当前路径摘要：`family=v4|v6` + `path=direct|punch` + 建链耗时
- 全局总览（`miopunch ls` / UI 总览）：
  - 默认不展开“他人的端点全集/候选全集”；只给状态摘要（online/last_seen、角色、hint、是否有链路/链路类型）。

### 9.2 默认输出契约（正常短、失败全）

- 正常（默认）：
  - 一句摘要（成功/进行中 + 当前阶段）
  - 关键事实 3–6 条（本机映射、对端 last_seen、当前 transport/cc、session/单写者状态等）
  - 进度：固定阶段机的当前位置（见 9.3）
- 失败（自动展开）：
  - 一句摘要：`卡在 <阶段>：<白话原因>`
  - 诊断路径：沿该阶段诊断树命中的“问题→分支→叶子结论”
  - 推断链：`观测事实 → 推断(置信度) → 建议动作(≤3)`
    - 置信度：`确定 / 大概率 / 可能`（只给 1 个主结论 + 1 个备选）
- 细节展开：
  - CLI/UI 均提供“展开详情”的方式（具体开关/交互后定）；debug 才输出更底层的原始细节。
- 非侵入性硬原则（必须贯穿所有可解释性实现）：
  - 任何“诊断回报/回执/统计/解释事件”均为 sidecar/best-effort：可缺失、可延迟、可丢失。
  - sidecar 的缺失/延迟/丢失**不得改变**打洞/建链/握手/能力层的流程与结果；只能降低解释置信度与可展示信息量。
- 因此：可解释性信息永远不作为协议硬前置；无法获取时一律视为 `unknown` 分支继续推进。

### 9.2.1 稳定字段与演进规则（POC Freeze）

- `stage`/`reason_code`/`term_id` 为稳定标识：POC 内不重命名，只允许新增。
- `--format json` 顶层字段（POC 下限，至少保证存在）：`stage, reason_code, exit_code, message, facts, suggestions, request_id`（允许额外字段扩展）。
- 需要“改名/合并”时：保留旧 `reason_code` 作为 alias/deprecated（Alpha/POC 生命周期内保持兼容），并在报告/建议中提示新口径。

### 9.3 固定阶段机（CLI/UI 一致）

1. `ControlPlaneReady`：信令可达（mesh 优先 / MQTT 兜底）+ E2E 就绪
2. `SelfDiscovery`：本机公网映射与 hint 自发现
3. `PeerContact`：对端在线/可验真可达（last_seen/hello）
4. `CandidateExchange`：候选交换完成（仅默认显示数量与最终选中家族）
5. `PunchAttempt`：直连/打洞尝试（direct/punch）
6. `DataplaneHandshake`：`kcp|quic(+cc)` 建链
7. `CapabilityHandshake`：shell 能力握手 + 单写者锁
8. `SessionAttach`：`tmux` attach/create（target/session）

### 9.4 每阶段诊断树（v0，POC 先覆盖高频）

> 规则：进度推进到某阶段，只启用该阶段诊断树；失败时输出命中的诊断路径与叶子结论。
>
> 约束（非侵入性）：诊断树只读取可获得观测；观测缺失则走 `unknown` 分支并降低置信度，但绝不阻断流程推进。

`ControlPlaneReady`

> 目标：把“连不上 / 能连但解不开 / 能解但不可信”区分清楚。

观测前提（POC 级别）：

- broker 地址解析是否成功（DNS）
- 到 broker 的连接是否成功
- subscribe/publish 是否成功（至少对自己的 inbox）
- 收到消息后：是否能解密、是否能验签

诊断树（v0）：

- DNS 失败 → `CP_DNS_FAIL`
- 连接失败 → `CP_CONNECT_FAIL`
- subscribe 失败 → `CP_SUBSCRIBE_FAIL`
- publish 失败 → `CP_PUBLISH_FAIL`
- 能收包但解密失败 → `CP_DECRYPT_FAIL`（常见：invite/net_secret 不一致）
- 解密成功但验签失败 → `CP_SIGNATURE_INVALID`（常见：身份不可信/已被 revoke/视图过旧）
- publish/subscribe 看似成功但始终收不到“可验真控制消息” → `CP_NO_VERIFIED_RECV`

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `CP_DNS_FAIL`：无法解析 broker/STUN 域名
  - 证据：DNS 解析失败（明确错误）
  - 建议：切换内置/显式 DNS；临时改用 IP；换网络
- `CP_CONNECT_FAIL`：无法连接 broker（网络/阻断）
  - 证据：TCP 连接失败/超时
  - 建议：换 broker（cn/global）；换网络；检查公司网/代理/防火墙
- `CP_SUBSCRIBE_FAIL`：无法订阅 inbox（权限/连接不稳）
  - 证据：subscribe 明确失败
  - 建议：重连 broker；换 broker；检查 broker 配置/限制
- `CP_PUBLISH_FAIL`：无法发布控制消息（权限/连接不稳）
  - 证据：publish 明确失败
  - 建议：重连 broker；换 broker；检查 broker 配置/限制
- `CP_DECRYPT_FAIL`：控制消息解密失败（密钥不匹配）
  - 证据：能收到密文消息，但解密失败
  - 建议：确认当前处于 join/in-network 的正确入口；重新入网/检查配置来源；必要时清理旧状态后重试
- `CP_SIGNATURE_INVALID`：控制消息验签失败（身份不可信/视图过旧）
  - 证据：解密成功但验签失败
  - 建议：先 `state_pull` 更新视图；确认对端未被 revoke/未更换 key；必要时重新入网
- `CP_NO_VERIFIED_RECV`：收不到任何可验真控制消息（对端离线/投递路径断）
  - 证据：发布/订阅无明确错误，但超时无可验真回包
  - 建议：回看 `PeerContact`（对端是否在线）；强制走 MQTT 兜底；换 broker/网络

置信度规则（最小）：

- DNS/连接/订阅/发布的明确错误：`确定`
- 解密/验签失败属于强证据：`确定`
- “无可验真回包”需结合 `PeerContact`：默认不高于 `大概率`

`SelfDiscovery`

> 目标：把“STUN 不可达 / 只有 v4/v6 / 约束冲突”讲清楚。

观测前提（POC 级别）：

- STUN 解析/可达性（失败原因需可见）
- 本机可用的公网映射：`v4/v6 ip:port`（若存在）+ STUN RTT（若有）
  - RTT 口径（POC，敲定）：直接使用 STUN 请求-响应的往返时间（transaction RTT）；不额外对映射 `ip:port` 做 ping
- 用户约束：`auto/-4/-6`
- 内置 STUN 分桶（POC，敲定；保持现有流程）：
  - 分桶：`stun_servers_global` 与 `stun_servers_cn`
  - 采样：每桶受限并发（并发=3）+ 早停；每个 hostname 最多展开 2 个 A 记录
  - 产物：两份 view 观测（`available/nat_difficulty/rtt_ms/ok_count/mapped_addrs`）；后续在 `candidates` exchange 中仲裁为唯一最终 view（见 4.7）
  - 输出：默认仅展示最终汇总的“本机公网映射 v4/v6 ip:port + RTT”；失败或 `--log-level debug` 才展开每个 view 的摘要与采样错误

诊断树（v0）：

- STUN 解析失败 → `SD_STUN_DNS_FAIL`
- STUN 超时/不可达 → `SD_STUN_TIMEOUT`
- 约束的地址族不可用（例如 `-6` 但无 v6 映射） → `SD_FAMILY_UNAVAILABLE`
- 能拿到映射但 hint 只能给 `unknown` → `SD_HINT_UNKNOWN`（不阻断流程，只影响排序/解释）

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `SD_STUN_DNS_FAIL`：STUN 域名解析失败
  - 证据：DNS 解析失败（明确错误）
  - 建议：切换内置/显式 DNS；改用 IP（临时）；换网络
- `SD_STUN_TIMEOUT`：STUN 超时/不可达（UDP 受限或 STUN 不可用）
  - 证据：`global/cn` 两桶 STUN 尝试均未得到可用映射（或明确网络错误）
  - 建议：调整/精简 `stun_servers_global/stun_servers_cn`；换网络（热点/宽带）；检查 UDP 出站/防火墙
- `SD_FAMILY_UNAVAILABLE`：地址族不可用（与 `-4/-6` 约束冲突）
  - 证据：仅有 v4 或仅有 v6 映射；且用户强制了相反地址族
  - 建议：改 `auto` 或改用可用地址族；必要时换网络获取 v6/v4
- `SD_HINT_UNKNOWN`：无法可靠判断 NAT/hint（不阻断）
  - 证据：映射存在但特征不足/数据过旧
  - 建议：继续流程；在失败时再回收证据；网络变化时重算

置信度规则（最小）：

- DNS/超时属于强证据：`确定`
- hint 属于启发式：不得高于 `可能`

`PeerContact`

> 目标：区分“对端离线 / 投递路径断 / 对端身份不可信”。

观测前提（POC 级别）：

- 对端 `last_seen` 与 online 窗口（POC：2m）
- 是否收到对端任意“可验真控制消息”（解密成功 + 验签通过）
- 对端是否已被 revoke（来自本地视图/`state_pull`）

诊断树（v0）：

- 对端 `last_seen` 超窗 → `PC_OFFLINE_LIKELY`
- mailbox 投递无回音（超时无可验真回包） → `PC_NO_VERIFIED_REPLY`
- 收到消息但解密/验签失败 → `PC_UNVERIFIABLE_REPLY`
- 对端被 revoke → `PC_REVOKED`（优先级最高）

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `PC_OFFLINE_LIKELY`：对端可能离线/未运行 agent
  - 证据：`last_seen` 超出 online 窗口
  - 建议：让对端上线并保持一段时间；确认对端网络可达；再试
- `PC_NO_VERIFIED_REPLY`：投递无回音（对端控制面未恢复或链路断）
  - 证据：超时无可验真回包；（可选）对端进度停在更早阶段
  - 建议：让对端先恢复到 `ControlPlaneReady`；换 broker/网络；强制走 MQTT 兜底
- `PC_UNVERIFIABLE_REPLY`：对端回包不可验真（密钥不匹配/视图过旧/疑似攻击）
  - 证据：能收包但解密失败，或验签失败
  - 建议：先 `state_pull` 更新视图；确认 invite/net_secret 一致；必要时重新入网
- `PC_REVOKED`：对端身份已被撤销（不可恢复）
  - 证据：本地视图/声明集合明确该 key 被 revoke
  - 建议：对端必须换新 identity key 重新入网

置信度规则（最小）：

- `PC_REVOKED`：`确定`
- 解密/验签失败：`确定`
- `last_seen` 超窗：`大概率`（仍允许用户重试）

`CandidateExchange`

> 目标：把“我有没有候选 / 对端有没有回 / 有没有共同地址族”分开讲清楚。

观测前提（POC 级别）：

- 本端候选数量（按 v4/v6 计数即可）与用户约束 `auto/-4/-6`
- 是否收到对端候选（以及是否收到“对端已收到我的候选”的回执，若实现）
- 候选交换阶段是否走 MQTT 兜底

诊断树（v0）：

- 本端在所需地址族下无候选 → `CE_NO_LOCAL_CANDIDATE`（回到 `SelfDiscovery`）
- 等待对端候选超时 → `CE_PEER_NO_REPLY`（回到 `PeerContact/ControlPlaneReady`）
- 对端回了但无共同地址族 → `CE_NO_COMMON_FAMILY`
- 对端显式拒绝（例如参数/版本不兼容） → `CE_PEER_REJECTED`

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `CE_NO_LOCAL_CANDIDATE`：本端无可用候选（无法开始打洞）
  - 证据：在 `auto/-4/-6` 约束下，本端候选数量为 0
  - 建议：回到 `SelfDiscovery` 排查 STUN；改 `auto` 或换族；换网络
- `CE_PEER_NO_REPLY`：对端未回候选（对端未到该阶段或控制面未通）
  - 证据：超时未收到对端候选；（可选）对端 last_seen 超窗/对端进度更早
  - 建议：先按 `PeerContact` 诊断对端在线；强制走 MQTT 兜底；换 broker/网络；让对端重试
- `CE_NO_COMMON_FAMILY`：双方无共同地址族（或约束冲突）
  - 证据：一侧只有 v4、另一侧只有 v6；或用户强制 `-4/-6` 导致无交集
  - 建议：统一两端使用 `auto` 或统一强制同一地址族；必要时换网络获取 v6/v4
- `CE_PEER_REJECTED`：对端显式拒绝（不满足对端约束/不兼容）
  - 证据：对端回了明确拒绝原因（白盒可直接显示）
  - 建议：统一两端版本/配置（`data-proto/quic-cc` 等）；必要时重新入网/更新状态

置信度规则（最小）：

- “对端显式拒绝/本端无候选”属于强证据：`确定`
- “对端未回”必须结合 `PeerContact`：默认不高于 `大概率`

`PunchAttempt`

> 目标：按“阶段诊断树”一步步给出结论；不堆砌玄学猜测。

观测前提（POC 级别）：

- 本次尝试的路径摘要：`family=v4|v6`、`path=direct|punch`
- 本次选中的端点对：`local ip:port → remote ip:port`（仅在点对点详情默认展示；总览可隐藏）
- 本机侧：在本次窗口内是否收到来自对端的 UDP 包（有/无）
- 对端侧：对端是否收到来自本机的 UDP 包（有/无/未知）
  - POC 默认要求：对端必须回报“是否收包”（白盒诊断用）
  - 该回报是诊断与解释用途，不作为打洞流程的硬前置；若缺失则视为未知并降低置信度
  - 回报时机（最小语义）：
    - 每个打洞“尝试窗口”结束时，对端回报一次“收包摘要”（收到/未收到）
    - 可选：首次收到对端 UDP 时立刻回报一次（提升解释及时性）
  - 回报路径：
    - 作为控制面点对点消息（mesh 优先；必要时 MQTT 兜底）
    - 端到端加密 + 签名；仅参与的两端可解密（不进入全网可读长期态）
  - 回报内容（概念级，不写结构）：
    - 必含：是否收到来自对端的 UDP（有/无）
    - 可选：对端观测到的“对端端点”（用于判断端点漂移/不一致）

诊断树（v0）：

- 0) 同公网出口（同公网 IP）优先分支：
  - 若本机与对端公网 IP 相同：优先尝试 LAN 候选（若存在）
  - 若公网端点失败且 LAN 也不可用/失败：推断 `PU_LAN_OR_HAIRPIN`
- 1) 收包矩阵（核心证据）：
  - 本机无入站、对端有入站：推断 `PU_LOCAL_INBOUND_BLOCKED`
  - 本机有入站、对端无入站：推断 `PU_PEER_INBOUND_BLOCKED`
  - 本机无入站、对端无入站/未知：推断 `PU_HARD_NAT_LIKELY`（结合 hints 提升置信度）
  - 双方都有入站但仍失败：进入 2)
- 2) 端点一致性与稳定性：
  - 收到“可验真”包的端点与“选中端点对”不一致：推断 `PU_ENDPOINT_DRIFT`（POC：允许自动切换 active endpoint）
  - 端口频繁变化/反复漂移：推断 `PU_ENDPOINT_DRIFT`（并降低稳定性置信度）
  - 端点一致但确认握手反复失败/收包断续：推断 `PU_UNSTABLE_LINK`

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `PU_LAN_OR_HAIRPIN`：同一出口 NAT 下公网回环不通/子网隔离；优先走局域网
  - 证据：同公网 IP；公网端点尝试失败；（可选）LAN 候选不可用/失败
  - 建议：优先 LAN 直连；检查路由/AP isolation；必要时把两端放同一子网重试
- `PU_LOCAL_INBOUND_BLOCKED`：本机 UDP 入站疑似被拦（对端收得到，我收不到）
  - 证据：本机无入站；对端有入站；多轮超时
  - 建议：检查本机防火墙/安全软件/路由策略；换网络；若可优先 v6
- `PU_PEER_INBOUND_BLOCKED`：对端 UDP 入站疑似被拦（我收得到，对端收不到）
  - 证据：本机有入站；对端无入站
  - 建议：让对端检查防火墙/安全软件/路由；让对端换网络；若可优先 v6
- `PU_HARD_NAT_LIKELY`：疑似硬 NAT/对称 NAT/强过滤；POC 可能无解（需要换网或引入网内可达中继节点；不做中心化公共 relay）
  - 证据：双方均无入站或对端入站未知；多轮多端点失败；（可选）`v4_hint/v6_hint` 显示偏硬
  - 建议：换网络（通常最有效）；让其中一端或新增一台节点具备公网可达性（自持）；后续再引入“网内数据面中继/转发”（不做中心化公共 relay）
- `PU_ENDPOINT_DRIFT`：端点漂移/不一致导致候选对不上（映射变化/过滤）
  - 证据：收到包来自非预期端点；或对端观测到本机端点频繁变化
  - 建议（POC，KISS）：
    - 若收到了“可验真”包来自新端点：优先自动切换 active endpoint 并继续（无需立即 STUN 重测）
    - 若切换后仍失败：开启新一轮 attempt（重新自发现→重新交换候选）
    - 若频繁 drift：缩短 candidates 有效期；避免频繁切网；后续再精调
- `PU_UNSTABLE_LINK`：链路不稳（丢包/抖动/MTU 等），有入站但无法稳定确认
  - 证据：双方都有入站但确认失败；耗时明显变长/成功率很低
  - 建议：换族（v6/v4）；换网络；后续再加更细的链路质量指标与解释

置信度规则（最小）：

- 若“对端是否收到本机 UDP”可验真：上述结论多为 `确定/大概率`
- 若对端收包信息未知：只输出 1 个主结论 + 1 个备选，且置信度不得高于 `可能`

`DataplaneHandshake`

> 目标：把“打洞成功但传输握手失败”的原因讲清楚；避免用户只看到一个 timeout。

观测前提（POC 级别）：

- 已确认的路径摘要：`family=v4|v6`、`path=direct|punch`、端点对（见点对点详情）
- 本次传输栈选择：`kcp` 或 `quic`（若 `quic`，记录 `cc=bbr|brutal`）
  - 选择来源（POC，敲定）：默认取决于用户配置（POC 默认 `quic`）；必要时允许命令行覆盖；POC 不做自动协商/自动降级
- 握手结果：成功 / 超时 / 明确错误（例如“协议不匹配”“身份验证失败”等）

诊断树（v0）：

- 0) 栈/参数一致性（最常见）：
  - 若双方 `data-proto` 不一致（`kcp` vs `quic`），或 `quic-cc` 约束不一致：推断 `DP_STACK_MISMATCH`
- 1) 加密与身份绑定：
  - 若握手明确报“证书/身份验证失败”：推断 `DP_IDENTITY_BIND_FAIL`
- 2) 超时类：
  - 若 PunchAttempt 已有稳定双向收包但握手仍超时：推断 `DP_HANDSHAKE_TIMEOUT_UNSTABLE_PATH`
  - 若 PunchAttempt 本身就不稳定/证据不足：回到 `PunchAttempt` 的诊断树先收口

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `DP_STACK_MISMATCH`：双方传输栈/参数不一致（POC 不做自动协商）
  - 证据：本端选择 `kcp|quic(+cc)`；对端回报选择不同/握手报“协议不匹配”
  - 建议：统一两端的 `--data-proto`；若用 `quic` 统一 `--quic-cc`（`bbr|brutal`）
- `DP_IDENTITY_BIND_FAIL`：身份绑定失败（对端身份与控制面预期不一致）
  - 证据：握手明确报验证失败；或能建立连接但身份校验失败
  - 建议：先 `state_pull` 更新对端身份视图；确认对端未被 revoke/未更换 key；必要时重新入网
- `DP_HANDSHAKE_TIMEOUT_UNSTABLE_PATH`：握手超时（链路质量不足/丢包/路径 MTU 等）
  - 证据：PunchAttempt 双向收包成立；但握手持续超时/耗时异常长
  - 建议：优先换族（v6/v4）；换网络；重试；后续用链路质量指标（丢包/RTT）增强解释

置信度规则（最小）：

- “协议不匹配/身份验证失败”属于强证据：`确定`
- “握手超时”必须结合 PunchAttempt 的稳定性与对端回报：否则不得高于 `可能`

`CapabilityHandshake` / `SessionAttach`

> 目标：把“命令选错 target / 会话被占用 / 目标侧环境不满足”清晰区分。

观测前提（POC 级别）：

- 用户输入的 `<peer>/<target>/<session>` 与解析结果（是否发生歧义）
- 单写者锁状态：是否 `in use`
- 连接器执行结果：WSL/SSH 是否成功连到目标侧
- tmux attach/create 的结果（成功/明确错误）

诊断树（v0）：

- 0) target 解析：
  - target 不存在 → `SH_TARGET_NOT_FOUND`
  - target 歧义（命中多个）→ `SH_TARGET_AMBIGUOUS`（要求用 `wsl:`/`ssh:` 消歧）
- 1) 单写者锁：
  - 目标 `(peer,target,session)` 已被占用 → `SH_IN_USE`
- 2) 目标侧可用性：
  - 连接器失败（WSL/SSH 连不上/退出码异常）→ `SH_CONNECTOR_FAIL`
  - tmux 不可用 → `SH_TMUX_MISSING`
  - tmux attach/create 失败 → `SH_TMUX_ATTACH_FAIL`

叶子结论词典（POC v0，白话 + 证据 + 建议）：

- `SH_TARGET_NOT_FOUND`：找不到 target
  - 证据：解析结果命中 0 个 target
  - 建议：运行 `miopunch sh ls <peer>` 查看 targets；必要时用 `wsl:`/`ssh:` 前缀
- `SH_TARGET_AMBIGUOUS`：target 名称歧义
  - 证据：同名命中 ≥2 个 targets
  - 建议：使用 `wsl:<name>` 或 `ssh:<name>` 消歧；或在 config 中改别名
- `SH_IN_USE`：会话已被占用（单写者）
  - 证据：同一 `(peer,target,session)` 已有控制端 attach
  - 建议：先退出占用端再重试；POC 默认不提供 `--steal/--force`
- `SH_CONNECTOR_FAIL`：目标侧连接失败（WSL/SSH）
  - 证据：连接器明确失败（错误/退出码）
  - 建议：确认目标侧可用（WSL/VM 正常、网络可达、凭据正确）；必要时换 target；重试
- `SH_TMUX_MISSING`：目标侧缺少 tmux（POC 必需）
  - 证据：目标侧无法执行 `tmux`（明确错误）
  - 建议：在目标侧安装/启用 `tmux`；或换一个已具备 tmux 的 target
- `SH_TMUX_ATTACH_FAIL`：tmux attach/create 失败（会话名/权限/环境问题）
  - 证据：tmux 明确报错（白盒可直接显示）
  - 建议：检查 session 名是否合法；确认目标侧权限/环境；必要时换 session 名重试

置信度规则（最小）：

- target 解析/单写者锁/tmux 报错属于强证据：`确定`

### 9.5 呈现模板（CLI/UI 同源，敲定）

> 目标：同一套“阶段 + 诊断树 + 叶子结论词典”同时服务 CLI 与桌面端 UI，避免两套口径。
>
> 约束：遵守 9.2 的“非侵入性硬原则”（sidecar/best-effort），可解释性不改变流程与结果。

统一模板（概念级）：

- 摘要：一句话说明“正在做什么/做到哪一步/是否成功”
- 关键事实：默认展示 3–6 条（见 9.1 的边界）
- 阶段时间线：固定 8 阶段的 stepper（开始/成功/失败 + 耗时）
- 失败专属：
  - 诊断路径：输出命中的“问题→分支→叶子结论”
  - 推断链：`事实 → 推断(置信度) → 建议动作(≤3)`
- 展开详情（按需）：
  - 只在用户展开或 debug 时输出更多底层细节（不影响默认可读性）

CLI 呈现（POC 口径）：

- 默认：短摘要 + 当前阶段 + 关键事实（含本机公网映射；直连后含端点对与传输栈）
- 默认不做地址脱敏：用户需要知道“到底连到了哪一个端点”；对外分享时才显式使用 `--redact`
- 失败：自动追加“诊断路径 + 推断链 + 建议”
- `-v/展开详情`：展示“尝试次数/每步耗时/候选数量/端点对（若适用）”等更多证据（仍遵守信息边界）

桌面端 UI 呈现（POC 口径）：

- 必备三块视图：
  - 阶段 stepper（8 步）+ 当前卡点高亮
  - 点对点详情卡片（端点对、family/path、transport/cc、耗时；失败时显示诊断路径）
  - 可复制的摘要/报告（用于粘贴给自己或排障）
- 可选增强：对关键名词提供“hover/点击解释”（不要求用户查文档）

### 9.6 运行态一览无余（网络已运行时，敲定）

当网络处于运行态（已入网、存在邻居/链路）：

- 总览至少要让用户看清：
  - 当前在线 peers（online/last_seen）
  - 当前已建立的邻居/链路（direct/punch；v4/v6；kcp/quic+cc）
  - 对任意一条“已建立直连链路”，可下钻查看端点对与会话/能力状态
- 全局总览默认不铺开所有端点全集；下钻到“我正在连的 peer/链路”才展示端点对（见 9.1）
  - 但对“当前正在操作/选中”的 peer/链路：必须直接展示已选定端点对（避免用户不知道连到哪里）

### 9.7 可复制诊断报告（POC 建议）

- 目的：把“白盒可定位问题”的信息变成一段可复制文本（或文件），便于自查/复现/交流。
- 触发方式（敲定）：
  - 不新增 `miopunch report` 子命令
  - 在 `miopunch join/ping/sh` 上提供 `--report <path>`：将本次运行的诊断报告以 Markdown 写入文件
  - 写文件属于 sidecar/best-effort：写失败只提示 warning，不改变主流程结果/退出码
- 报告模板（Markdown v0，敲定）：
  - `# Summary`：一句话摘要（成功/失败/卡点阶段）+ 目标（peer/target/session）
  - `# Timeline`：8 阶段 stepper（开始/成功/失败 + 耗时）
  - `# Facts`：3–6 条关键事实（遵守 9.1 的信息边界）
  - `# Diagnosis`：`stage` + `reason_code` + 诊断路径 + 推断链（含置信度）
  - `# Suggestions`：≤3 条建议动作
  - 若启用 `--redact`：上述涉及 `IP:port` 与 broker/STUN 的字段输出为 `<redacted>`
- 内容（概念级）：
  - 摘要 + 阶段时间线（含耗时）
  - 命中的叶子结论（reason code）+ 诊断路径 + 推断链
  - 本机公网映射；若已直连则包含端点对与传输栈
- 约束：
  - 报告生成与收集为 sidecar/best-effort；失败不影响流程
  - 不自动长期落盘（默认）；由用户显式导出/复制

### 9.8 落地规则（实现约束，敲定）

sidecar（解释/诊断回报/回执/统计）：

- 所有 sidecar 信息均为 best-effort：允许缺失/延迟/乱序/丢失；仅影响解释的置信度与可展示信息量。
- sidecar 的发送/接收/等待不得改变主流程推进与结果（打洞/建链/握手/能力层）；缺失一律走 `unknown` 分支继续推进。
- sidecar 回报必须走控制面点对点消息（mesh 优先，必要时 MQTT 兜底），端到端加密 + 签名；不进入“全网可读长期态”。

超时与重试（避免拖慢主流程）：

- 主流程的超时/重试与 sidecar 的超时/重试必须解耦；sidecar 只允许短超时与有限重试（或直接不重试）。
- sidecar 超时策略要“宁可缺失也不拖慢”：超过短超时直接降级为 `unknown`，并继续推进阶段机。
- 若 sidecar 信息在稍后到达：
  - UI 可更新诊断卡片/时间线
  - CLI 以“可复制诊断报告/再次查询”方式补齐（避免默认输出刷屏）

事件留存与日志（POC，敲定）：

- 只保留一套日志系统：同一套事件既服务“可解释性时间线/诊断报告”，也服务 debug 日志输出。
- 内存事件环（ring buffer）：
  - 目的：用于 UI 时间线与 `--report` 生成（即使未写日志文件也可生成报告）
  - 粒度：随 `--log-level` 增减（`info` 更少、`debug` 更细）；不额外发明第二套事件等级
  - 形态：环形覆盖（不追求全量留存）；POC 仅要求“能覆盖最近一次操作的关键事件”
- 日志文件（可选）：
  - 若配置了日志落盘路径，则写入单个文件并做轮转覆盖
  - 轮转策略（POC 默认）：单文件上限 `10MB`，最多保留 `1` 个旧文件；超过上限则覆盖最旧日志
  - 若未配置落盘路径：仅输出到 stderr/stdout（由 CLI 捕获/重定向）

报告导出与敏感项边界：

- 报告默认允许包含本机公网映射与“点对点直连详情”（端点对、family/path、transport/cc、耗时），因为这属于用户自身排障信息。
- 报告不得包含“全网端点全集/候选全集”；只包含与本次连接/本次 peer 相关的最小集合（见 9.1）。
- 默认不自动落盘；由用户显式导出/复制。
- `--redact`（敲定）：用于对外分享时脱敏
  - 隐藏所有 `IP:port`（包含本机映射与端点对）
  - 隐藏 broker/STUN 地址（避免暴露网络环境细节）
  - 仍保留：阶段/耗时/reason_code/置信度/建议动作（确保“外发也能看懂”）

## 10. 后续待讨论（不在本次敲死）

- “可解释性”词典扩展：更细的 NAT 解释、指标（吞吐/质量）与可视化呈现
- 文件传输（后置，不进 POC）
- 离网一次性连通性测试（ad-hoc ping code）：不依赖入网/不落盘，仅用于实验/排障（POC 暂不做）
- 网内数据面中继/多路径/缓存（后续增强；不引入中心化公共 relay）
- 安装与分发：Windows service、自启动、Android 打包与更新
