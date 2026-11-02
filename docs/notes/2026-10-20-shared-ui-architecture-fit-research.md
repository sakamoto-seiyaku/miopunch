# 共享 UI 与 Rust 节点分工：架构适配复核

后续决定：用户已采纳本技术栈方向，见 [ADR 0018](../adr/0018-shared-vue-ui-with-tauri-shells.md)，并将剪贴板收敛为一对一、开启后自动同步。下文保留采纳前的研究状态与证据边界，分组剪贴板不再属于当前范围，选型确认不代表运行验证完成。

日期：2026-10-20。状态：研究建议，**Vue 3 + TypeScript + Vite、安装版 Tauri 2 仍是提案**。本报告不接受框架决定、不选择组件库或代码生成器，也不启动 UI、构建、测试、依赖安装或源码重构。核对对象是 Windows、Linux（含无界面节点）、Android，以及作为独立节点的浏览器／PWA；首版五个业务模块沿用[当前讨论记录](2026-10-03-shared-web-client-ui-direction.md)。

资料方法：先读当前领域与源码，再通过 Context7 核对 Vue／Tauri 官方文档，并直接检查官方站点、官方文档源码和产品自有源码。下文区分“已有源码／证据”“上游机制”“工程建议”；上游支持某机制不代表 miopunch 已完成其集成。

## 结论与证据强度

**建议保留现有栈提案，优先把接入 Interface、平台能力和节点生命周期说清楚。** Vue 能承担设备、审批、设置及五个业务模块的交互；Tauri 能给安装版提供系统集成与 Rust IPC；浏览器仍运行自己的 Rust/WASM 节点。当前没有证据表明更换 UI 框架能解决 Android 后台运行、浏览器系统接口限制或内网页面的透明兼容问题。该判断来自下面的官方机制和现有源码，不是多框架性能实测。

