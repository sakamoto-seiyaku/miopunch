# 从当前 demo 出发：已验证能力与个人／小团队产品用途调研

日期：2026-10-12。状态：研究与讨论建议，**不是定位决定、PRD、实施批准或任务清单**。

本轮只读当前源码、设计与证据，并查阅第一方项目、产品文档和标准；仅新增本报告，没有编译、手机测试、路由修改、VPN、部署或 tracker 写入。外部资料查阅日期为 2026-10-12；在线主分支和产品限制可能继续变化，复用时仍须固定实际版本。

## 先给结论

**当前 demo 已经是一块能让获准设备跨网络建立端到端会话、交换真实业务消息，并保存和补齐管理状态的底座。它还没有成为普通用户可用的通用 VPN、文件工具或远程服务入口。** 最大的进展是：浏览器不是只能显示原生设备状态的控制网页，而是有独立身份、入网材料和真实数据路径的参与者。

最值得继续讨论或做窄原型的两条路线是：

1. **按设备访问明确开放的已有服务。** 先把一个现有 TCP 服务接到获准成员会话，原生端提供本地入口，让用户沿用自己的应用；浏览器先参与产品内明确实现的功能。它最贴近“自己的设备与服务，软件自己找路”的原始动机，也避免把首个问题扩大到全系统 IP 接管。但它需要业务桥接、服务边界和实际生命周期验证，不能只加一个设备列表。
2. **已知设备之间的文件投递，浏览器可以收发。** 用户选文件、选设备、对方接受或保存，直接获得可感知收益；既有网络身份与授权能减少反复配对。最窄范围是双方在线、前台完成一次投递，不自动推导离线邮箱、目录同步或匿名分享。它需要二进制长流、文件协议、保存和恢复边界；可以研究复用 iroh-blobs，但现在没有接入。

第一条用于判断底座本身是否能减少用户当前组网维护；第二条用于判断不用先改造其他应用，是否也有足够日常价值。这是**研究排序**，并不替用户选择首版。若第一条最终仍要求用户管理相同复杂度的常驻服务，或第二条不能比已有工具减少操作，两条建议都应撤回。

“能自动配置”应当隐藏找路、联络、重试、连接维护和状态补齐；用户仍要表达加入哪个可信网络、向谁开放什么、是否接收文件、允许哪些资源。减少配置不意味着软件自行扩大授权。

## 1. 本报告采用什么事实基线

当前取向以 [CONTEXT.md](../../CONTEXT.md)、[产品主线](2026-09-27-product-mainline-draft.md)、[场景推演](2026-09-28-network-scenario-walkthrough.md)、[最新产品讨论](2026-10-03-shared-web-client-ui-direction.md) 最后几节和 [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md) 为准。以下既有约束没有被本报告改写：

- 面向个人和小团队；少配置、上手即用、自配置、端到端保护、浏览器与原生参与是体验取向，具体业务功能仍未选定。
- 用户长期使用已有组网工具与 Android 远程客户端满足 SSH／tmux；再提供远程 shell，本身不是切换理由。用户当前部署中的 VPS 费用与中心服务维护是个人痛点，不能扩大成“所有 Tailscale 用户必须租 VPS”。
- frp XTCP 与大陆打洞是历史动机；文件传输、虚拟 IP、Android VPN、root 增强与已有 VPN 共存均是候选。“共存”刚被明确纠正为例子，不是硬性条件。
- 服务 ACL 设计仍暂停。成员资格、应用可访问的服务范围、文件读取／写入许可不能混成一项权限；本报告标出缺口，不制定新 ACL 体系。
- 官方资源可禁用、可替换；信令转交与业务数据中继分开。业务数据中转须获准，不能为了连通隐式打开。
- [ADR 0008](../adr/0008-genesis-recovery-credential-pins-create.md) 与当前讨论已明确：原创世凭证所有副本永久丢失，不能找回旧创世权限；新建网络后迁移是另一操作。本报告不继续展开凭证备份 UI。

仓库根 [README.md](../../README.md) 仍有九月“没有 active runtime”的文字，[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md) 也保留当时“SDK 尚未统一实现”的阶段描述。它们不能覆盖十月当前源码与新实测；同样，归档实现不作为新基线能力证明。

### 1.1 已实测：具体证明了什么

本表主要依据 [10 月 11 日换 Wi-Fi 后复测 README](../evidence/rtc-switched-wifi-retest-2026-10-11/README.md)、[summary.json](../evidence/rtc-switched-wifi-retest-2026-10-11/summary.json)、[sdk-proof.json](../evidence/rtc-switched-wifi-retest-2026-10-11/sdk-proof.json) 与 [route-proof.json](../evidence/rtc-switched-wifi-retest-2026-10-11/route-proof.json)。列出的源码是当前接线入口，**不意味着这些文件所有分支均被本轮覆盖**。

