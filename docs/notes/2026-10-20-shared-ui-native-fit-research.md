# 共享 UI 的原生端适配复核：桌面、Android 与节点生命周期

后续决定：用户已采纳本技术栈方向，见 [ADR 0018](../adr/0018-shared-vue-ui-with-tauri-shells.md)。下文保留采纳前的研究状态与证据边界，选型确认不代表运行验证完成。

日期：2026-10-20。状态：研究结论与有条件建议，不是框架决定、实施计划或新的平台承诺。本次只读核查现有源码、当前产品讨论及官方资料，未编译、运行、安装或修改现有实现。

## 结论

**建议有条件保留“共享 Web UI + Tauri 2 安装版外壳 + 原生 Rust 节点”。Windows/Linux 的外壳适配理由充分；Android 可以继续作为候选，但必须把原生 Service、应用 Context、文件授权与生命周期接线列入真实开发成本。Tauri 不能同时被当成后台节点运行框架。** 现有证据没有构成换掉 Vue 的理由：当前决定成败的限制主要在操作系统、节点宿主和数据流边界。

三个最容易误判的点：

- Android 10+ 普通应用没有后台读取其他应用剪贴板的普遍权限；前台服务不等于当前获得输入焦点。因此不能承诺桌面式、全天候、双向透明剪贴板自动同步。[Android 10 剪贴板限制](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data)
- Android 后台联网必须选对真实适用的运行机制。运行于 Android 15+、target Android 15+ 的 `dataSync` 前台服务在应用位于后台时有每 24 小时合计 6 小时的限制；重新打开前台会重置计时。该类型不能承担无限常驻节点。[前台服务超时](https://developer.android.com/develop/background-work/services/fgs/timeout)
- Tauri 官方插件覆盖的是不同程度的系统入口。Android 的 clipboard 仅支持纯文本，shell 仅能打开 URL，autostart 明确不支持 Android；“Tauri 支持 Android”不能推出桌面插件功能全部可用。[Clipboard](https://v2.tauri.app/plugin/clipboard/)、[Shell](https://v2.tauri.app/plugin/shell/)、[Autostart 源码平台表](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/autostart/Cargo.toml)

这里的“有条件”不是要求立即启动验证。若后续发现 Tauri 的 Activity/WebView 宿主使必要 Android Service 接线不可维护，可以替换 **Android 外壳**，保留共享 Vue 页面和 Rust 核心；这比因 Android 系统限制更换全部 UI 框架更有针对性。是否需要替换，目前缺少本项目集成证据。

## 评估依据与证据边界

按 [当前讨论记录](2026-10-03-shared-web-client-ui-direction.md) 末尾确认内容评估：首版是剪贴板共享、文件传递、端口转发、浏览器访问指定内网地址端口的服务、远程 shell 五个独立模块；平台为 Windows、Linux（含无界面）、Android、独立浏览器/PWA 节点。共用节点身份、成员资格和服务授权，不代表共用全部 OS 能力。管理员远程配置与节点本机配置修改同一套有效 ACL，由实际提供端执行。[CONTEXT](../../CONTEXT.md)、[ADR 0014](../adr/0014-browser-role-parity-with-contextual-reminders.md)、[ADR 0017](../adr/0017-guest-access-with-scoped-invitations.md)

官方资料使用 Tauri v2 文档；本次打开的 Rust API 文档显示 `tauri 2.12.4`。插件源码核查来自 `plugins-workspace` 的可变 `v2` 分支，反映检索日状态，不是已锁定的项目依赖；例如 autostart 清单当时版本为 `2.7.0`。IPC 细节另核对 `tauri-v2.12.4` 标签。Context7 用于定位官方文档与源码，再核对直接来源。没有将文档支持声明当成本项目性能或生命周期验收。[Tauri API 版本](https://docs.rs/tauri/latest/tauri/enum.RunEvent.html)、[固定标签 IPC 源码](https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/ipc/channel.rs)

当前源码边界也需要更新理解：[src/android_shell.rs](../../src/android_shell.rs) 已有 JNI 入口，保留 Java Context 的全局引用、安装 Android DNS Context 并启动 Tokio/现有 session；[MioPublicShell.java](../../scripts/android/MioPublicShell.java) 用 `app_process` 风格的前台工具入口取得系统 Context。它仍不是已安装的普通应用、Activity、Service 或正式 Tauri 接线。早期 [Android 说明](2026-10-03-android-management.md) 中“测试壳没有 Java Context”的描述不能盖过后来源码，但后来补上 Context 也不能证明应用沙箱、后台运行和应用销毁场景已经通过。桌面材料同样是 CLI 与 Wine 下运行的 Windows PE 证据，明确未完成原生 Windows 主机、安装器和 UI 验收。[桌面证据范围](2026-10-03-desktop-management.md)

## Tauri 能提供什么，需要另外提供什么

Tauri 的 Rust Core 调用系统并管理全局状态，WebView 渲染 HTML/CSS/JS；两者经 IPC 通信。这个边界适合“Rust 节点拥有业务与权限，Web 页面负责交互”。但 Core 进程仍是一个应用进程：多进程 WebView 架构本身不提供系统服务托管、进程被杀后的节点恢复，或无桌面依赖的 Linux daemon。[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)

| 首版模块 | Windows/Linux 安装版 | Android 安装版的真实边界 |
| --- | --- | --- |
| 剪贴板共享 | 系统剪贴板读写有官方插件；自动变更观察、桌面会话和 Linux 显示环境仍需模块实现。无界面 Linux 节点不能凭空具备用户桌面剪贴板。 | 官方插件仅纯文本；普通应用后台采集受系统硬限制。前台聚焦后读取、用户显式分享/粘贴可以保留，不能默认全时采集。 |
| 文件传递 | 选择文件后由 Rust 直接读写和传输，UI 接收进度。 | 支持系统文件选择与 `content://`，但必须处理 URI 授权、提供者和进程重启；不是桌面文件路径模型。 |
| 端口转发 | 数据面适合留在原生节点，窗口仅配置和显示状态。 | 原生 socket/节点方向可行，但持续提供服务取决于后台宿主；Tauri 不自动配置系统路由或 VPN。 |
| 浏览器访问服务 | 外壳可提供入口，但具体 HTTP/HTTPS/WebSocket、站点与来源兼容性由业务实现决定。 | 同样不能靠嵌入 WebView 自动变成任意内网站点兼容层；远端页面与节点管理界面必须保持可信权限边界。 |
| 远程 shell | UI 终端消费原生节点字节流；远端 tmux 与目标进程是另一层职责。 | 作为远程终端使用端不需要本机 shell spawn；不能将官方 shell 插件当作 Android PTY/tmux 提供器。 |

表中插件范围据 [Clipboard](https://v2.tauri.app/plugin/clipboard/)、[Dialog](https://v2.tauri.app/plugin/dialog/)、[FS](https://v2.tauri.app/plugin/file-system/)、[Shell](https://v2.tauri.app/plugin/shell/)；节点侧数据面与 UI 职责是针对本产品的工程推断，尚未实现。Windows/Android 自身作为 tmux 服务提供端的目标环境、交付方式和能力声明仍需模块设计，不能从终端使用端可行直接推导。

Linux 无界面交付应直接运行 Rust 节点宿主，不启动 Tauri/WebKitGTK；有界面版按需附着同一业务核心。是否让有界面桌面的节点与 UI 分处两个 OS 进程，可以按“关闭窗口后是否仍需提供服务”决定，不能为了分离而重复运行两个相同身份、同时占用同一数据目录的节点。Tauri 可以管理退出事件，但系统 daemon/service 的安装、启动、停止和本机授权通道仍属项目工作。[Tauri 退出事件 API](https://docs.rs/tauri/latest/tauri/enum.RunEvent.html)、[Linux 图形依赖](https://v2.tauri.app/start/prerequisites/#linux)

## Android：可做原生接线，不能绕过系统语义

### 前台服务必须按用途选择

Android 官方规定在 Android 14+ 上，target Android 14+ 的应用必须声明适当的前台服务类型与对应权限；Android 12+ 上 target Android 12+ 在后台启动前台服务也受限制，只有列出的例外适用。用户开启后持续执行某项可见任务，与进程已不存在时收到任意 peer 数据便自行启动服务，是不同前提。[服务类型](https://developer.android.com/develop/background-work/services/fgs/service-types)、[后台启动限制](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)

| 类型 | 对本产品的判断 |
| --- | --- |
| `dataSync` | 上传/下载等有限任务可能匹配；上述 6 小时限制使它不适合所有节点维护统一常驻。target 15+ 还不能由 `BOOT_COMPLETED` 启动该类型。 |
| `connectedDevice` | 官方范围含通过网络与外部设备交互，因而值得核实是否匹配用户明确开启的设备互联；有规定的权限/API 前置条件。不能仅因拥有网络连接就宣称整个产品常驻均已获准。 |
| `remoteMessaging` | 描述是用户在设备间延续文本消息任务，不能仅凭 clipboard 也是文字、shell 也是文字就套用。 |
| `specialUse` | 覆盖其他有效前台服务用途，但需说明 subtype；Google Play 会审查用途。不是一个无需论证的通用后台豁免。 |

类型表依据 [Android 官方服务类型定义](https://developer.android.com/develop/background-work/services/fgs/service-types) 和 [超时定义](https://developer.android.com/develop/background-work/services/fgs/timeout)。这是候选适用性分析，未决定选用哪类，也未假定 Play 分发路线已确定。`dataSync` 的限制不证明所有后台设备联网均不可行；真实 peer networking 可能适用 `connectedDevice`，仍需核对实际用途、权限和目标系统行为。

Doze 会限制网络与 CPU，普通 wakelock 不构成无限联网保证；前台服务不应被描述成所有省电与 OEM 策略的统一豁免。Android 13+ 用户还可以从系统 Active apps 停止整个前台服务应用，系统不会给应用一个确保保存状态的回调。因此节点必须保留持久身份，恢复后重新判断连接和实际可提供能力，不能把“有前台通知”显示成永久在线。[Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)、[用户停止 FGS 应用](https://developer.android.com/develop/background-work/services/fgs/handle-user-stopping)

这与产品“不必每对节点常驻连接”的取向兼容：Android 可以采用不同维护预算、有限活跃服务和恢复机制；但“让限制留在软件内部”仍须诚实呈现可用/暂停状态。不能暗中引入不可替换的云推送依赖来弥补后台限制。

### Service/JNI 的所有权需独立于 Activity

官方移动插件允许 Kotlin/Java 与 Rust 互调，提供 Activity 生命周期回调；这证明原生桥接可行，不证明已有全后台节点模板。当前插件常持有 Activity，例如 clipboard 和文件插件。建议由应用/Service 持有 Rust 节点运行实例与 application Context，Activity/Tauri Adapter 只附着、发命令和订阅当前状态，重建界面时不另造节点。[移动插件开发](https://v2.tauri.app/develop/plugins/develop-mobile/)、[插件生命周期源码](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/mobile/android/src/main/java/app/tauri/plugin/Plugin.kt)

Service 默认也运行在应用主线程，不能把 JNI 阻塞式 session 直接搬进 `onStartCommand`。需要由受控的原生运行时/工作线程承接，并明确停止与重新附着；是否配置独立 Service 进程要按故障隔离需求决定，不能假定“Service 就是第二个进程”。[Android Services](https://developer.android.com/develop/background-work/services)

上述是可实现但尚未验证的原生集成工作。若 Tauri runtime 自身依赖 Activity 的生命周期，使 service-only 启动必须构造 UI，最应调整的是宿主边界，而非节点身份、成员角色或共享页面。

### 剪贴板是硬边界，不是补一个插件即解决

Android 10+ 的允许条件是默认输入法或当前获得焦点的应用，官方没有把 FGS 列为同等条件。研究结论只针对普通安装、普通应用权限；不把 root、默认输入法、系统签名或 ADB 授权作为首版默认前提。仍可研究前台读取和用户显式分享的简单流程，但这是能力与交互范围选择，需要产品确认，不能由研究默认为已决定。[系统限制](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data)

官方 clipboard Android 实现直接调用 `ClipboardManager`，没有独立后台观察流；它不替产品处理组/双机范围、回环抑制或 ACL。也应区分“远端内容已收到”“写入本机剪贴板成功”“本机剪贴板变化已读出”三种事实，而不是给所有平台一个无条件的 `clipboardSync=true`。[插件 Android 源码](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/clipboard-manager/android/src/main/java/ClipboardPlugin.kt)

## 文件、终端和 IPC：统一操作语义，保留不同数据入口

### 大文件可以留在 Rust 侧

Tauri dialog 在 Android 返回 `content://`；FS 插件 Rust 侧的 `Fs::open` 通过 Kotlin `ContentResolver` 取得文件描述符，再构造成 `std::fs::File`。因此“Android 必须把整个文件读进 JavaScript 再传回 Rust”并不成立。普通 `std::fs::File::open("content://...")` 也不是正确替代。[Dialog URI 说明](https://v2.tauri.app/plugin/dialog/#usage)、[FS Android Rust 实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/fs/src/android.rs)、[FS Kotlin 实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/fs/android/src/main/java/FsPlugin.kt)

但当前官方 `v2` 源码的打开选择器使用 `ACTION_GET_CONTENT`，旁边仍留有 `ACTION_OPEN_DOCUMENT` 的 TODO；没有取持久 URI 授权。因此不能从“选择并传完一次文件”推断“进程重启、手机重启后可直接恢复同一传输”。需要恢复时应明确采用何种授权入口，或在用户许可范围内暂存文件；暂存增加磁盘占用，不是免费方案。[Dialog Android 实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/dialog/android/src/main/java/DialogPlugin.kt)

Android 官方 SAF 支持可持久化授权，但文档移动/删除后授权仍会失效，提供者也可能返回未知大小、虚拟或不同能力的文件。统一文件传递 Interface 宜表达可读/可写流、显示名称、可选长度和可恢复性，不要求所有端都提供永久文件路径。后台 Service 若不依赖 Activity 工作，也需通过 application Context 接入相应授权文件源，而不是假定前台插件对象永远可用。[Android 文档与授权](https://developer.android.com/training/data-storage/shared/documents-files)

### IPC 适合控制和必需的 UI 字节流

Tauri 普通 command 的序列化结果走 JSON；原始二进制响应可用 `tauri::ipc::Response`，持续更新可用 Channel。将文件本体、转发 socket 流量都绕经 UI 并非必要；原生节点直接传输，UI 只收到操作句柄、进度与结果。终端则确实需要把输出字节交给终端渲染器，适合分块、批量、有界传递。[调用 Rust 与二进制响应](https://v2.tauri.app/develop/calling-rust/)

一个重要细节：普通 `Channel<Vec<u8>>` 或 `Channel<&[u8]>` 的序列化类型仍可成为 JSON 数字数组；需要原始二进制时应明确使用 `Channel<InvokeResponseBody>` 的 `Raw`，或 `Response`，不能只因用了 Channel 就宣称零拷贝。固定 `tauri-v2.12.4` 源码还包含小 Raw 消息转 JS 数组及较大消息排队后由前端取出的分支，`send` 成功也不等于终端已经消费。项目仍需限制未消费数据、提供取消/停止订阅，并决定 UI 暂停期间终端输出如何限量保留。[固定版本 Channel 实现](https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/ipc/channel.rs)

远端 tmux 保持的是远端进程/工作区；Android UI 被回收或本地网络断开以后重新附着，不能依靠旧 WebView Channel 作为会话身份。浏览器和原生 Adapter 可以共用“打开/附着终端、输入、resize、关闭连接”等语义，实际数据桥不同；这些建议不决定尚未讨论的离线输入重放或 tmux 安装方案。

## 系统 WebView 与资源消耗的含义

Tauri Windows 使用 WebView2、Linux 使用 WebKitGTK、Android 使用系统 WebView。Android provider 版本可独立于应用更新，Linux 的引擎和系统依赖也不同，所以 Vite 输出能打包不等于 CSS、终端渲染和所有浏览器 API 已在目标系统同样可用。[WebView 版本与平台](https://v2.tauri.app/reference/webview-versions/)

不随应用捆绑完整引擎可以减少分发体积，但引擎仍在运行并占内存；Tauri 的多进程模型也没有给出本项目的总体内存预算。不能用安装包大小或“Rust 后端”证明运行时低内存。后续若获准做最小验证，只需比较同负载的纯节点与节点加 UI，并计入 WebView/renderer 相关进程，观察空闲和终端活跃时是否满足约定预算；当前没有运行此验证。[进程模型与系统引擎](https://v2.tauri.app/concept/process-model/)

Android 官方明确 renderer 可能因内存或崩溃而消失，相关 WebView 需销毁并重建；Activity/WebView 也可在配置变化或资源回收时销毁。因此网络状态、文件传输任务和 ACL 生效不能依赖 Vue 页面是否存在；恢复 UI 应重新获取节点快照与新订阅。[WebView renderer 与生命周期](https://developer.android.com/develop/ui/views/layout/webapps/managing-webview)

对于“浏览器访问服务”模块，远端网页是内容，节点管理页是可信界面。Tauri 的远端 API 默认关闭，但官方特别说明 Linux/Android 无法区分嵌入 iframe 与宿主窗口发来的 IPC 请求；不能建议在有节点权限的管理 WebView 内嵌任意远端网页，再只靠 URL 或 iframe 隔离。应采用经过验证的无权限内容容器/外部浏览器等边界；具体浏览器访问方案仍待模块研究。自定义应用命令默认向应用窗口开放，也需通过 `AppManifest::commands` 等机制显式控制。Tauri IPC permissions 和节点网络 ACL 是两道不同边界，不能互相替代。[Tauri Capabilities](https://v2.tauri.app/security/capabilities/)

## 最小决策问题

恢复 UI 工作前，以下四项足以判定是否应继续当前方向，不需要并行比较多个完整 UI 框架：

1. **Android 的实际能力承诺是什么？** 是否接受普通权限下的前台/显式剪贴板操作，以及节点因系统策略暂停后恢复？若要求普通 Android 全天透明采集其他应用剪贴板，需求与 OS 冲突，换壳不能解决。
2. **一个 Service 所有的 Rust 节点能否与 Tauri UI 独立启动、重建和重新附着？** 只需证明关闭/重建 Activity 时不会生成第二个节点，service-only 恢复不需要活着的 WebView，且真实用途对应的 FGS 类别与退出行为成立。若 Tauri 在此处造成不可接受耦合，替换 Android 宿主层。
3. **一个真实文件源与一个终端流能否保持有界资源？** 文件在 Rust/授权文件源侧传输，界面重建不丢失任务事实；终端快速输出时原始二进制通路与消费反馈不会形成无界队列。失败先调整桥接和所有权，不由此泛化为 Vue 性能不足。
4. **实际目标设备的 WebView 总体开销是否落在用户接受范围？** 先约定最低设备与粗粒度预算，再用上述一个业务流决定；目前没有足以批准或淘汰 Tauri 的本项目测量。

这些是后续如需决策时的验证问题，不是本次已授权的实验安排。当前最稳妥的研究建议是：保留共享 UI 和原生/浏览器两套节点 Adapter，Tauri 桌面继续作为首选候选；Android 外壳有条件保留，平台服务层单独承担原生接线。所有端继续按实际能力声明服务，并遵守相同角色与 ACL，不用 UI 框架为能力差异制造权限等级。
