# 共享 UI 技术栈适配核查汇总

日期：2026-10-20。状态：研究结论。用户在研究及后续讨论后已确认采用 Vue 3 + TypeScript + Vite 与 Tauri 2，见 [ADR 0018](../adr/0018-shared-vue-ui-with-tauri-shells.md)，以复用及满足需求为验收依据；下文保留研究时的建议与待验证边界。本轮按用户要求，由三个 GPT-6.1 Sol、high reasoning 子代理分别核查架构、原生平台、浏览器能力，主代理交叉核对源码与关键官方资料。本轮没有构建原型、安装依赖、运行性能或生命周期实验。

## 结论

**推荐继续以 Vue 3 + TypeScript + Vite 共享界面、Tauri 2 安装版外壳为方向。现有证据支持这种分工；尚不足以认定五个业务模块在各端已经完全满足要求。** Windows/Linux 外壳适配依据充分，Android 需要应用／Service 的原生接线；浏览器访问内网页面是当前最需要验证的独立业务问题。没有发现必须更换 Vue 的依据，也没有本项目实测可以证明它是资源消耗最低的选择。

| 选择 | 本次判断 | 依据及限制 |
| --- | --- | --- |
| Vue 3 + TypeScript + Vite | 推荐保留 | Vue 官方工具链与 Tauri 推荐的 SPA 接入相符；适合现有设备、审批、任务及设置交互。不等于性能已验收。 |
| Windows/Linux 的 Tauri 2 外壳 | 推荐作为首选候选 | WebView 展示、Rust 核心和系统入口的分工适合现有节点；实际 Windows/Linux 发布包尚未验证。 |
| Android 的 Tauri 2 外壳 | 有条件保留 | 可用 Kotlin/Java 接入系统；后台节点、文件授权、Activity 重建需专门处理。若宿主接线不合适，优先调整 Android 外壳而保留共享页面。 |
| 独立浏览器/PWA 节点 | 保留现有方向 | 已有 Rust/WASM 节点与持久身份接线；普通网页系统接口和生命周期限制仍存在。 |
| Linux 无界面节点 | 直接使用原生节点库 | 不应为共享业务而启动隐藏 WebView；具体宿主与交付尚待实现。 |

