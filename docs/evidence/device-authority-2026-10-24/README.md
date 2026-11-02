# 设备权限与完整节点恢复验收

## 2026-10-29 实机补充

当前 Native 构件已完成 Windows/Linux 两方向的具体权限确认、凭证托管与恢复、完整节点替换；修复共享 SQLite 关闭竞态后的构件还完整通过双方向审批／precode 账本续办及独立文件读取，16 个全新 Windows 申请节点初始化均通过。原地址前置失败、主线程误改 CLI 断言的失败、Windows OS5 原始失败及清理记录均保留。详见[本次实机资格与构件来源](../device-authority-2026-10-29/current-platform-qualification.json)。

SQLx 0.8.6 在连接归还与池关闭交错时可能在仍有连接的情况下返回；真实 SQLite 同时序测试 RED 为剩余连接 1，修复后为 0。共享存储关闭现在等待晚归还连接的 worker 真正结束，覆盖创建、加入、恢复和备份。Linux 465 Fresh／2 rebuilt、Windows 526 Fresh／2 rebuilt；WASM 实际 229 个输入未变，未重建。历史 Windows OS5 的具体占用句柄未逐个建立，不能一概归因于此竞态。

以下旧记录中的 Windows 授权／账本待运行状态已被本次实际运行补充；原图来源保持不变。2026-10-29 随后的当前 Native 效果验证补齐旧凭证交付与组合备份权限缺口，主线程据此完成 M116 非 UI 验收；后文 UNPROVEN／PENDING 是保留的历史状态。详见[实际效果资格](../device-authority-2026-10-29/effect-qualification.json)。

MIOPUNCH-116；比较起点 `7da3b11d46b8342851874c502c998c86a63667c5`。公开操作见[接口说明](../../notes/2026-10-24-device-authority-and-node-replacement.md)。这里记录非 UI SDK／JSON host／真实 Chromium 的实际观察，原始秘密状态不进入 Git。

## 构件与来源

所有图使用锁定依赖、Rust 1.98.1、仓库共享 `target/` 及缓存租约。构件通过 `scripts/pin_artifact.py` 固定到共享内容存储；运行控制器只验证和复用构件。各次源文件、锁、编译器、依赖输入、原始编译输出、控制器启动源码和结束漂移检查保留在 `.artifacts/runs/`。

| R1 前图 | 结果 | SHA256 |
| --- | --- | --- |
| Linux Native5，dev/default | 465 Fresh / 2 本仓库构件，21.03 秒 | `68e1c25bfb57afc80725459d6dc7156539e602559adb1a221b3eda89c5b5b87e` |
| Windows Native5，dev/default | 526 Fresh / 2 本仓库构件，26.66 秒 | `7871cb1d68b8dbd1958910a6d104eb0cfae7cf96ccfefd8f33739ed27452c24d` |
| WASM5，dev/rtc-probe | 295 Fresh / 1 本仓库构件，15.69 秒及固定 bindgen | 包 `e35aa2d4cd4c7d9d666b6fa00f94b66698dd870205b4d73ad7d60868cf1aca81` |

R1 前 Native SDK 库为 `697cfb416b108f0924ed309b2a02db1f8b8503d63f7fd3a69b88fcd40ec1acaa`。WASM4→5 的 220 个选中输入只改变了共享 coordinator 的 Rustfmt 排版，源码等价检查见 `m116-high-20261024/wasm4-to5-selected-equivalence.json`；恢复逻辑保持原样。旧图的实际结果保持各自来源标记，不改写成新图实机结果。

## 实际观察

