# 共享 Web 与客户端 UI 的讨论记录

日期：2026-10-03。状态：UI 延后，保存当前讨论结果；框架推荐与接入结构仍是建议。本次不启动 UI 实现，也不因本记录自动安排原型、性能实验或框架迁移。

本轮讨论关注普通浏览器与安装版客户端如何复用界面代码，以及在 Rust 节点承担业务功能的前提下如何控制开发、维护和运行成本。当前推荐为 Vue 3、TypeScript、Vite，安装版使用 Tauri 2；用户尚未明确指定采用这套框架，因此不将该推荐写成已接受的 ADR。

## 已确认的取向

- UI 可以延后，当前先保存讨论结果。
- 代码复用和长期维护成本优先，不要求所有界面代码都用 Rust。
- 可以使用 WebView，但不接受为了界面造成大量无必要的内存开销；有一定性能要求，具体预算尚未给定。
- 业务功能以 Rust 节点为主体。前端负责展示、输入和交互，网络、身份、授权、存储与同步由节点处理。
- 希望最终使用一个 UI 框架，持续维护；只有遇到严重实际问题才考虑更换，不为理论上的性能差异反复选型。
- UI 的延期不改变已有浏览器节点定位，也不启动仍为低优先级的 [WebRTC 候选](2026-10-03-browser-webrtc-assessment.md)。

## 当前框架建议及依据

**当前建议：Vue 3、TypeScript、Vite，安装版配合 Tauri 2。** 这是一项基于项目需求的工程判断，不是社区规模统计、成本实测或用户已经批准的框架决策。

