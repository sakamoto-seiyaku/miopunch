# 2026-04-15 Alpha/POC 讨论记录（临时）

> 状态：临时讨论记录（非最终 spec / 非 roadmap 承诺）。
>
> 目标：把本次讨论中**最终敲定**的方向与规则沉淀为可执行口径；后续实现以 change 为准。
>
> 约束：本文只写**语义/流程/规则/验收口径**，不写具体数据结构/字段/函数签名。

## 1. 产品定位（最终口径）

- `miopunch`：面向个人/小团队的、跨平台（Windows / Linux / Android）的**私有直连组网工具**。
- 核心差异点：**高度可解释性**（用户一眼能看懂“现在在做什么、为什么通/不通、当前走哪条路径”）。
- POC 不再是“打洞演示”；必须提供一个真实可用能力。
- `web` 只能作为辅助面（控制台/配对/状态展示）；**不能替代客户端**。
- 加密遵循业界最佳实践（复用标准库/成熟方案），不自创密码学。

## 2. Alpha/POC 首个能力：远程 Shell（敲定）

- 首个 POC 能力：**远程 Shell（PTY relay）**。
- 不把“访问对端 HTTP 服务”作为首个主能力；文件传输是否进入 Alpha 未敲死。
- 必须支持**纯 CLI** 使用方式（任意终端可用）；UI/web 仅作为辅助入口与解释面。

### 2.1 平台角色（敲定）

- 被控端（提供 shell 目标的一侧）：**Windows host、Linux**。
- 控制端（发起连接的一侧）：**Windows、Linux、Android**。
- Android：**仅控制端**（POC 不做 Android 被控常驻）。

### 2.2 Windows 被控端：目标与连接器（敲定）

- `miopunch` 常驻运行在 Windows host（真网卡上），**不要求**在每个 WSL/VM 里跑 agent。
- Windows 对外暴露的 shell 目标仅来自 Linux 环境：
  - `wsl:<distro>`：WSL/WSL2 发行版
  - `ssh:<name>`：本机 VM（通过 SSH）
- 连接器：
  - WSL：**ConPTY + `wsl.exe`**（不要求 WSL 内启用 `sshd`）
  - VM：`ssh` 连接器（VM 内只需 `tmux`）
- 不支持（POC）：Windows 原生 `powershell/cmd` 作为 target。

### 2.3 会话语义：现场锚定在 tmux（敲定）

- “现场”定义：目标侧的 **`tmux session`**。
- 进入/恢复语义固定为：`tmux new -A -s <session>`（存在即 attach，不存在即创建）。
- 不支持“接管一个已经在跑的任意终端 tab/PTY”；只管理 `miopunch` 创建/管理的会话（即上述 tmux 现场）。
- 断线/控制端重启/被控端 `miopunch` agent 重启：都等价于 `tmux client` 断开；**重新 attach 即恢复现场**。
- 被控机系统重启：tmux 进程消失；**不保证现场仍在**（POC 不引入更重的恢复方案）。
- `miopunch` **不实现自己的 multiplexer**；直接依赖 `tmux`。

### 2.4 单写者（敲定）

- 默认单写者：同一 `(peer,target,session)` 同时只允许一个控制端 attach。
- 其他 attach 默认报错 `in use`；`--steal/--force` 作为后续能力（POC 先不做）。

### 2.5 运行模式：常驻 vs 临时（敲定）

- 常驻（owned devices）：
  - 被控端长期运行 `miopunch`（开机自启），维护 targets / 会话列表 / mailbox。
- 临时（temporary devices）：
  - 临时机器下载二进制后用一次性授权加入，退出即清理；不要求长期驻留。

## 3. CLI 与配置（敲定）

### 3.1 配置自动加载（可省略）

- 配置是可选的；存在则自动加载。
- 加载顺序：**先匹配先用（不 merge）**
  1. `--config <path>`
  2. `$MIOPUNCH_CONFIG`
  3. 二进制同目录：`miopunch.{yaml,yml}` 或 `config.{yaml,yml}`
  4. 默认路径：`os.UserConfigDir()/miopunch/config.yaml`

### 3.2 最小命令集合（短、可记）

- 列 peer：`miopunch ls`
- 进入/恢复 shell：`miopunch <peer> sh [target] [-s session] [-4|-6]`
- 列 target / session：
  - `miopunch <peer> sh ls`
    - 0 个 target：报错 + 提示先发现/配置 target
    - 1 个 target：直接列该 target 的 tmux sessions（输出头部显示 `target=<name>`）
    - ≥2 个 target：列 targets，并提示 `miopunch <peer> sh ls <target>` 查看 sessions
  - `miopunch <peer> sh ls <target>`：列该 target 的 tmux sessions