| 范围 | 原始运行记录 | 结果及边界 |
| --- | --- | --- |
| 具体创世确认，两端 Native 角色互换 | `m116-native-confirmations-1` | 实际 Windows laptop／Linux host：远端待确认、同结果重试、另一动作不可借用、离线降权、普通业务保存。 |
| Native 托管、旧管理员离线恢复、管理员远端退出 | `m116-native-custody-2b`、`m116-native-custody-2b-windows-observed`、`m116-admission-breadth-3b` | 第一完整运行保留 FALSE；缺失的 Windows 方向后来实际通过。四种 manual/precode × 两 OS 管理端入口真实通过，没有添加启动睡眠。 |
| Native 同身份替换、旧授权快照 | `m116-native-replacement-3b` | 两 OS 互换：源停止、目标隔离、当前联系人核对、身份／根／恢复确认／ACL 保留，实际保存，签名回执清理，无 Leave；旧降权／移除快照不得恢复原权限。 |
| 完成恢复后的原结果跨备份 | `m116-recovery-backup-green-4b` | 同原操作和授权，备份恢复后仍为 Complete；fresh TLS 重试保持原授权，不增长记录。原 Native3b RED 保留。 |
| 已完成恢复 Browser→Native→Browser | `m116-recovery-portable-5b` | 同身份／尝试／完成证明／原授权保留，两运行时重新证明后仍返回原 grant，Native 二次导出的管理记录字节及最终 Browser ID 列表完全相同。隔离前恢复重试拒绝；两种源签名退役且无 Leave，实际 OPFS 保存。 |
| Browser 逐项确认与双向文件 | `m116-browser-confirmations-4b-2b` | 真实独立 Chromium/WASM 与 Native：远端结果幂等、根未复制、离线降权、旧通用窗口明确拒绝；Native 文件与 OPFS 实际保存并独立验 SHA256。 |
| Browser 托管及四种恢复端组合 | `m116-browser-custody-4b-4` | 两种托管源均实际保存、重开读回、重复回执、普通／显式风险删除。WASM/Native 联系人与恢复端四组合均 Complete、原授权重试、证明前业务拒绝、降权后旧授权不能再授予管理员；四次实际保存。 |
| Native→Browser→Native | `m116-browser-portable-4b-2` | 身份、ACL、根、恢复确认、未完成确认、双方确认的托管提议及原结果保留；实际 OPFS／Native 保存。源暂停／退役、旧降权／移除快照拒绝均观察到。 |
| Native 审批与预编码账本 | `m116-native-backup-ledgers-local-4b-2` | 当前本机真实节点：pending、原 phase1 grant 续交且不增长记录、拒绝不能变批准、消费次数保持 1、原结果续办、恢复旧码停用、新码可用，实际保存及申请者原材料代次保留。此项不代替新增 Windows 实机资格。 |
| Browser→Browser→Native 完整广度 | `m116-browser-backup-breadth-5` | Native5/WASM5 全运行通过。实际活动传输见到 65536 字节后停止，无成功回执或部分文件；原端暂停、错误回执拒绝、显式中止、幂等退役。审批／拒绝／原 grant／旧码账本保留；来宾迁移后实际接收确认及保存，旧快照收到当前撤销或真实截止后拒绝。四种用途的禁用项及仅当前允许 relay 集合保留；普通 Browser／Native 退出同结果重试、业务拒绝。 |
| Native 来宾替换专门入口 | `m116-browser-backup-guest-native-5` | 同构件扩展完整广度：来宾实际 Browser→Native→Browser 保持同身份与原截止，Native 专门的来宾完成入口及实际接收确认／保存通过，Browser 与 Native 源均按签名回执退役；原撤销、到期和其他广度断言仍通过。 |

公共 SDK 检查不重新解析依赖，直接使用固定 SDK 与编译器构建独立消费者：`m116-public-device-consumer-6` 三项通过（传统备份 policy／恢复确认及隔离前接收目录不能创建；真实文件系统保存失败、错误回执和显式中止；未完成轮换拒绝及完成轮换的提案／确认／材料保留），`m116-public-operation-consumer-6` 五项通过（具体描述／网络／签名绑定；移除重加入新授权期；后来成为管理员时旧普通 Remove／Leave 不得绕过创世确认；降权后的原结果保留；托管接收端及新挑战绑定）。

新增 Windows 账本运行尚等待本次远端临时测试／测试胶囊传输的明确授权。当前 Windows5 已编译，不能据此声称该项实机通过。

## 原始失败与修复