组合的官方依据见 [Vue TypeScript 工具链](https://vuejs.org/guide/typescript/overview.html)、[Tauri 前端配置](https://v2.tauri.app/start/frontend/)、[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)、[移动插件接线](https://v2.tauri.app/develop/plugins/develop-mobile/)。上表推荐是针对本项目约束的工程判断，不是这些文档对 miopunch 的背书。

## 最符合 KISS 的工程实践

建议共享页面、交互和业务结果语义，在启动入口选择两个节点接入实现：

```text
共享 Vue 界面
  ├─ 安装版接入 → Tauri IPC → 原生 Rust 节点／业务
  └─ 浏览器接入 → WASM     → 本页 Rust/WASM 节点／业务

Linux 无界面宿主 → 原生 Rust 节点／业务
Android 应用／Service → 原生 Rust 节点／业务，界面按需附着
```

安装版不同时再启动一份浏览器节点；浏览器仍拥有自己的节点身份。节点、授权和长任务不由某个 Vue 页面持有，切换页面只改变视图与订阅。这里表示职责，不要求现在拆出固定数量的 crate、进程或仓库。

用户后续已确认安装版生命周期：Windows／Linux 桌面关闭窗口继续后台运行，Android 切到后台或锁屏继续联网，明确退出应用时才停止本机节点。宿主需承接这一行为，UI 重开附着现有节点；具体进程结构仍由实现决定。

Windows／Linux 桌面还已确认默认在用户登录系统后自动启动并联网，可在设置关闭；自启时仅在后台运行，不弹主窗口，用户需要时再打开界面。Android 另已确认提供手机重启后自动恢复联网的机制，默认开关尚未指定；其系统接线及 Linux 无界面宿主的系统服务接线仍需实现设计。

浏览器交付已确认提供默认访问地址，并支持自托管；两种入口都运行独立浏览器节点。具体域名和托管方式尚未选择，此决定不增加必需的中心控制或业务中转服务器。

实际可借鉴的案例是 Hoppscotch：共享 Vue 应用定义平台接口，由 Web／desktop 入口装配平台实现，桌面包接入 Tauri 2。它证明这类复用有真实工程实例，不能证明本项目的 Android、P2P 或五模块能力。[平台接口](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-common/src/platform/index.ts)、[Web 入口](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-selfhost-web/src/main.ts)、[桌面依赖](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-desktop/src-tauri/Cargo.toml)

具体建议收敛为五条：

1. **五个业务保持清楚的模块接口。** 先用静态模块；不增加动态插件运行时、通用 RPC 总线或多层原样转发。模块内部处理连接与授权，页面表达用户意图。
2. **平台差异集中在接入层。** 文件选择、剪贴板、落盘和生命周期通过平台实现接入；页面按真实能力呈现操作，不散布平台判断，也不增加浏览器专属角色等级。
3. **正式契约与探针分开。** 当前 JSON 状态与 64 KiB 消息交换提供了真实节点接线，仍需补业务类型、错误、取消、订阅和完成条件。两端类型应来自一致契约，避免三处手写漂移。[现有 BrowserNetwork](../../browser-probe/src/network.rs)
4. **大数据采用有界流。** 原生文件和端口流量直接由 Rust 处理，UI 接进度；终端输出必须送到渲染器，但需分块及消费反馈。不要把整文件或无限终端历史放入 Vue 状态。Tauri Channel 也不自动证明零拷贝或背压。[Tauri 通信](https://v2.tauri.app/develop/calling-frontend/)、[xterm.js 流控](https://xtermjs.org/docs/guides/flowcontrol/)
5. **按需加载并测量完整应用。** 浏览器入口加载 WASM，安装版加载 Tauri 接入，重型终端界面按需加载。安装包小不能证明低运行内存；应比较同负载的纯节点与节点加 UI，并计入相关 WebView 进程。[Vue 性能实践](https://vuejs.org/guide/best-practices/performance.html)、[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)

这些是研究建议；没有选择组件库、类型生成器、最低系统版本或进程部署方式。

## 五个模块能否满足

下面评估的是实现方向与已知边界，不是已交付能力表。

| 首版模块 | 适配判断 | 决定性边界 |
| --- | --- | --- |
| 一对一剪贴板同步 | 用户已确认一对一、开启后自动同步；共享 UI 合适 | Android 10+ 普通后台应用不能持续读取系统剪贴板；普通浏览器也受焦点、激活与许可限制。平台受限情形仍需适配。 |
| 文件传递 | 方向可行 | Android 使用授权文件 URI，浏览器选择与保存接口不同；大文件要验证低内存落盘及保存完成语义。 |
| TCP／UDP 端口转发 | 用户已确认两种协议都支持；原生数据面合适，管理界面可共享 | 普通浏览器没有通用本地 TCP／UDP socket 能力，不能作为相同的原生转发提供端。 |
| 浏览器访问内网地址端口的网页 | 核心接入尚未证明 | P2P stream 不接管地址栏、资源请求或 WebSocket；需要独立的网页承载方案与站点兼容验证。 |
| 远程 shell／tmux 多会话 | 用户已确认多会话、关闭标签页保留会话，Windows 提供端采用 WSL + tmux，Android 仅作为终端客户端；终端使用界面适合共享 | 远端宿主提供 PTY/tmux；本地断线后重新附着，不能依赖旧 QUIC 连接或 WebView 生命周期保留工作区。 |

能力依据详见[浏览器报告](2026-10-20-shared-ui-browser-fit-research.md)和[原生报告](2026-10-20-shared-ui-native-fit-research.md)。权限资格与平台能力继续分开，沿用 [ADR 0014](../adr/0014-browser-role-parity-with-contextual-reminders.md) 和 [ADR 0017](../adr/0017-guest-access-with-scoped-invitations.md)。

### Android：后台节点与后台剪贴板是两个问题

Android 官方明确，Android 10+ 读取剪贴板要求应用当前有焦点或成为默认输入法，前台服务不构成普遍豁免。因此不能把桌面式自动采集承诺移植到普通 Android 应用。[官方剪贴板限制](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data)

节点后台联网则需按真实用途选择运行机制。`dataSync` 的时限不能证明所有设备互联均不可行；`connectedDevice` 官方范围包含通过网络与外部设备交互，但有适用条件。应由应用／Service 管理原生节点、持久身份和恢复，验证界面销毁／重建不会重复启动同一节点。[前台服务类型](https://developer.android.com/develop/background-work/services/fgs/service-types)、[超时限制](https://developer.android.com/develop/background-work/services/fgs/timeout)

用户后续已确认产品要求：Android 切到后台或锁屏后继续联网，明确退出应用时停止本机运行；具体后台宿主和平台生命周期仍由适配实现并验证。此要求不等同于后台剪贴板采集许可，也不改变系统强制停止等实际边界。

当前 [Android JNI 接线](../../src/android_shell.rs) 已有 Java Context，并非“完全没有原生桥”；但 [前台 shell 入口](../../scripts/android/MioPublicShell.java) 不是正式 APK/Service 的验证。本次没有将既有 Android 探针成功外推为后台产品已成立。

### 内网页面：由所选节点接入其可达服务

HTTP 数据经节点流往返是合理实现方向；登录、重定向、相对／绝对资源地址、cookie 和 WebSocket 全部正常，需要额外证据。Service Worker 能处理部分 Fetch，不是任意 origin 的网络替换器；标准 WebSocket 握手绕过 Service Worker。框架选型无法代替这部分设计。[Service Worker 注册范围](https://w3c.github.io/ServiceWorker/#register-algorithm)、[WebSocket 握手](https://websockets.spec.whatwg.org/#opening-handshake)

远端网站还应与持有节点权限的管理界面分离。Tauri 官方明确提醒 Linux/Android 无法区分嵌入 iframe 和宿主窗口的 IPC 请求；不能直接在有权限的主 WebView 嵌入任意网站并假定已隔离。普通浏览器也不能把远端网页放在保存节点密钥的同一 origin 下执行。[Tauri capabilities](https://v2.tauri.app/security/capabilities/)、[HTML origin](https://html.spec.whatwg.org/multipage/browsers.html#origin)

这里的具体限制是 **JavaScript 调用原生 Rust 的桥接来源识别**。Wry 的 `with_ipc_handler` 文档明确写出，Linux/Android 收到 iframe 的 IPC 时使用主 frame 的 URL；Android 当前源码也将 WebView 记录的当前 URL 随消息交给 Rust。因此若管理页内嵌远端网站，仅凭主页面来源和窗口权限不足以区分两者的调用。这不表示网页同源策略整体失效，也不表示加载任何 iframe 必然取得全部原生权限；实际可达性取决于桥接暴露、命令校验与配置。[Wry IPC API](https://docs.rs/wry/latest/wry/struct.WebViewBuilder.html#method.with_ipc_handler)、[Android 桥接源码](https://github.com/tauri-apps/wry/blob/dev/src/android/kotlin/Ipc.kt)

工程建议是让管理界面保留必要管理接口，远端网站在不授予节点管理 IPC 的独立内容视图中运行；独立视图是否共享 origin／存储还需明确。单纯新建窗口也不够，自定义命令与共享状态不能仍全局开放。此建议不要求一定拆成不同应用或 OS 进程，具体容器尚未选择。

**研究后用户已确认：内网页面必须留在 miopunch 应用内。** 因而原先提出的交给系统浏览器打开不再作为交付方案；安装版与浏览器／PWA 应分别实现应用内承载和隔离。该要求已记入[当前 UI 决定](2026-10-03-shared-web-client-ui-direction.md#内网页面必须在-miopunch-应用内打开)，不代表任意网站兼容性或具体框架已经通过验证。

这会影响网页模块采用隔离入口、代理或安装版本地访问等哪种方式。所有路径仍是候选，没有新增必需中心网关，也没有把普通浏览器改成远端节点的管理附属品。

用户随后明确：目标就是所选节点本身能访问的内网服务，不需要先向用户索取具体网站名称。需求按通用服务访问表达，站点及协议兼容性由实现内部验证。节点可达目标与浏览器完成页面呈现仍是不同集成环节，这项需求确认不替代运行证据。

## 后续验证范围

研究已足以支持继续讨论这套栈。后续进入验证时，由实现选取有代表性的服务与协议行为，覆盖应用内页面的导航、登录、资源、实时连接及隔离；不把用户先指定一个产品作为前提，也不把测试样本变成按网站品牌设置的产品白名单。

其他决定性证据包括 Android 真实应用的节点生命周期，以及文件／终端的有界数据流和 UI 成本。既有 SDK 证据继续按其原范围成立，本次不能据文档把这些项目标为通过，也没有启动实验。

分项报告：

- [架构、共享接口与工程实践](2026-10-20-shared-ui-architecture-fit-research.md)
- [原生平台与 Android 生命周期](2026-10-20-shared-ui-native-fit-research.md)
- [浏览器五模块与内网页面](2026-10-20-shared-ui-browser-fit-research.md)