### 3.3 target 选择规则（隐藏类型、歧义才显式）

- target 来源：
  - WSL：distro 名（`wsl.exe -l -q`）
  - VM：config 中定义的 ssh shortcut 名
- 显式指定 target：
  1. 写了前缀 `wsl:<name>`/`ssh:<name>`：强制按该类型匹配
  2. 否则：按名字在全部 targets 里匹配（建议大小写不敏感）
     - 命中 0：报错 + 提示 `miopunch <peer> sh ls`
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

- 可通过终端 profile/快捷方式把常用命令固化，实现“一键恢复到同一会话”（例如 Windows Terminal profile 直接跑 `miopunch <peer> sh ...`）。
- 桌面端 UI 若提供“进入/恢复会话”入口，优先拉起用户常用的终端软件；不强制要求内嵌终端。
- 桌面端 UI 应优先展示“当前可恢复/可进入”的 target 与 session，再由用户点选进入。
- 重命名语义：
  - session 重命名：沿用 `tmux` 原生命令/快捷键（POC 不额外发明 rename 子命令）
  - peer 别名 / VM shortcut / target 别名：通过 config 修改

### 3.6 peer 标识与发现（敲定）

- 每个节点同时具备：
  - 稳定 `peer_id`（由身份密钥派生/哈希短码即可）
  - 人类可读 `name`（可在本机 config 中设置）
- `miopunch ls` 默认展示：`name` + 在线状态；`-v` 才展示 `peer_id`。
- CLI 中的 `<peer>` 解析规则：
  - 优先按 `name` 匹配
  - 若重名/歧义：要求使用 `peer_id`
- 发现 vs 配置：
  - `miopunch ls` 以“在线发现”为主（来自 control-plane 的 presence/hello）
  - config 只承担本机参数 + 可选别名映射；不要求用户预先配置全网 peers

## 4. 组网/控制平面（Alpha/POC 口径，敲定到事件与规则）

### 4.1 目标/边界

- 目标：形成可用的私有网络，让多个 peer 能互相发现、入网、恢复、管理。
- 不做（POC）：
  - 完整 ACL 体系/策略语言
  - 数据面 relay（控制面可中继）
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
- 可接受的元数据：
  - broker 仍可能看到连接层元数据（连接 IP、时间/频率/密文大小、订阅数量等）
  - POC 不做 cover traffic / 代理 / Tor 等强隐匿

### 4.3 控制面消息安全（必须）

- 所有控制面消息：**端到端加密（AEAD）+ 签名认证**。
- 允许使用的标准方案：
  - 签名：Ed25519
  - ECDH：X25519 + HKDF
  - AEAD：AES-256-GCM（或同等级标准 AEAD）
- broker/中继只能重放/丢包/DoS，不能伪造“有效管理动作”。
- 数据面（payload）加密口径：
  - QUIC：内建 TLS（端到端）
  - KCP：在其上跑 TLS 1.3（端到端），并将对端身份绑定到控制面已知的 peer identity（防 MITM）

### 4.4 “全网可读” vs “仅收件人可读”（敲定）

> “全网可读”不等于明文；含义是“所有成员可解密”（对外仍是密文传输）。

- 全网可读（成员可解密）：用于长期态信息
  - membership/声明集/治理快照头/`presence`/reachability hints 等
- 仅收件人可读：用于敏感短期态
  - STUN 结果、candidate 集、端点/端口、打洞证据等
- 转发不需要解密敏感载荷：
  - 仅在明文 header 中携带最小路由字段（`dst_peer_id`、`msg_id`、`hop_limit`）

### 4.5 传输策略：mesh 优先 + MQTT 兜底（敲定）

- 只要已有任意 dataplane 邻居链路：控制面消息优先走网内转发。
- 仅在以下情况下使用 MQTT：
  - 未入网/无任何邻居
  - 直连/邻居转发超时
  - bootstrap/recover 的 request-response 类消息

POC 路由策略（最小）：

- 1-hop flooding：
  - `hop_limit=1`
  - `msg_id` 去重（带 TTL 窗口）
  - 不回传给来源邻居

### 4.6 密钥分层（最终口径）

- `invite_secret`（PSK/邀请码）：
  - 仅用于派生临时入网入口（invite topic/mailbox）与 join 阶段加密/认证
  - 可设 expires/max_uses；轮换＝生成新 invite
  - **永远不等于 `net_secret`**
- `net_secret`：
  - 网络根 secret，用于派生入网后稳定的控制面命名空间与加密 key
  - **仅在入网批准（或 PSK 自动批准）后交付给 joiner**

## 5. 入网（join/approve）语义（敲定）

### 5.1 两种入网模式