| 能力事实 | 当前代码入口与行号 | 实测范围与保留边界 |
| --- | --- | --- |
| 原生创建网络，浏览器作为独立节点申请加入；审批、成员授权和准入秘密交付并保存 | [native_network.rs](../../src/native_network.rs) 152–171；[network.rs](../../browser-probe/src/network.rs) 2135、2233 | OP13／PJZ110 运行 Native Android 前台 ADB 命令行 SDK；Pixel 6a Chrome 运行真实 WASM。不是正式安装客户端、JNI 或后台交付证明 |
| 保存后双方重开，仍具有相应身份和已完成准入材料 | [native_network.rs](../../src/native_network.rs) 360；[storage.js](../../browser-probe/src/storage.js) 1–55 | 成员授权与准入秘密的持久边界、双方重开通过。浏览器身份与 Native 身份独立；本次不是浏览器首次创建网络或创世恢复证明 |
| 非空管理副本经真实同步、验签、保存、应用；浏览器再次重开仍在 | [offline_replica.rs](../../src/offline_replica.rs) 119–171；[replica.rs](../../browser-probe/src/replica.rs) 40、83 | 本次 4 条管理记录，scope／saved／applied 均为 4；包含独立作者离线后的已保存副本供给。不是任意业务数据、文件或 CRDT 文档同步 |
| 普通成员获得真实 RTC 数据路径，原成员会话上的 PING／PONG 成功 | [connection_coordinator.rs](../../src/connection_coordinator.rs) 96–119、618；[member_connection.rs](../../browser-probe/src/member_connection.rs) 455；[member_message.rs](../../src/member_message.rs) 7–26 | strict purpose 1、fresh final ACK、预期对端、同一原 M／binding 均核对；业务前后 selected RTC，selected relay 为 false。未配置 data relay 或 TURN |
| 未审批节点、真实撤销后的成员请求遭拒 | [admission_gate.rs](../../src/admission_gate.rs)；[management_store.rs](../../src/management_store.rs) 524、552、671 | 本次 open_member 与 sync_replica 负例通过。不由此声称所有上层资源授权、全网瞬时撤销或长文件传输中的撤销已验收 |
| 当前物理蜂窝／Wi-Fi 数据通路有限定的 WAN 证据 | [route-proof.json](../evidence/rtc-switched-wifi-retest-2026-10-11/route-proof.json) | collector 报 physicalData／physicalWanQualified=true：selected nonrelay、Native tuple ownership、mark、精确路由和双向 ICE payload 关联。socket UID 实际观察数为 0，Chrome protect 等仍未知；collector 不独立验证 MESSAGE-INTEGRITY，不代表完整平台／方向矩阵 |
| 独立 relay 路线及 IRC 联络路线也有当前成功样本 | [10 月 11 日蜂窝测试 README](../evidence/op13-cellular-current-2026-10-11/README.md) | relay-only 无 MQTT、RTC 禁用仍通过完整故事；允许 RTC 的 relay 例仍选 relay。IRC 的受限申请成功，随后普通业务经 relay，不是 IRC 承载业务或 IRC→RTC 打洞成功 |

成功复测的成员 SDK 调用约 **7.858 秒**、副本调用约 **8.580 秒**；整轮约 **74.104 秒**，包含审批、重开、负例与清理。它们是单次故事的观测，不能换算成持续带宽、普遍打洞成功率、默认操作时延上限或跨方案性能优势。

**特别容易误读的三点：**

1. 用户先更换 Pixel Wi-Fi，随后使用同一 SDK 构件和原 controller 做了新一轮成功测试；不是在持续文件／SSH 会话中换网并证明旧应用流无损继续。
2. 前一 Wi-Fi 的 MQTT→RTC `NoData` 失败原样保留；网络更换后的成功没有把旧失败变成成功。这说明环境适配与明确失败仍是产品核心问题。
3. 清理成功不等于 SDK 终态收尾完整。Native close 后即时和 100ms 仍有 4 个 socket；本轮不证明完整 terminal socket retirement。浏览器、安装原生客户端的实际关闭、重开、后台状态仍需交付验证。

### 1.2 已存在代码，但尚不能当作用户功能