- 原 Native 目录备份丢失服务历史的公开断言 RED 保留于 `m116-backup-public-red-2b`；相同 revision 断言在 `m116-backup-public-green-3b` 通过。随后原恢复结果丢失的 RED 为 `m116-recovery-backup-red-3b`，Native4b 的同操作原授权查询及重试通过。
- 完整备份一致性类别的原始单调时钟为 `91990.496306355`。统一十组件扩围在 `94132.643993942` 进行，较原 1800 秒阈值晚 `342.147687587` 秒；该迟延、原失败和类别范围保留，没有按子分支通过重新计时。
- Browser 恢复原失败 `96791.911175037` 是保存新入网密钥时提前关闭根证明会话；修正后第一次 Complete 通过，同尝试重试又暴露完成 phase 回退问题。两端保留单调 phase 后，四种端组合完整通过。普通业务仍绑定当前资格，不能利用恢复连接保留绕过权限。
- Native 导入当前 Browser 禁用配置的原失败保留于 `m116-browser-backup-breadth-4b-4c`。旧配置清理误删 Signaling／DataRelay 标记；修正施加顺序后，Native5 同四用途断言和实际有效配置在 `99540.466907436` 通过。
- 控制器错误与产品失败分开保留：JSON 字符串解码、申请者才持有的材料代次、issuer→provider 签名状态交付前置、邀请哈希参数等。修正实际前置或接口类型，保留原操作、原授权／记录／实际保存核心断言；没有把失败重新标为通过。
- 一次 Native Windows 初次普通申请超时的确切原因未建立。后续同构件缺失方向和四入口真实观察通过，不声称已修复该原超时，也不将其武断归为环境问题。

## 清理与限制

通过的运行均保留控制器启动源码和 `controllerSourceDrift=[]`。Browser 关闭上下文、结束读取线程并核对进程组消失；Native 停止所属宿主，移除本次私有状态目录。实际卡片、根、入网秘密和完整胶囊仅在私有临时目录或 RPC 管道中使用；公共日志保留身份、签名结果、计数及文件摘要。

SSH 仅用于已授权的 Native 部署、控制和测试材料交付，业务传输走 miopunch。Browser 实际路径包含普通 `aps1-1.relay.n0.iroh.link` relay；Native 结果另保留实际选中地址。这里不声称无限制 WAN／NAT 覆盖、全局最新状态、物理单活、热切换或身份胶囊包含用户业务文件。Android 与正式安装宿主的系统生命周期不能由这些结果替代。


## R1 后防御性续接：资格尚未完整

M116 仍 In Progress。R1 的两项实质发现及跨端确认差异，分别通过备份副本隔离、持久停止保管状态及共享确认资格修复。未编译未运行的旧权限效果探测草稿从活动测试移除，原文和补丁保留于 `m116-high-20261024/platform-stop-source/` 及 `platform-stop-unreviewed.patch`，没有执行。原完整备份计时、R1 轮数及迟触发记录均不重置。

新图：Linux Native6 `ff2300e5d95bf4c365d742d4c1d03e5bc4b24528057f05a273f846109f52f31b`（465 Fresh／2 本项目构件，21.16 秒）；SDK 库 `f22d39e401123631cf32aa23c5104117f0149ed18bdf252ecd3c45f537dc29ba`；WASM6 包 `10e9b68d832bd43836f901c779687193aa217c6a922ab9ed86d5774aace2119e`（295 Fresh／1 本项目构件，16.03 秒及固定 bindgen）。源码图、原始日志及固定构件分别在 `m116-linux-devices-6`、`m116-wasm-devices-6`。各图仅因相关源码改变构建一次，旧图及通过项保留原来源。

普通公开状态测试 `m116-status-consumer-red-6` 复用 Native5，实际观察到已知备份直开错误显示 ready；首次沙箱 socket 禁止是独立环境失败，原样保留。Native6 上同一状态测试及原三项正常备份 SDK 回归共四项通过于 `m116-status-consumer-green-6`。这些操作仅打开／关闭节点、读取状态和普通自有文件生命周期，没有尝试隔离端的越权效果。`m116-state-consumer-green-6b` 七项域测试通过，覆盖停止同意、历史回执、序列化重开、新提议重新确认及普通 Leave 的同结果查询。首次七项运行中，原托管测试将无序成员列表当作有序比较而失败；原结果保留于 `m116-state-consumer-green-6`，修正为按身份排序后完整比较成员与角色，核心不变，未重编 SDK。

**UNPROVEN：**未执行旧凭证重送消费者或隔离端权限效果探测；源码与域／普通状态结果不能冒充这些运行效果的证明。新增 Windows/Linux 账本实机资格仍待真实用户许可，不能由本机、浏览器或 Windows 编译代替。独立 R2 复审及本次正常生命周期运行结果另以实际完成记录追加；此处没有提前声明整票完成。