- `mode=approve`：需要审批
- `mode=auto`：自动批准（纯 PSK 模式）

共同规则：入网入口都基于 `invite_secret` 的临时入口；不复用 `net_secret`。

### 5.2 入网材料（invite）包含什么（概念级）

- broker 选择（可省略→用内置默认）
- `invite_topic`
- `invite_secret`
- `expires_at`（可选/可很长）
- `mode=approve|auto`

### 5.3 membership bundle（批准后下发，端到端给 joiner）

- `net_id`/`net_secret`
- owner/admin 公钥集合（验签信任根）
- 全量 peers 长期态信息 + 声明集合（approve/revoke…；逐条验签）
- peers 长期态边界（概念级）：
  - 包含：`peer_id`/`name`/公钥/角色（owner/admin/member）/`v4_hint`/`v6_hint`
  - 不包含：STUN/candidates/公网端点列表/端口映射细节
- seed peers（2–3，含 approver/admin）
- bootstrap 推荐（初始 2 个）

约束：

- 不包含 STUN/candidates 等短期元数据；这类永远点对点加密。

### 5.4 入网握手（事件序列）

1. 管理节点生成 `invite_topic + invite_secret`，在有效期内监听 `invite_topic`
2. 新节点拿到 invite（扫码/粘贴），连接 MQTT
3. 新节点生成 identity key；再生成随机 `reply_topic` 并订阅
4. 新节点向 `invite_topic` 发送 `join_request`（用 `invite_secret` AEAD 加密；携带 `reply_topic` + 自己公钥/指纹）
5. 管理节点处理：
   - `approve`：进入 pending，人工批准后继续
   - `auto`：直接批准
6. 管理节点向 `reply_topic` 回 `membership bundle`（端到端加密给新节点）
7. 新节点收到后开始 bootstrap，连上 1 个邻居即可视为入网成功；随后补齐邻居
8. 新节点一旦拿到 `membership bundle`，后续控制面即切到由 `net_secret` 派生的正常 inbox / mailbox / control-plane；不再依赖 invite 入口

补充：

- Joiner 在 join 成功后不再监听/使用 `invite_topic`
- 管理节点是否继续监听该 `invite_topic`，取决于该 invite 的 `expires/max_uses`；达到条件后停止监听

入网成功口径：

- 与任意 1 个 bootstrap peer 建立可用通道并完成确认（最小 payload 交换/确认事件）。

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
- 重放/去重：presence 也带唯一消息 ID，并按去重 TTL 丢弃重复消息

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
  - Approver 回 `bootstrap_more_response`（2 个新 peer）
- 最多重复 2 次（总计最多尝试 6 个 peer）；仍失败则 join 失败并提示“环境过硬/节点不在线/未来需要 relay”。

Approver 选 peer 的最小规则：

- 先按桶逐级放宽（direct/easy → hard1 → hard2/unknown）
- 尽量从 online 池取；不够则放宽
- 桶内随机/轮换
- 必须去重（不得重复 joiner 已尝试的 peer）

### 6.5 入网后的邻居维护（避免单点）

- bootstrap 只负责“第一跳”
- 入网后：按 `k = max(2, ceil(ln(n)))` 维持邻居数（n 为已知 peer 数）
- 选择策略：桶内随机，逐级放宽，避免所有节点都只连“最好连的那台”

### 6.6 Rejoin/Recover（重启/换网/长期离线统一流程）

本地持久化（最小）：

- identity key
- `net_id`/`net_secret`
- broker 列表
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

### 7.1 状态收敛：owner 签名 snapshot 链

- admin/owner/config 这类“带删除语义”的状态，使用 **owner 签名 snapshot 链**：
  - 每个 snapshot 带 `prev_hash`
  - 所有节点以链头为准
  - 分叉极少见；如出现，用确定性规则选 head（高度优先；再按 hash 兜底）

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
- 若目标是 admin 且要执行 `revoke member`：必须 owner 级验证/签名（最高等级验证）

危险动作 UX 护栏（敲定）：

- `revoke member` 必须二次确认（例如 `--dangerous` + 确认短语），并明确提示“该 key 永久不可恢复（需换新 key）”

### 7.3 revoke 语义（核按钮，敲定）

- `revoke member`：永久拉黑该身份公钥（不可恢复）
- 重新回来：必须换新 identity key 再 join
- 不用 revoke 来降权：降权走 admin 集合变更（owner-signed snapshot）

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

## 9. 后续待讨论（不在本次敲死）

- “可解释性”事件模型：要展示哪些关键事件/为什么失败/当前路径与 NAT 解释
- 文件传输是否进入 Alpha
- 数据面 relay/多路径/缓存（后续增强）
- 安装与分发：Windows service、自启动、Android 打包与更新