Vue 有完整的[中文官方文档](https://cn.vuejs.org/guide/introduction)，以及 [PrimeVue](https://primevue.org/) 等提供表单、列表和对话框的组件库。这些现成资源与 miopunch 的成员管理、审批和设置界面相符，因此建议优先用其降低界面基础设施的开发和维护成本。具体组件库尚未选择。

Tauri 官方提供 `vue-ts` 模板，并推荐使用 Vite 构建普通 Web 前端，支持 Vue、Svelte、React 等多种框架；官方没有指定唯一最佳搭配。[官方模板](https://github.com/tauri-apps/create-tauri-app)、[前端集成指南](https://v2.tauri.app/start/frontend/)

Hoppscotch 提供 Vue 前端同时用于 Web 和 Tauri 桌面版的实际参考：共同代码位于共享包，入口根据 Web 或 desktop 选择平台配置后创建应用。这个实例支持代码组织方式可行，不证明 miopunch 的移动端交付或性能已经合格。[共享包依赖](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-common/package.json)、[共享应用入口](https://github.com/hoppscotch/hoppscotch/blob/main/packages/hoppscotch-selfhost-web/src/main.ts)

此前 Svelte、Leptos 和 Dioxus 也进入了比较。GitButler 的 Svelte、TypeScript、Tauri 组合是一个实际项目参考；Leptos 与 Dioxus 可以用 Rust 编写界面。用户后来明确不追求全部 Rust，并更重视长期维护成本，因此本轮最终推荐收敛到 Vue，不安排多框架并行实现。[GitButler 开发说明](https://github.com/gitbutlerapp/gitbutler/blob/master/DEVELOPMENT.md)、[Leptos 客户端构建](https://book.leptos.dev/getting_started/index.html)、[Dioxus 平台说明](https://dioxuslabs.com/learn/0.7/guides/platforms/)

## 共享界面与两种节点接入

Tauri 提供安装版客户端的窗口、WebView、系统集成、打包和前后端通信。具体页面由 Vue 等前端代码提供；安装版前端通过 Tauri command 调用本地 Rust，普通浏览器版本则调用浏览器中的 Rust/WASM 节点。[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)、[调用 Rust](https://v2.tauri.app/develop/calling-rust/)

建议采用一个前端项目、两个运行入口，复用页面和操作流程，将差异集中在节点操作 Module 的 Interface 后。Interface 包括统一的结果、错误、操作完成含义和运行能力；客户端与浏览器分别提供 Adapter。下面是建议结构，尚未实现：

```text
共享 UI 页面与操作流程
    -> 节点操作 Module 的统一 Interface
         -> 客户端 Adapter -> Tauri 通信 -> NativeNetwork
         -> 浏览器 Adapter -> WASM 调用 -> BrowserNetwork
```

当前源码已有 [NativeNetwork](../../src/native_network.rs) 和通过 `wasm_bindgen` 暴露的 [BrowserNetwork](../../browser-probe/src/network.rs)，浏览器 SDK 也[导出了节点入口](../../browser-probe/src/lib.rs)。这些是当前接入依据；统一的 UI Interface、Tauri 外壳和正式页面尚未建立，不能将现有 SDK 和测试入口当作已经交付的产品 UI。

| 范围 | 建议复用方式 |
| --- | --- |
| 页面、组件、样式、导航和表单 | 共用源码，按屏幕尺寸调整布局。 |
| 加入、审批和管理流程 | 共用展示与交互，根据节点返回的业务状态更新界面。 |
| 节点命令、状态、错误和更新订阅 | 面向同一 Interface，由两个 Adapter 接入。 |
| 文件、通知、后台运行和应用生命周期 | 由平台分别提供能力，界面据实际能力调整。 |
| 业务权限与持久状态 | 继续由 Rust 节点验证和保存，UI 不另建业务权限或存储模型。 |

普通浏览器仍是具有独立身份和成员资格的节点，不因共享 UI 变成远程原生节点的管理网页。原生侧与浏览器侧的传输、存储和生命周期能力不同，共用源码不承诺所有平台能力相同，也不要求两端使用完全相同的构建产物。现有能力依据见[桌面管理](2026-10-03-desktop-management.md)、[Android 管理与生命周期](2026-10-03-android-management.md)及[浏览器管理与复制](2026-10-02-browser-control-replica.md)。

## 性能和生命周期约束

以下是本轮建议，尚未实施或测量：

- 安装版由原生 Rust 节点处理业务，WebView 只运行界面，避免在同一客户端再启动一份浏览器 WASM 节点。
- UI 获取必要的状态和增量更新，控制更新频率与队列；业务数据路径不绕经界面，避免频繁跨进程传输整份历史或大数据。
- 按页面需要引入组件，长列表和大日志视图在实际需要时采用分页或虚拟列表。Vue 官方提供按需加载、列表虚拟化等[性能实践](https://vuejs.org/guide/best-practices/performance.html)。
- 节点运行生命周期与界面分开。Android 后台参与需要平台运行入口承接；浏览器仍遵守页面冻结、关闭和重开的限制，UI 框架不自动提供持续后台联网能力。
- 后续若恢复 UI 工作，用目标平台适合的指标比较纯节点与节点加 UI 的内存、空闲 CPU、启动和操作延迟。统计应包含 WebView 相关进程，并使用相同业务负载；小安装包、编程语言和框架名称都不是低运行内存的证据。

目前没有 miopunch UI 的内存或速度实测，也没有确定 MB 上限、延迟目标或受支持的最低设备规格。上述观察方法不是本次已经获准运行的实验；本记录只保存后续讨论所需的依据。

## 延后时保留的未决项

恢复 UI 工作时再确认框架是否采用当前 Vue 与 Tauri 推荐、具体组件库、平台覆盖与能力展示、统一 Interface 的操作和错误语义、Android 运行接入以及性能预算。现阶段不冻结依赖版本，不生成正式 UI 规格，也不把文档记录当作实现领取。

已有入网语义继续作为未来界面依据：等待批准、已批准但材料未保存完整、已入网但暂不可达需要分别表达；成员资格、当前连接与来源限定的同步结果也需要分别表达。详见 [CONTEXT](../../CONTEXT.md)、[入网完成记录](2026-10-01-admission-completion-validation.md)及[场景推演](2026-09-28-network-scenario-walkthrough.md)。