`m116-custody-lifecycle-6` 的正常 Native→Browser、Browser→Native 两方向现已通过：未保存前停止待交付同意、普通存储重开后保持停止、旧提议本地重新接受拒绝；两次不同新提议各由双方明确确认并实际保存，再正常删除与重开。已完成提议的历史回执保留。两端普通 Leave 完成后，经确认入口查询同一签名结果，前后公开状态完全相同。此运行没有在停止后传送任何旧提议材料，也没有构造 raw Deliver 请求。Browser contexts／reader／进程组、所有 Native 宿主和唯一私有运行目录清理通过，`controllerSourceDrift=[]`；host-visible 自有进程清点为空。该观察证明普通状态与生命周期及新同意正例，仍不代替前述 UNPROVEN 运行效果。

Windows Native6 的本机编译为 526 Fresh／2 本项目构件，26.80 秒，现有 compiler 清理 exit0；固定 PE SHA256 `077d73c5f10bd42298d406d06abd4da00e8f363e14502eb2dd7428164bc24e97`。本轮只准备了 `m116-defensive-20261024/prepared-windows-ledgers-native6.md` 精确命令与新 owned temp，原 Native5 及自动审批拒绝原文保留；未执行或重提远端审批，授权仍 PENDING。


## R2 后同类备份计数修复与 Native7

R2 两轴共同发现同一个完整备份连续性缺口：新备份直开只重建隔离，未停止发行端私有邀请码计数；签名成员历史不能证明该计数未回滚。原两份报告保留于 `m116-defensive-20261024/review-r2/`。普通公开计数测试在 Native6 实际 RED：`m116-backup-counter-red-6`，仅本地读 `precode_usage_status`，没有尝试旧码兑换或权限效果。

Native7 的共享 Native 激活准备同时用于 `NativeNetwork::open`、`NativeClient` 宿主及新备份发布：先停止继承码，再持久保存绑定当前替换包的本机停止标记；旧无证据副本即便已有资格回执，也一次保守停止当时所有邀请码，保留签名资格。已有正确标记的已资格目录正常重开保留后来新码。标记参与原端退役清理。Linux7 465 Fresh／2 本项目构件、21.86 秒，binary `3e5375ce290964dbd42f1f7f8e3650a56c15a8e4d7f681f6b4970987bcab2837`，SDK `510a1b067671bcc0e6bb92678dde04583dbf632b3ad9898687af92a7d075446d`；Windows7 本机编译 526 Fresh／2 本项目构件、27.43 秒，PE `4b8449c3b6ab5389e90edad1761a387131e9fb7316f0e89101874d4bc37d8a45`，compilerCleanup0。WASM6 的全部 220 个选中输入未变，直接复用，没有重复构建。

`m116-backup-counter-green-7` 六项通过，35.15 秒：原四项备份断言保留，加上同一继承计数核心及正常资格／新码流程。原活动源仍信任其真实本地计数；新副本、显式恢复、再备份及旧 Native5／Native6 格式副本激活后均不再信任继承计数。两代旧夹具由各自固定 SDK 的普通公开备份接口产生，没有修改文件来伪造版本或资格；Native5 无 restore marker、Native6 有 marker 的前置已保留。正常当前成员核对后，旧计数仍不可信；完全新发码经过关闭／重开仍为 0，新独立成员正常加入后为 1。实际资格核对路径记录为普通 aps1 relay；两宿主均在本机，不冒称 Windows 或广域资格。

旧夹具生成器第一次在编译并固定第一程序后写 JSON 时遇到 Path 序列化错误；原 driver、原失败及空的原 compiler stderr 保留，第一程序复用已验 SHA／独立 store 的 pin，未重编；未发布的原 compile 秒数明确为 unknown，没有补造。修正收据代码后只编译第二个程序，两代夹具正常生成并经测试后按唯一 owner marker 清理，自有进程检查为空，SDK／源码／原失败均保留。

`--case all` 现包含两个 `custody-lifecycle` 方向。`selector-coverage-r2.json` 保留旧选择器缺项的静态结果、新完整选择列表及显式用例主体 AST 未变证明；显式用例的既有实际 Native6/WASM6 运行结果保持原来源，未为一行选择器修改重复运行。此项是选择器静态验证，不冒充新的运行结果。完整原类别计时与迟延持续累计，Native7／R3 不重置。

上述结果仍不能代替 **UNPROVEN** 的受限运行效果，也不能代替 **PENDING 用户授权** 的新增真实 Windows/Linux 账本验收；M116 仍未完成。