| 当前代码能力 | 准确入口 | 能复用什么／不能推出什么 |
| --- | --- | --- |
| 共同的用途与建连协调；受限控制阶段与数据放行分离 | [connection_coordinator.rs](../../src/connection_coordinator.rs) 7–14、96–119、618、762；[control_session.rs](../../src/control_session.rs) 302、372 | 可以在权限和有效数据路径成立后取得底层 QUIC Connection。新增应用协议须接进用途、准入、取消与恢复边界，不能直接旁路为任意 ALPN 放行 |
| Native 可供上层使用 Connection／stream | [network_maintenance.rs](../../src/network_maintenance.rs) 265–272；[control_session.rs](../../src/control_session.rs) 372 | 有应用承载基础；没有现成 TCP→stream 桥接、HTTP gateway、SSH 客户端或文件业务 |
| 当前 WASM 普通业务接口是有界文本交换 | [member_connection.rs](../../browser-probe/src/member_connection.rs) 455–489 | `exchange(String)` 限 65536 字节、`open_bi` 后 `read_to_end` 并要求 UTF-8；`serve_exchange` 回显。Native 现成 [member_message.rs](../../src/member_message.rs) 7–26 只接受 PING，接收上限 64B。通用二进制／长流 API 与应用 handler 仍要新增 |
| 管理记录存储与 LogSync 接线 | [management_store.rs](../../src/management_store.rs) 106、524、552；[replica.rs](../../browser-probe/src/replica.rs) 13–14、18、40、83 | 有签名管理历史的验证、因果应用与持久复制。浏览器此交换上限 4096 记录／16MiB；[ManagementAction](../../src/management_action.rs) 8–13 仅有成员添加／移除与管理员任免，不含设备描述、业务目录或任务 schema |
| Native 管理、precode、轮换、恢复、迁移代码；浏览器管理已加入网络 | [daily_management.rs](../../src/daily_management.rs) 95、116、140；[precode.rs](../../src/precode.rs) 58、239、256；[network_migration.rs](../../src/network_migration.rs) 76、307、356；[genesis_recovery.rs](../../src/genesis_recovery.rs) 244、345；[management.rs](../../browser-probe/src/management.rs) 28–111 | 不在本次实机故事中推定这些所有分支都已验证。BrowserNetwork 首次启动生成独立 key 与空管理历史，见 [network.rs](../../browser-probe/src/network.rs) 1610–1640；未见与 Native `create/recover/backup` 对等的公开入口。浏览器可管理不等于平台能力全面对等 |
| 默认资源、用途许可、禁用／替换与更新保留选择 | [resources.rs](../../src/resources.rs) 110–139、231–267、294–336；[native_network.rs](../../src/native_network.rs) 771、817 | Native 默认含获准公共 relay 与观察资源，默认 MQTT 列表为空；实测显式资源配置不能直接称为已交付的全平台“零配置默认”。尚需默认设施可用性、更新和完整产品启动流程验证 |
| 节点维护预算与换网触发入口 | [network_maintenance.rs](../../src/network_maintenance.rs) 48–100、228、265、1176–1246 | 有保守／活跃预算、network_changed、业务 activity 与维护入口；不是已验收 Android 后台驻留、耗电或一键自启动 |
| 普通成员的一跳信令转交与显式业务中转 | [member_transport.rs](../../src/member_transport.rs) 230、294、727–789 | 数据服务代码明确默认禁用，客户端传递不透明端点 QUIC 报文。不能称任意多跳 IP 路由，也不能用现存代码替代当前实机方向／资源撤销／承载连续性验证 |

### 1.3 上游现成功能与本项目新增工作要分开