| 核对点 | 证据与判断 | 证据不能证明什么 |
| --- | --- | --- |
| Vue + TS + Vite 的常规工程路径 | Vue 官方脚手架直接支持此组合；Tauri 推荐 Vite 接入 Vue 等 SPA 框架。证据强。[Vue TS](https://vuejs.org/guide/typescript/overview.html)、[Tauri 前端配置](https://v2.tauri.app/start/frontend/) | 不证明任何平台的 miopunch 内存、启动速度合格。 |
| 同一 Vue 应用复用于 Web／Tauri 桌面 | Hoppscotch 自有源码有共享包、平台定义、按 Web／desktop 注入配置的入口，桌面包使用 Tauri 2。证据强。[入口](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-selfhost-web/src/main.ts)、[平台定义](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-common/src/platform/index.ts)、[桌面依赖](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-desktop/src-tauri/Cargo.toml) | 不是 miopunch Android、独立 WASM 节点或五模块兼容性的证明。 |
| Native／WASM 已有可复用节点逻辑 | 根库与 browser-probe 编译共享验证／控制模块；两端各有节点入口与存储接线。源码证据强。[根库](../../src/lib.rs)、[浏览器入口](../../browser-probe/src/lib.rs)、[BrowserNetwork](../../browser-probe/src/network.rs) | 不表示正式 UI Interface、五个业务协议和安装包已经存在。 |
| 安装版接入现有 Android 节点 | 目前文档明确是前台 ADB shell 可执行程序。[实际边界](2026-10-03-android-management.md#actual-capability-boundary) | 不是 APK、应用私有存储、Service、后台存活或系统权限的验证。 |
| 低维护与低资源 | 共享源码和集中 Adapter 能减少重复维护，这是结构上的推断。 | 没有 miopunch UI 的资源数据；不能报告节省百分比、MB 或耗电改善。 |

## 1. 为什么目前继续选 Vue，而不再开框架竞赛

Vue 官方提供 TypeScript 类型声明、Single File Component 与 `<script setup lang="ts">`，官方 `create-vue` 使用 Vite；需单独运行 `vue-tsc`，因为 Vite 转译本身不做类型检查。对未来实施的建议是沿官方默认工具链，严格检查类型，让页面使用少量可复用 composable；无需为了共享 UI 再增加 SSR 服务器或框架层。[Vue 官方 TypeScript 指南](https://vuejs.org/guide/typescript/overview.html)

这符合当前设备管理、表单和状态交互的形态，也符合“一个框架长期维护”的已确认偏好；它是工程适配判断，不能归纳为 Vue 在所有指标上优于其他框架。[当前已确认取向](2026-10-03-shared-web-client-ui-direction.md#已确认的取向)

Svelte 是真实可行的替代：GitButler 自有开发说明明确采用 Svelte／TypeScript 展示、Rust 处理磁盘操作，并有独立 CLI 开发路径。它证明 Tauri 架构并不依赖 Vue；没有提供足以要求 miopunch 改选 Svelte 的本项目证据。更换为 Rust UI 也不会消除平台 SDK、权限和 WebView 生命周期工作；用户已经不要求 UI 全用 Rust，因此本次不建议为语言统一增加另一轮迁移。[GitButler 开发说明](https://github.com/gitbutlerapp/gitbutler/blob/master/DEVELOPMENT.md)、[当前框架讨论](2026-10-03-shared-web-client-ui-direction.md#当前框架建议及依据)

Hoppscotch 值得借鉴的是 **“共享页面，由启动入口装配平台实现”**。其 `PlatformDef` 是明确类型，入口通过 `PLATFORM_CONFIG` 选择实现，再调用共享 `createHoppApp`。不应照搬其全部依赖注入、interceptor、动态应用装载或 feature flag 体系；其公共包也仍依赖若干 Tauri 包，所以这个实例不证明天然实现了完美平台隔离或最小 Web bundle。[共享入口](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-selfhost-web/src/main.ts)、[PlatformDef](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-common/src/platform/index.ts)、[公共包依赖](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-common/package.json)

## 2. 可共享的是源码与业务语义，不是所有运行环境

当前根包名为 `miopunch-native-probe`，库产物是 `rlib`，依赖原生 Tokio、文件系统和 SQLite 等；浏览器包通过 `wasm_bindgen` 暴露入口，并用 `#[path]` 复用部分 Rust 源文件。这已经支持“公共语义 + 平台 Adapter”的方向，但不能把根原生库不加区分地编成浏览器库。[Cargo 清单](../../Cargo.toml)、[根库模块](../../src/lib.rs)、[浏览器模块与导出](../../browser-probe/src/lib.rs)

建议依赖方向如下。它表示职责，不要求现在建立这些目录、crate 或一个独立进程：

```text
共享 Vue 页面、交互流程、视图模型
  → 各业务 Module 的 UI Interface
    → 安装版 Adapter → Tauri IPC → 原生 Rust 节点与业务 Module
    → 浏览器 Adapter → WASM 调用 → 本页 Rust/WASM 节点与业务 Module

原生 Tauri 外壳／CLI／Linux 无界面宿主／Android 宿主
  → 原生节点库

节点与业务库不反向依赖 Vue、路由、DOM、Tauri 窗口或 UI store。
```

安装版 WebView 只接原生节点，不同时启动第二份浏览器 WASM 节点；浏览器 Adapter 不把管理命令转发给远端原生节点替自己执行。这样保持[独立浏览器节点的现有语义](../../CONTEXT.md)与[浏览器权限平等决定](../adr/0014-browser-role-parity-with-contextual-reminders.md)。原生 crate 的平台化整理应尊重[现有库契约与默认行为](../adr/0012-preserve-library-contracts-and-defaults.md)，不为 UI 改写 vendor 库的默认策略。

| 可共享的部分 | 必须保留差异的部分 |
| --- | --- |
| 页面、文案、导航、设备与任务视图、来宾受邀入口 | 桌面窗口／托盘、Android Activity／Service、浏览器页面生命周期 |
| 输入校验、等待／完成／失败交互、视图状态转换 | 文件选择／落盘、剪贴板读取／监听、系统通知与权限请求 |
| Rust 身份、成员／授权验证及适合跨平台的业务协议 | SQLite／IndexedDB、原生 socket／浏览器可用传输、平台密钥保管 |
| 同一业务结果、错误类别和取消语义 | 不同数据流句柄、IPC／WASM 封送和真实能力限制 |

这不承诺全部业务 Rust 源码都能原封不动共享；将系统访问放进 Adapter，是为了让公共 Module 的调用者不必重新理解平台细节。Linux 无界面运行应能使用节点与业务库而不初始化 WebView；不需要为了复用而运行隐藏浏览器。[现有原生独立库入口](../../src/lib.rs)、[原生节点](../../src/native_network.rs)

## 3. 五个业务 Module 应有真实 Interface，不先造动态插件平台

“像插件”可以先采用静态链接的 Rust Module／crate 与前端业务包。五个模块已有不同职责，无需动态加载、独立进程、插件市场、运行时权限 manifest 或通用 RPC bus 才能实现模块化。先保留一个仓库和简单包结构；仅在跨平台依赖、所有权或复用确有收益时拆 crate。这是本次 KISS 建议，不是新增产品决定。[模块化讨论](2026-10-03-shared-web-client-ui-direction.md)

设计建议采用 small Interface / deep Module：Interface 除类型还要描述完成条件、错误、顺序、取消和资源所有权；Module 在内部完成建连、验证和平台适配。真实变化已经存在于 Native／Browser 两个 Adapter，无需再添加暂时没有第二种实现的抽象。[本次采用的 codebase-design 定义](/home/js/.agents/skills/codebase-design/SKILL.md)

| 首版 Module | 对共享 UI 应表达的意图 | 应隐藏在 Rust／平台 Adapter 内的工作 |
| --- | --- | --- |
| 指定组／双机剪贴板 | 选择同步对象，开启／暂停，显示本端采集或粘贴能力 | 平台采集与写入、回环抑制、身份与范围核验；不因成员资格自动全网广播 |
| 文件传递 | 选文件、选目标、查看任务和结果、取消 | 数据流、校验、保存结果、失败处理；原生路径与浏览器文件句柄不能假装相同 |
| 端口转发 | 使用节点声明的转发服务，查看可用状态 | 实际监听／拨号、范围检查、字节流与关闭；UI 不编排打洞 |
| 浏览器内网 Web 访问 | 选择目标服务并进入其网页 | 内网地址连接、HTTP／WebSocket 等承载、源与内容隔离；不能仅复用一个输入框就声称兼容任意站点 |
| 远程终端／tmux | 选择设备、打开／重接会话、输入和显示终端 | 远端 PTY 与 tmux 接线、授权、流控与会话标识；网络失败不能当成 tmux 会话已退出 |

本表是建议的职责分配，不冻结方法名、服务实例参数或每平台支持范围。首版范围和剪贴板／tmux 语义取自[当前讨论](2026-10-03-shared-web-client-ui-direction.md)；具体实现仍未建立。

避免把所有方法不断堆进一个巨大的 `NodeClient`。可以让应用组合一个节点管理 Interface 和五个业务 Interface；各业务只取得自己需要的连接、存储和授权能力。也不必给每条命令额外套“路由 → action → manager → service → repository”链条；若一层只原样转发，尚未封装实际差异，就没有维护收益。这是设计建议，并非现有目录的改造任务。

能力发现至少区分 **平台支持、当前开启、当前有效授权、当前可达**；提供端和使用端也应分开表达。例如终端使用端能显示并输入，不表示本机能提供 tmux。UI 可用能力解释按钮或入口，执行时仍由服务提供节点重新检查。无需为此增加一个通用能力推理引擎；沿现有“节点 → 服务”声明、统一有效 ACL 和必要状态即可。[CONTEXT 术语](../../CONTEXT.md)、[服务与来宾决定](../adr/0017-guest-access-with-scoped-invitations.md)

Tauri 的 capabilities 控制哪些 WebView 能调用本机命令／插件，与 miopunch 网络成员能访问哪些远端服务是两套不同对象，不能互相替代。实际网络服务 ACL 继续由 Rust 提供端执行；前端隐藏入口只改善体验。[Tauri capabilities](https://v2.tauri.app/security/capabilities/)、[本项目统一 ACL 决定](../adr/0017-guest-access-with-scoped-invitations.md)

## 4. 跨 Tauri／WASM 的类型契约需要补齐

当前 `BrowserNetwork::start(configuration: String)`、`status() -> String` 以及大量诊断方法以 JSON 字符串传递结构，错误为 `JsValue`；`exchange(message: String)` 还限制到 65,536 字节的请求／响应。它们是真实 SDK 接线，但不是五业务的正式类型化 Interface，更不是文件、终端的流式接口。`BrowserMemberConnection` 另有连接尝试、取消与诊断机制；应保留已有工作，不把它简化成一个虚构的统一生产 SDK。[BrowserNetwork](../../browser-probe/src/network.rs)、[成员连接](../../browser-probe/src/member_connection.rs)

建议先选定每业务必要的 command／query／订阅结果及稳定错误类别，让 Adapter 负责解析和封送；页面不要自行 `JSON.parse` 诊断字符串、匹配英文错误或调用 probe 名称。结果要区分“请求已接受”“对端已接收”“对端保存完成”和“当前无法确认”；尤其不能把入网获批等同于材料保存完成，或把单一来源同步等同于全网最新。[入网与同步语义](../../CONTEXT.md)、[浏览器保存完成证据](2026-10-02-browser-control-replica.md)

Tauri command 支持可序列化的参数、返回值、错误和异步调用；这提供 Rust 接入能力，但调用点写一个 TS 类型参数并不能自动证明它与 Rust DTO 相同。建议由一份稳定的 Rust DTO／schema 生成或核对两端类型，配少量真实序列化和 Adapter 契约验证；还须明确字段命名、枚举格式、可选值及超出 JS 安全整数范围的值。不要让 UI、Tauri、WASM 三处各手写一份长期漂移的业务模型。[Tauri 调用 Rust](https://v2.tauri.app/develop/calling-rust/)

`tauri-specta` 是可研究的命令／事件类型生成工具，其自有 README 明确区分 Tauri 1／2 与 Specta 版本；当前 Tauri 2 文档链接仍指向 v2 RC 路径。它可减少 Tauri 一侧重复声明，但不自动统一浏览器 SDK，也不替代运行时授权和输入校验。是否采用及如何固定版本应留到契约确定后，不能因为存在生成器就先把核心库绑定到 Tauri。[tauri-specta 第一方 README](https://github.com/specta-rs/tauri-specta)

浏览器端即使 `wasm_bindgen` 产出了绑定，现有返回值仍是字符串；绑定无法从该函数签名推出字符串内部的业务 schema。这是对本仓库函数类型的直接推论。浏览器 Rust/WASM 也处于同一页面／origin 的信任环境，不能声称其凭证天然不受页面脚本或恶意更新影响。[浏览器 SDK](../../browser-probe/src/network.rs)、[页面信任边界](2026-10-02-browser-control-replica.md#page-trust-and-sensitive-material)

## 5. IPC、数据流与响应式状态

Tauri Core 与 WebView 是分离的职责：Core 承接系统资源与状态，WebView 执行 HTML／CSS／JS；系统 WebView 不随应用重复打包，减少包体，但跨平台差异仍存在。这支持原生业务留在 Rust 的分工，**不能据此推出低运行内存**。[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)

Tauri 官方明确指出 event 面向少量数据，不适合低延迟／高吞吐，payload 为 JSON；channel 则面向有序流式更新。建议 command 承担操作，状态／进度采用必要的增量订阅，终端等确需前端消费的数据使用专门的流接入。官方 channel 的存在不等于已经具备 miopunch 需要的端到端背压或无限吞吐保证。[Tauri Rust → 前端通信](https://v2.tauri.app/develop/calling-frontend/)

“数据面留在 Rust”应精确解释为：原生文件网络收发与落盘、转发 socket 等不用绕经 Vue；但终端内容必须送到终端渲染器，浏览器选中的文件也可能需要 JS／WASM／浏览器存储接口共同处理。不能许诺所有业务字节永远不跨前端。建议让这些流位于专门句柄和有限队列中，Vue store 只保留任务、进度、会话状态和用户选择，避免整文件、全部终端历史、完整复制账本或密钥进入深层响应式对象。

Vue 官方提供按路由／功能动态加载、列表虚拟化、`shallowRef` 等实践；适合按实际热点采用。建议终端等较重 UI 延迟加载，WASM 只在浏览器入口导入，Tauri 依赖只在安装版 Adapter 导入。选 Adapter 集中在启动装配处，可采用分入口或构建别名；无需在每个页面散布 `isTauri`／`isAndroid`。运行时动态 import 可延迟加载，但不保证另一个 Adapter 完全不进入分发产物，是否排除应检查实际 bundle。[Vue 性能实践](https://vuejs.org/guide/best-practices/performance.html)、[Vite 动态导入](https://vite.dev/guide/features#dynamic-import)

终端高输出是需要真实验证的案例：xterm.js 官方说明异步 `write` 会缓冲，快速生产者可能让队列持续增长，需通过消费确认和水位传播流控；不能依赖“QUIC 本身有流控”就认为 IPC 与渲染队列也有界。使用 xterm.js 属于可复用实现候选，本次不选择或安装。[xterm.js 流控指南](https://xtermjs.org/docs/guides/flowcontrol/)

## 6. 生命周期与远端网页是独立的集成风险

建议原生节点由宿主拥有，页面卸载只取消自己的订阅，避免切换页面就销毁节点。是否关闭窗口后继续运行、是否分离守护进程，要按产品行为选择；不必预先把所有安装版改成 daemon + UI。Linux 无界面宿主需要独立运行节点库；Android 后台参与则需要明确的应用宿主与系统生命周期接线，Tauri 的跨平台窗口本身不提供“始终在线”保证。[原生节点](../../src/native_network.rs)、[当前生命周期方向](2026-10-03-shared-web-client-ui-direction.md#性能和生命周期约束)

Tauri 官方允许移动插件通过 Kotlin／Java 执行原生功能，也允许公共 Rust 逻辑复用；因此少量 Android Adapter 是正常做法，不必强迫所有系统调用都通过 Vue 或 WASM。Android 前台服务有用户可见通知、启动限制、服务类型及超时等规则，具体适用模式仍需核对，不能泛称“加一个 Service 就解决后台”。[Tauri 移动插件](https://v2.tauri.app/develop/plugins/develop-mobile/)、[Android 前台服务概览](https://developer.android.com/develop/background-work/services/fgs)

本仓库 Android 证据目前明确排除 APK、Service、应用私有存储及持续后台能力，且 ADB shell 中的 HTTPS 平台验证上下文问题曾需要受控资源替换绕开。未来安装版必须完成实际应用上下文接线；这不是换 UI 框架能解决的工作。[Android 实际能力边界](2026-10-03-android-management.md#actual-capability-boundary)

浏览器实际 `BrowserNetwork` 以 IndexedDB 和 origin-wide Web Lock 保存并独占节点，不能拿较早 `BrowserNode` 的 localStorage ping 入口代表当前存储实现。页面关闭仍是断连而非退网；浏览器管理员资格平等不承诺页面冻结后仍在处理请求。[storage.js](../../browser-probe/src/storage.js)、[节点 close](../../browser-probe/src/network.rs)、[浏览器角色决定](../adr/0014-browser-role-parity-with-contextual-reminders.md)

将浏览器节点移入 Worker 可以是后续调度选择，但不是现状：当前浏览器源码仍有 `web_sys::window()`，例如 IRC 定时器；不能把整包直接搬入 Worker 当作零成本优化，更不能把 Worker 或 PWA 当成持续后台节点保证。[浏览器 IRC](../../browser-probe/src/irc.rs)、[当前浏览器库](../../browser-probe/src/lib.rs)

**远端内网页面应与管理 UI 的高权限运行环境隔离。** Tauri 官方 capabilities 文档明确：应用注册命令默认可由应用 windows／webviews 使用，可用 `AppManifest::commands` 调整；远端来源的 IPC 需显式配置。文档还特别提示 Linux／Android 无法区分嵌入 iframe 与窗口的请求。因此不能只加一个 iframe 显示远端服务，再认为 Tauri 自动隔离了所有本机命令。确切窗口／WebView、命令权限、远端来源与导航隔离方案须单独设计验证。[Tauri capabilities 与 remote API access](https://v2.tauri.app/security/capabilities/)

浏览器内网 Web 模块还须明确受支持的 HTTP、WebSocket、重定向、认证 cookie、相对／绝对 URL 以及 origin 行为；这里不选择代理、重写或导航方案，也不承诺任意 host:port 服务透明兼容。共享 Vue 能复用入口和会话状态，不能替代这一模块的内容承载与兼容设计。[当前已明确的场景边界](2026-10-03-shared-web-client-ui-direction.md)

## 7. 前端验证与实际平台验证各证明什么

恢复实施时建议用 Vue 推荐的常规测试工具验证共享交互和公开行为，重点覆盖入网阶段、服务关闭／不支持／被拒绝、来宾受邀入口及取消等用户可观察结果。Adapter 可用测试替身，但不能把替身成功当成 OS 插件成功。[Vue 测试指南](https://vuejs.org/guide/scaling-up/testing.html)

Tauri 提供 `mockIPC` 截获命令、模拟返回；这适合检查页面是否传对参数和处理结果，无法证明 Rust command 注册、capability 配置、文件选择器、剪贴板权限、真实网络或后台生命周期。[Tauri Mock APIs](https://v2.tauri.app/develop/tests/mocking/)

截至本次核对，**不能沿用“所有移动端 E2E 均不支持”或“所有桌面 WebDriver 均不支持 macOS”的旧概括**。当前官方 WebDriver 总览推荐 `@wdio/tauri-service`，嵌入式 server 支持 Windows／Linux／macOS；直接运行 `tauri-driver` 的桌面路径仍仅 Windows／Linux。官方 manual setup 写明 Android／iOS 可通过 Appium 2，但流程尚不顺畅。这是工具可用性证据，不是 miopunch 移动端测试已接入。[当前 WebDriver 总览](https://v2.tauri.app/develop/tests/webdriver/)、[官方 manual setup 源码](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/develop/Tests/WebDriver/manual-setup.mdx)

最有价值的后续验证是少量穿过真实 seam 的纵向流程：同一页面对 Native／Browser 两个 Adapter 使用一致的完成语义；实际 Windows／Linux WebView 调用 Rust；实际 Android 安装包完成一项系统能力与生命周期流程；文件传输遇到取消／保存失败、终端持续高输出后能及时中断。浏览器自动化可证明浏览器 UI 与 WASM 节点协作，无法代替 Android 系统权限和后台测试。现有探针的 P2P 证据继续有效，但不会自动覆盖这些新增层。[既有探针与产品 UI 的区分](2026-10-03-shared-web-client-ui-direction.md#共享界面与两种节点接入)

## 8. 尚不能用文档替代的核验

| 关键未知 | 对栈判断的影响 |
| --- | --- |
| Tauri 安装版如何接管 NativeNetwork 所有权与长任务，Android 应用／后台宿主如何接线 | 决定生命周期 Adapter；目前无证据要求更换 Vue。 |
| 五 Module 的最小 Interface、实际支持矩阵与数据流句柄 | 决定共享与平台差异的真实范围；不宜先写万能 SDK。 |
| 浏览器内网页面的 origin／代理／隔离与站点兼容范围 | 是独立业务架构问题；可能影响可交付边界。 |
| 原生键材保管、浏览器长期凭证接入与恢复流程 | 属于节点／平台安全和已有产品语义，不应由前端 store 发明规则。 |
| 内存、空闲 CPU、启动延迟、终端输入延迟、传输期间 UI 响应 | 需目标平台实测；框架宣传、压缩 bundle、包体大小均不能回答。 |

资源比较应使用一致的业务负载、发布模式、相同设备与平台版本，对照纯节点和节点加 UI；统计应覆盖关联 WebView 进程，按平台使用一致口径，并避免把共享页重复累计成应用私有消耗。也应观察关闭重开页面／终端后订阅、队列与句柄能否释放。现在没有预算与实测，不能给出“足够省内存”的结论。[现有性能约束](2026-10-03-shared-web-client-ui-direction.md#性能和生命周期约束)

本报告支持继续讨论既有栈提案，不构成框架批准、实现计划、动态插件设计或新 ACL 决议；所有确定产品语义仍以 CONTEXT、当前讨论与已接受 ADR 为准。