| 能力 | 第一方依据 | 对当前 demo 的含义 |
| --- | --- | --- |
| 内容哈希、范围读取、可验证流与可恢复文件获取 | [iroh-blobs README](https://github.com/n0-computer/iroh-blobs)、[sendme README](https://github.com/n0-computer/sendme) | 有现成机制可研究复用；当前 [Cargo.toml](../../Cargo.toml) 与 [browser-probe/Cargo.toml](../../browser-probe/Cargo.toml) 没有 iroh-blobs 依赖。协议接线、获准会话、文件许可与浏览器存储不能自动继承 |
| 将本地 TCP 服务与 iroh stream 连起来 | [dumbpipe README](https://github.com/n0-computer/dumbpipe#forward-development-web-server) | 有现成应用层做法和示例；不是 miopunch 当前已提供的端口访问功能。应复用桥接思路或组件，保留本项目身份与权限门控 |
| 可承载自定义数据的模块化复制组件 | [p2panda 官方说明](https://p2panda.org/) | 支持 raw bytes／自定义类型与 CRDT 的上游能力是应用开发基础；当前接线专门消费 ManagementRecord，新增业务日志、授权和合并语义仍是新工作 |
| 系统虚拟网卡与路由 | [Android VPN 文档](https://developer.android.com/develop/connectivity/vpn)、[Linux TUN/TAP 文档](https://www.kernel.org/doc/html/latest/networking/tuntap.html) | OS 提供接入机制，当前新基线没有 VpnService、TUN、虚拟 IP 分配或系统路由接入。root 独立 TUN／策略路由是研究路线，不是已实现、已兼容或首版要求 |

因此，当前底座上新增用户用途至少要补上其中一种：**应用协议与通用二进制流、现有应用接入、文件与设备资源许可、业务状态模型、平台交付生命周期**。不同用途需要的层不同，不能统称“剩下只是 UI”。

## 2. 从真实任务整理出的七个候选用途

这里的“接近”表示复用底座多少，不代表有成品可发布。“用户默认要做什么”是讨论用操作假设，不是新交互决定。所有候选共同要求：网络与端点身份可信、双方当时有获准数据路径、失败如实返回。实时业务要求目标在线；管理副本存在不意味着业务文件也有在线副本。

| 候选用途与普通人的收益 | 现有基础接近程度 | 用户配置负担（候选体验） | 最小新增层／主要风险 | 浏览器／Native／Android 范围 |
| --- | --- | --- | --- | --- |
| A. 访问自己的电脑／家中设备已开放服务；小团队访问成员主动提供的工具 | 较近：成员会话、路径与身份可复用 | 加入设备；服务提供者选一个现有服务；访问者选设备和服务。不能保证完全无需服务端设置 | TCP↔二进制长流桥接、服务发现描述、开放边界、并发与关闭；现有应用认证仍保留 | Native 本地端口／应用代理可行；Android 原生先限支持本地入口的应用；浏览器先限产品内实现的协议 |
| B. 文件投递到自己的另一台设备或获准同伴；减少网盘中转与反复配对 | 中等：身份／准入／连接现成，文件层全缺 | 选文件、选目标；目标接受／保存；已知设备不用每次重新建立网络信任 | 文件元数据、二进制流、完整性、进度／取消、部分文件与恢复、磁盘限额 | 浏览器用户选文件与保存；Native 文件系统；Android 系统文件选择／分享入口需新壳接入 |
| C. 远程读取明确共享的文件夹／成果包；不用先拷走全部内容 | 中等偏远：可连接，目录和文件资源边界全缺 | 提供者选择一个目录；访问者浏览、选取和下载 | 目录协议、路径限制、文件变化一致性、按需读取；避免任意路径访问 | Native 常开提供者更自然；浏览器适合读取界面；Android 仅用户授予的文件范围 |
| D. 浏览器短时加入并完成一次收发／查看；借用设备也能参加 | 参与能力近，安全分享业务中等 | 打开可信网页、接收可信邀请、审批／precode；完成具体任务后关闭／退出 | 上层业务、邀请交付与明确退出；成员权限范围、网页信任、存储和会话回收 | 浏览器是独立端点；Native 提供者／管理员；Android Chrome 不等于 Android 后台客户端 |
| E. 看设备状态、执行明确的一项操作；如查询备份作业进度／请求生成成果 | 小消息承载近，设备控制业务远 | 设备主动开放支持的状态或操作；用户选设备、看结果／触发动作 | 有界 RPC、设备适配器、操作授权、幂等和状态新鲜度；任意 shell 不作为默认业务 | 产品内浏览器和 Native 都可呈现；Android 权限范围限制执行能力 |
| F. 小团队保留与补齐轻量业务记录；如设备作业结果和共享清单 | 管理复制思路近，业务语义远 | 围绕一项任务发布／查看；默认补齐相应授权记录 | 新日志类型、授权范围、存储和删除／冲突语义、离线副本策略 | 浏览器可前台参与；Native 可持有副本；Android 后台收集另验。不是已实现协作文档 |
| G. 通用私有 IP 网／网关接入，让既有应用继续按 IP 使用 | 较远：可复用安全隧道，IP 接入层缺失 | 安装并允许系统网络接入；网关拥有者另表达可访问范围 | TUN、寻址／DNS、IP 封装、路由／MTU、UDP／广播边界、平台权限和后台 | Native／Android 需系统能力；普通网页不能承担任意其他应用的虚拟网卡 |

### A. 按设备访问明确开放的已有服务

**场景与收益（推论）：** 在外面查看家中某个已有管理界面，或给同伴访问自己开发机上的一个工具；对 SSH／数据库等原生应用，让它继续连接一个本地入口。用户获得的是稳定设备与服务选择，减少记地址、开放公网端口和维护固定入口。SSH 可用于检验透明业务承载，但用户已经有成熟 SSH／tmux，不能据此把远程 shell 定为产品。

**复用与最小新增：** 复用成员准入、原会话路径和撤销门控；新增服务端本地 TCP 连接与 QUIC stream 双向桥接，以及客户端本地监听。第一项只接一个明确服务，不能把任意主机／端口扫描或全部局域网开放当默认。现有 [dumbpipe](https://github.com/n0-computer/dumbpipe#forward-development-web-server) 已演示对应桥接，适合用作接入层参考；直接启动另一套独立 Endpoint 会绕开本项目网络权限，须检查接线位置。

**平台与用户操作：** 提供者明确选择已有服务；访问者选择它。Native 可沿用现有应用。Android 普通应用并非都能配置本地代理／端口；浏览器 WASM 的 stream 不自动变成 `https://某节点`，也不会给其他网页或应用建立系统路由。若希望原样打开任意内网网页，还要解决 origin、Cookie、重定向、WebSocket 和 TLS 的完整语义；首个窄验证可用受控服务，不能声称任意 Web 应用兼容。

**关键验证：** 双向长流、半关闭／背压、并发连接、目标服务拒绝、调用取消、长流中真实撤销、路径失效；本地监听不意外扩大到局域网；SDK 完整收尾。若仍要每项手工编排公网配置，或者用户主要需要全系统 IP 兼容，本方向价值会减弱。

### B. 已知设备之间的文件投递

**场景与收益（推论）：** 手机把选中的照片／资料交给家中电脑，或小团队把一个成果包交给明确的同伴；接收方是浏览器也可参与。相较复制命令、临时网盘或重复配对，候选收益是“在已经信任的设备列表里直接交付”。不承诺比现有工具更快，也不把数据不长期托管等同永远不经过中继。

**复用与最小新增：** 复用稳定身份、成员准入、RTC／relay 数据路径；新增 offer／accept、文件清单、二进制流、校验、取消、完成回执和保存。不能将文件塞进现有 64KiB UTF-8 exchange，或塞进管理 Replica。iroh-blobs 的范围请求／校验／恢复是可复用候选；接入获准会话和文件权限仍需实现，见第 3 节。

**平台与用户操作：** 用户主动选文件与目标，接收端表达保存或拒绝；自有设备能否常规自动接收是后续权限选择。[W3C File API](https://www.w3.org/TR/FileAPI/) 提供用户选取文件、Blob 与流式读取的浏览器基础，不是本项目已拥有的收发接口；Android 可用 [Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files) 让用户选择文件／保存位置，现有 ADB SDK 尚未接壳。Native 更适合文件系统与持久恢复；浏览器首轮宜前台、有限范围，文件很大时不能把全量留在内存后再下载当完成方案。

**关键验证：** 多种二进制、比内存大的文件、存储不足、取消与拒收、页面关闭、进程结束、再次取回、同名文件、校验失败与撤销中断。双方离线不交付；若加入第三方持久转存，则新增业务保存与加密／授权问题。若用户实际只做局域网随手传或一次陌生人临时分享，LocalSend／PairDrop／sendme 可能已足够，长期组网会增加负担。

### C. 读取明确共享的文件范围

**场景与收益（推论）：** 在外面从家中电脑取一份资料，或团队成员只查看当前成果包；不要求同步整个目录。相比 B 的“发送者现在选一个文件”，这里是接收者在提供者已明确开放的范围内主动取文件。

**复用与最小新增：** 复用连接、节点身份和成员状态；需只读目录／对象索引、文件版本或快照、范围读取、下载协议，以及限制越出共享目录和符号链接跳转的资源边界。可以复用通用文件协议的合适部分，但 SMB／WebDAV／SFTP 的存在不会自动解决浏览器接入与本项目授权。

**平台与用户操作：** 常开 Native 电脑／NAS 更自然；提供者选共享范围，浏览器／Native 访问者选文件。Android 只能覆盖获系统授权的范围，不能假定完整磁盘开放。不要自动把同网成员资格变成整个文件系统读取权。

**关键验证：** 文件正在修改、文件被删除、只读范围撤回、路径穿越、目录很大、断连和增量读取。它比一次投递更早触及长期资源权限；服务 ACL 暂停的情况下适合保留作候选，不宜认定最小层只是文件浏览 UI。

### D. 浏览器短时参加一个具体任务

**场景与收益（推论）：** 外出时在可信浏览器里收一个资料包，或同伴临时打开网页参加一次明确操作；接收者不必先装完整原生客户端。浏览器参与是 A／B／C／E 的分发方式，而不是“能打开网页”本身就有用途。

**复用与最小新增：** 独立身份、申请／审批、持久入网与真实 RTC 业务均有本次证据。最小新增是一个明确业务、邀请呈现及会话／成员边界。**短时使用不等于临时成员资格：** 当前成员资格没有自动到期，关闭页面只是断连；退出／撤销须遵循已有语义。若要只有一次文件的访问许可或匿名下载链接，那是新授权模型，不由 precode 有效期直接得到。

**平台与用户操作：** 打开可信网页、可信邀请、获得批准，然后执行具体任务。当前 [storage.js](../../browser-probe/src/storage.js) 1–55 以一个 origin 的 IndexedDB 保存节点，用 Web Lock 限制重复激活；共享／不可信电脑不适合自动保留长期成员权限。浏览器 UI、Native UI 可以复用表现，但后台和文件能力不须假定完全一致。

**关键验证：** 页面关闭与重开、同 origin 多标签、清空存储、私密浏览、支持能力缺失、选路回退、长任务暂停和明确退出。网页代码宿主的可信交付仍是端点安全的一部分；“信令服务不能解密”不能防止恶意页面代码读取本地端点秘密。

### E. 状态查看与少量明确操作

**场景与收益（推论）：** 查看某设备备份／生成任务是否完成，读取最新结果，或请求它执行已经安装的一个操作；不必打开整套远程桌面或 shell。只有存在具体设备任务时才有产品意义，通用“设备协作”描述不够。

**复用与最小新增：** 复用有界请求／应答的承载方式，替换 PING／PONG handler，定义状态字段与少量操作、超时、幂等请求标识和结果。不能用“发一个字符串”代替命令权限；也不能把已复制成员名单显示成实时设备状态。

**平台与用户操作：** Native 常开设备提供适配器，浏览器和 Native 读取／触发。Android 客户端仅执行自己有权完成的工作；不因加入网络获得其他应用或 OS 的控制权限。默认用户选择设备的一项已开放功能。

**关键验证：** 断线后重复请求不重复产生副作用、拒绝未开放操作、撤销后不能执行、区分过期结果与当前状态。若每设备都要写复杂插件或用户没有频繁的具体任务，它会退化为开发接口，暂不适合独立首版。

### F. 小团队轻量业务记录的离线补齐

**场景与收益（推论）：** 团队共享作业结果或一份轻量清单，成员离线回来从其他持有副本的成员补齐，原发布者此时不必在线。当前管理记录副本故事证明了这种机制的有限接线，但没有证明该业务已存在。

**复用与最小新增：** 可以研究 p2panda 的字节／日志同步与已有验证存储的经验；应用需另定义数据类型、作者权威、读取范围、编辑／删除／冲突、保留和业务副本策略。管理日志不能无条件扩成所有成员共同保存的业务数据库；端点会话加密也不等于非端点持久业务副本已加密。

**平台与用户操作：** 浏览器可前台发布和补齐，Native 适合持有稳定副本；用户围绕一项清晰任务使用，而非先设置复制拓扑。Android 后台采集、持续在线与耗电另验。

**关键验证：** 三设备、原作者离线、授权读写、重复和乱序、存储损坏、删除传播、离线撤销窗口。如果任务实为文件目录同步或多人文本编辑，其业务复杂度更接近成熟同步／协作产品；当前不建议以“已有 LogSync”为理由直接扩大到这些领域。

### G. 通用私有 IP 与网关接入

**场景与收益（推论）：** 让已有 SSH、浏览器、数据库客户端等继续连接一个网络地址，无需每个应用支持产品协议。它能减少应用侧改动，是 A 无法满足时自然的扩展候选；不是因为现有 demo 连通就已经存在的功能。

**复用与最小新增：** 复用端点安全通路，在其上新增虚拟接口、IP 映射、封装、路由与 DNS、MTU 和平台权限。网关访问其他局域网设备还增加网关边界与目标资源授权。按节点身份组织与虚拟 IP 可以组合：前者回答授权对象，后者提供既有应用入口。

**平台与用户操作：** Native 需要具体 OS 接入；Android 标准 VpnService 每用户／资料只有一个活跃服务，见 [官方文档](https://developer.android.com/develop/connectivity/vpn)。普通应用内业务无需占用这个位置，但不因此保证不受现有 VPN 影响。root TUN、与其他 VPN 集成或策略路由只是可研究方式；共存不是本轮硬性需求，不以它排除标准 VPN 或强制 root。

**关键验证：** 系统路由、IPv4／IPv6、DNS、UDP、MTU、默认网络变化、权限撤回、后台和目标 ROM 的具体行为；再按用户实际需要研究共存。网页不是通用 OS 虚拟接口。若用户接受 A 的明确服务入口，G 会增加大量与首次收益无关的工作，因此不作为最近路线。

## 3. 现有工具已经解决哪些价值，复用成本在哪里

下面比较的是对应任务，**不是竞品全面评分**。资料证明第一方公开能力，不是本轮对这些产品在用户网络、大陆运营商或手机上的实测。

| 第一方产品／项目 | 官方已有用途与限制 | 对 miopunch 的启示（推论／建议） |
| --- | --- | --- |
| Tailscale | 官方区分协调服务承担身份、发现、配置与策略，设备侧数据平面处理 WireGuard 流量；不是用户都要自建 VPS。[control/data planes](https://tailscale.com/docs/concepts/control-data-planes) | 通用私网和现有应用访问价值已经很成熟。可能差异是用户自己持有可验证成员控制状态、减少必需自管中心，以及浏览器作为独立端点；要用完整体验验证，不能仅凭去中心化标签声称更省维护 |
| Tailscale userspace／Serve | userspace 通过 SOCKS5／HTTP proxy 让应用接入，需应用配置；Serve 已支持 HTTP 反向代理、文件／目录和 TCP forwarding。[userspace](https://tailscale.com/docs/concepts/userspace-networking)、[Serve](https://tailscale.com/docs/reference/tailscale-cli/serve) | 不使用 TUN 的服务接入已有人解决。A 应比较设置与日常使用是否实际更简单；若只是换一个隧道品牌，切换理由弱 |
| frp XTCP | 官方说明 NAT 类型影响可用性和稳定性，建议可靠场景用 STCP，支持失败回退 STCP 与继续尝试打洞。[XTCP](https://gofrp.org/en/docs/features/xtcp/) | 当前多路径、获准 relay 与旁信道是在争取可用性，不证明打洞普遍优于 frp。候选价值应落在默认协作和设备授权体验，不落在“永远无中心、必然直连” |
| dumbpipe | 第一方工具已有 stream／TCP／Unix socket 桥接示例，可以通过本地端口访问远端服务。[README](https://github.com/n0-computer/dumbpipe) | A 的桥接无需自创输送原理；但接入本项目新 session purpose、成员／资源门控、取消与长流处理仍要做。直接运行独立工具并不自动取得 miopunch 授权语义 |
| sendme／iroh-blobs | sendme 已有 ticket 文件／目录获取、TLS、位置透明和中断恢复；iroh-blobs 提供内容哈希和范围读取协议，建连由上层安排。[sendme](https://github.com/n0-computer/sendme)、[iroh-blobs](https://github.com/n0-computer/iroh-blobs) | 一次 ticket 投递本身已经解决。B 的差异可能是获准设备列表、浏览器和原生一致的收发、既有网络信任复用；不应以重写文件协议作为差异 |
| Taildrop | 当前官方称 alpha，限定自己的个人设备；不能发给同 tailnet 的其他用户，双方需运行 Tailscale。[Taildrop](https://tailscale.com/docs/features/taildrop) | 小团队成员间和浏览器参与可成为 B 的讨论切口；这只是当前产品限制差异，不能推出 miopunch 其他方面更成熟 |
| LocalSend | 第一方定位本地网络附近设备，不需互联网；跨平台 app 使用 REST／HTTPS。[README](https://github.com/localsend/localsend/blob/main/README.md) | 同 LAN 的简单投递不需要长期私网。B 应验证跨网络、稳定已知设备和浏览器需求是否足够频繁；可参考其选择／接受流程或协议边界，不能直接把 LAN 发现接到 WAN 当已解决 |
| PairDrop | 已有浏览器传文件、持久配对和临时房间、互联网传输；复杂 NAT 可以经 TURN。官方 FAQ 同时指出仍需信任其服务器，计划进一步加密信令／验证。[README](https://github.com/schlagmichdoch/PairDrop/blob/master/README.md)、[FAQ](https://github.com/schlagmichdoch/PairDrop/blob/master/docs/faq.md) | “浏览器跨网传文件”不是空白市场。D／B 更有依据的差异是已有网络成员授权和受保护信令，但邀请来源、网页代码信任、文件许可仍需完整闭环。不要把网页 TLS 或 WebRTC 加密与预期端点验证混为一项 |
| Syncthing | 官方说明有文件索引、块级同步、冲突副本等业务语义；无法直连可经 relay，端点间仍加密。[同步语义源文档](https://github.com/syncthing/docs/blob/main/users/syncing.rst)、[relay 源文档](https://github.com/syncthing/docs/blob/main/users/relaying.rst) | C 的按需读取、B 的一次投递与持续同步不同。若要长期镜像目录，这些业务语义会成为主要工作，远超当前管理副本；不能以“四条记录同步成功”替代成熟同步能力 |

### 3.1 两项较自然的组件复用，仍须核查的接缝

**TCP 服务桥接：** dumbpipe 提供一个连接对应本地 TCP 连接的可读示例。[当前 Cargo.toml](https://raw.githubusercontent.com/n0-computer/dumbpipe/main/Cargo.toml) 为 0.39.0，依赖 iroh 1.0.0 范围。与本项目固定的 iroh 1.2.0 fork 处于同一主要版本，并不证明能不改代码直接接入；本项目用途协商和严格成员 data gate 是新增约束。低成本是复用桥接模型与合适代码，而非启动一条绕开网络授权的并行连接。

**文件获取：** [iroh-blobs 当前 Cargo.toml](https://raw.githubusercontent.com/n0-computer/iroh-blobs/main/Cargo.toml) 为 0.103.1，依赖 iroh 1.0.0 范围，区分浏览器 target，但默认包含 fs-store／rpc；浏览器字段存在不是完整 WASM 文件产品兼容证明。[sendme 当前 Cargo.toml](https://raw.githubusercontent.com/n0-computer/sendme/main/Cargo.toml) 为 0.36.1，使用该 blobs 版本。应检查是否可在**已获准的原会话**上调用协议、浏览器能用哪类 store、是否必须调整默认 feature，以及怎样保留本项目 RTC 与 purpose 规则；本轮没有构建或做兼容性实验。

一个实际影响复用的最新事实：原作者在 **2026-10-08** 发布 [iroh-blobs push 门控修复](https://www.iroh.computer/blog/iroh-blobs-push-fix)，修复默认情况下意外允许远端写入 store 的问题，给出 0.103.1 及若干旧系列补丁，sendme 对应更新为 0.36.1。这不构成当前 miopunch 的已知漏洞，因为现在没有该依赖；它说明文件层不能只检查能否连通和内容哈希，必须固定含修复版本，并逐项核查 get／get_many／observe／push 的许可、写入量与磁盘边界。上游 README 还保留生产质量提示，因此不在报告里替用户决定版本。

**为什么不直接推荐一个“全协议框架”：** 网络身份与准入当前已有具体实现；新增文件协议或 TCP 桥接只解决对应上层任务。p2panda 模块化组件可研究复用，但不能因此同时引入新的文档、文件、聊天或所有 CRDT 业务。这里研究的是最小接缝，不做框架选型。

## 4. 建议怎样缩小下一次讨论或原型问题

两个推荐方向可以共享现有底座，但检验不同价值，不需要同时实现。下表的范围和判定仅为建议。

| 讨论／窄原型建议 | 要回答的产品问题 | 最小可观察行为 | 反证与停止条件 |
| --- | --- | --- | --- |
| A：明确开放的一个已有 TCP 服务，先验证 Native 原有应用复用；浏览器参加已支持的产品功能 | 用户能否保留熟悉应用，同时减少地址、必需控制服务维护与重复配置？ | 提供者明确开放；成员用现有应用接入；拒绝未开放服务；取消／撤销／路径失败清楚；关闭后资源确实收回 | 首次设置仍需每用户租管固定服务；每应用大量手配；用户需要通用 IP 才能工作；只有 SSH demo 且没有现有方案之外的收益 |
| B：一份选中文件，Native↔浏览器，双方在线前台投递 | 浏览器参与与已知设备信任复用，能否降低一次真实交付的操作负担？ | 选文件和设备、接受／保存、显示完成；二进制和大于内存文件；关闭／取消／拒收不误报成功；权限撤回不继续交付 | 体验比 PairDrop／LocalSend／sendme 更繁琐；常用文件任务主要是全自动同步；没有浏览器或跨网需求；依赖默认公共资源后维护成本仍与痛点相当 |

**推荐依据（推论）：** A 接近用户原始私有组网动机，B 提供无需外部应用改造的直接收益；两者的新增层相对明确，也都能暴露底层长流、撤销、关闭和环境适配是否足够。C／F 提前引入长期业务状态与资源语义；G 提前引入系统路由和权限矩阵；E 需要先有具体设备任务才有稳定需求。D 应贯穿具体用途，不能单独以“浏览器也能连”作为产品定位。

**不成立时的替代解释：** 用户可能真正想拥有可嵌入小工具的安全组网底座，而非替代一整个 VPN 或打造独立文件产品。现有 Native Connection 与 WASM 有界 exchange 提供这种探索依据；但成为外部 SDK 仍需通用二进制流接口、稳定 API、错误与取消语义、生命周期和文档。若终端用户收益不成立，不能靠把底层称为“平台”自动证明另一个产品成立。

## 5. 自配置可以藏什么，仍必须让用户决定什么

| 应由软件处理的复杂度 | 用户仍要表达的意图／不能自动绕过的约束 |
| --- | --- |
| 实际平台能力探测、可用路径选择、观察地址、NAT 协商、获准 relay 回退与重试 | 加入可信的哪个网络、接受哪个身份／邀请；不能根据发现服务声称是成员就自动批准 |
| 已保存身份与成员状态恢复、缺少管理记录补齐、继续联系与合理退避 | 新成员和管理员授权、退出／撤销、哪些应用资源开放；成员资格不能自动开放所有本地服务 |
| 默认公共资源随交付维护，邀请或已认证配置提供自建入口 | 被禁用／替换的设施与用途继续有效；所有获准入口失效时报告暂不可达，不能暗开禁用项 |
| 前台业务优先、节点维护预算和取消传播 | 手机流量／电量与后台运行意愿；不能为连通自动开启业务数据中转或 root |
| 文件流量、部分数据、进度与完成回执等应用细节 | 选哪些文件或目录、向谁交付、是否接收／保存；内容哈希不代替读取／写入授权 |

这些原则有 [ResourceConfig](../../src/resources.rs) 231–336、[维护预算](../../src/network_maintenance.rs) 48–100 和 [成员数据中转](../../src/member_transport.rs) 727–789 的代码基础；默认交付、用户操作和全平台验证仍未完成。Android [官方后台任务指南](https://developer.android.com/develop/background-work/background-tasks) 也区分前台、异步和不同后台任务方式，不能把 ADB 前台 SDK 成功外推成长期后台客户端。

端到端保护的当前优势是辅助联络／转发设施不因搬运消息获得端点会话业务解密权。它不等于匿名、不隐藏所有元数据、不保证恶意端点安全，也不替代文件落盘后的保护。无需每个用户自购 VPS 的目标，与仍依赖当时可用的观察／联络／数据 relay 是兼容的；“不用自己租管必需中心”不等于“没有设施成本、永久免费或不限带宽”。

## 6. 当前能力结论仍应保留的复查点

- 本次最完整的证据是 OP13 Native 与 Pixel Chrome 之间的蜂窝／新 Wi-Fi 故事，普通成员业务由 Browser `open_member` 发起；其他平台组合、发起方向、安装客户端和后台不是它的证明范围。
- 管理副本成功、成员业务成功与 route collector 限定证明是三件事，应同时保留各自 scope。未知 socket UID／protect、不独立验 MESSAGE-INTEGRITY、完整终态收尾和完整矩阵的限界不抹去。
- 当前 Native／WASM 应用层分别为 PING／PONG 和有界文本交换。候选 A／B 无论采用现成组件还是自写薄层，都必须新增真正的应用 handler／长流接入与对应授权检查。
- 浏览器加入与管理有代码，当前本次证据没有证明浏览器首次 create、创世 recover 或备份交付；多平台取向不自动要求首版功能全面对等。
- 最新已保存的设计文档包含阶段文字；当前能力判定优先引用十月源码与精确测试，不把九月“尚未实现”或 archive 的功能覆盖到新基线。

这些是后续事实核对与讨论的边界，不是本报告新建的验收规格或实施任务。
