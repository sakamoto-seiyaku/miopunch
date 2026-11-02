# 共享 UI 的浏览器适配研究：五个模块与内网页面访问

后续决定：用户已采纳本技术栈方向，见 [ADR 0018](../adr/0018-shared-vue-ui-with-tauri-shells.md)。用户还要求内网页面留在 miopunch 内，并按所选节点可达的服务定义需求，无需先指定网站产品；剪贴板收敛为一对一、开启后自动同步。下文保留采纳前的候选分析与证据边界；系统浏览器打开和分组剪贴板已不作为交付方案，选型确认也不代表运行验证完成。

日期：2026-10-20。性质：只读研究与工程建议，不是框架批准、产品决议或实施授权。核对对象为 Vue 3 + TypeScript + Vite 的共享前端、Tauri 2 安装版外壳，以及独立 Rust/WASM 浏览器／PWA 节点。本轮只新增本文，未构建、安装、运行验证、修改生产代码或操作 Plane。

## 研究结论

**浏览器能力核查没有发现必须放弃 Vue 的依据；也不能据此批准整套 Vue／Tauri 方案。** 剪贴板、文件、转发管理和远程终端的交互适合共享 Web UI，但浏览器提供端与使用端的能力必须分别表达。最需要先消除的不确定性是模块四：通过现有 P2P 访问内网 `地址:端口`，怎样成为可登录、可加载资源、可保持安全隔离的浏览器页面。它决定的是服务访问接入结构和兼容承诺，通常不由组件框架决定。

需要明确分开三种结果：

1. **HTTP 数据能经 iroh stream 往返**：有现有传输基础，工程上合理；本次未实现 HTTP 模块，仍属待验证推断。
2. **若干选定 Web 应用能在浏览器中使用**：可能通过受限 HTTP 适配、隔离的代理入口或 Service Worker 等实现，取决于站点和架构。
3. **普通浏览器直接导航任意远端内网站点，保持原始网站语义**：现有 Rust/WASM 节点并未提供这种能力，前两项也不能证明它。当前产品并未承诺任意站点全部兼容。

**建议保留 Vue／Tauri 作为候选，先做最小的服务访问边界验证，再冻结外壳的网页承载方式。** 如果决定要求“纯普通浏览器、无本地辅助／浏览器扩展／可达 HTTP 网关，且对任意原站点完全透明”，应重新审视这组要求的组合；换 Vue 为其他 UI 框架不会改变浏览器网络接口。若只需选定站点，不能因通用透明访问困难就否定整个浏览器独立节点方向。

## 已知基线与证据等级

仓库的 [CONTEXT](../../CONTEXT.md)、[UI 讨论末尾](2026-10-03-shared-web-client-ui-direction.md)、[ADR 0014](../adr/0014-browser-role-parity-with-contextual-reminders.md) 和 [ADR 0017](../adr/0017-guest-access-with-scoped-invitations.md) 给出当前产品约束：五个模块；Windows、Linux（含无界面）、Android、普通浏览器／PWA；统一角色、准入与有效 ACL；节点仅声明实际具备的服务能力。浏览器的管理员或创世权限不因平台而另设限制；平台许可、运行生命周期和存储风险另行表达。

本研究使用以下标签区分结论：**源码事实**指本轮检查到的仓库结构；**平台事实**指下文链接的一手规范或官方文档；**推断／建议**指在这些边界上的工程判断；**待验证**不代表已经运行通过。规范草案中存在接口不等于全部目标浏览器已经支持；浏览器检测也不能只看品牌。

### 独立浏览器节点已经有接线，不能降格成管理网页

- **源码事实**：[BrowserNetwork](../../browser-probe/src/network.rs) 是 `wasm_bindgen` 暴露的节点，持有独立 `RestrictedEndpoint`、保存的身份材料、准入／管理状态和连接资源；启动代码以保存的 secret key 创建 iroh endpoint，按许可配置 relay 和自定义路径。
- **源码事实**：[member_data.rs](../../browser-probe/src/member_data.rs) 是已有一跳成员数据服务的浏览器客户端，内部 QUIC 数据保持不透明，并检查外层成员证明和显式 `ForwardData` 许可；它不是 HTTP 网页代理，也不是浏览器 TCP socket。
- **源码事实**：[storage.js](../../browser-probe/src/storage.js) 采用 IndexedDB 事务保存状态，并以 Web Lock 防止多个页面同时使用同一身份；[network.html](../../browser-probe/network.html) 直接初始化 WASM 并启动 `BrowserNetwork`，当前按钮仍是准入与测试消息入口。
- **边界**：这些源码证明有真实独立节点与流接口的接入基础，不证明五个产品模块、站点导航或生产 UI 已交付。本轮没有重新运行既有 RTC／relay 实验，不能扩大以前回执的证明范围。[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md) 仍约束普通数据路径、许可和断线语义。

普通网页不能任意建立通用 TCP／UDP socket 或监听本机端口；WebSocket 也不是原始 socket。Chrome 的 Direct Sockets 是 **Isolated Web App** 能力，不能当普通 PWA 的补齐接口；本项目已排除该交付路线。Rust 编译为 WASM 不会新增宿主没有提供的系统 API。[Chrome Direct Sockets 官方说明](https://developer.chrome.com/docs/iwa/direct-sockets)、[WHATWG WebSockets §1](https://websockets.spec.whatwg.org/#introduction)

## 五个模块：浏览器使用端与提供端

下表是能力边界，不是首版验收完成表；所有实际操作仍须通过统一的节点身份与服务 ACL。

| 模块 | 浏览器作为使用端 | 浏览器作为提供端 | 对共享 UI 的影响 |
| --- | --- | --- | --- |
| 剪贴板共享 | 可接收内容，用户明确复制到本机剪贴板；部分运行环境可在许可下减少动作 | 可让用户贴上／明确读取后发送给双机或指定组；不能统一承诺系统级后台监听 | 共享目标、内容、状态；按实际许可显示贴上／复制动作和自动模式可用性 |
| 文件传递 | 可在线接收并保存；大文件需有可控内存的落盘路径 | 可发送已选择的文件，在线时可按协议提供其数据；不等于任意读取本机目录 | 共享传输流程和进度，选择与保存由平台 adapter 承担 |
| 端口转发 | 可管理获准的原生转发，或在专门业务 adapter 中使用流；不能向本机其他应用提供通用监听端口 | 普通 Web 无法成为任意本地 TCP 服务的连接／监听网关；可提供应用自有协议能力，但不能冒充原生端口转发 | 设置和状态可共享；“可管理”与“本节点能转发”分别表达 |
| 浏览器访问服务 | 可以研究 HTTP／站点适配；完整导航、登录和 WebSocket 需额外接入 | 不能直接通过裸 TCP 访问自己的 LAN 服务；可对外提供应用自己持有的数据，但与代理已有 LAN 网站不同 | 主 UI 可共享；被访问网站的 origin 和视图必须独立设计 |
| 远程 shell | 终端模拟器作为 UI，经节点流连接远端 PTY／tmux | 普通 Web 不能启动宿主 shell、PTY 或 tmux，不能声明这类原生 shell 提供能力 | 终端视图可共享，远端会话与平台适配归 Rust／宿主 |

表中剪贴板、文件与端口的依据见下节；shell 使用／提供端的区别在于终端 UI 与宿主进程是两个部件，并非浏览器没有独立节点身份。

### 剪贴板：明确交互可行，后台无感同步不能作为共同保证

**平台事实**：Async Clipboard API 限于安全上下文，读取和写入受焦点、许可、用户激活等条件约束。Chromium 的已授权行为与 Firefox／Safari 不同：后两者的读取可能需要粘贴提示，写入通常需要短暂用户激活；不能把一个浏览器的永久授权行为作为共同接口契约。Clipboard API 也不在 Web Worker 中暴露。[Mozilla Clipboard API 参考](https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API)、[WebKit 官方实现说明](https://webkit.org/blog/10855/async-clipboard-api/)

2026-06 的 W3C 草案已经包含 `clipboardchange`，并讨论远程剪贴板同步；不能因此说 Web 永远没有变化事件。但草案的事件仍与激活／读取许可、系统焦点相关，失焦时记录待通知状态，实际互操作也需检测。因此，“可监听变化”仍不等于“所有浏览器持续后台自动同步系统剪贴板”。[Clipboard API 草案 §5.2.1](https://www.w3.org/TR/clipboard-apis/#clipboardchange-event)

**建议**：共同操作先覆盖“贴上并发送”“接收后复制”；将可用的自动读取／写入作为实际运行能力表达。远端管理员授予服务权限，不能代替本机浏览器用户对系统剪贴板的许可。不要将 UI 内持有的共享内容、已经收到远端内容、成功写入系统剪贴板合成一个成功状态。文本、HTML、图片和应用自定义格式的兼容范围另行验证，不能由纯文本成功外推。

### 文件：数据传输与用户保存分开，流式接口不等于整链路低内存

**平台事实**：`<input type=file>`／拖放取得用户选择的 `File`，可按 Blob 分块或流式读取，无需先把完整文件转成单个 ArrayBuffer。获取文件并不授予随意读取文件系统的能力。[W3C File API](https://www.w3.org/TR/FileAPI/)

接收端有两类路径：支持时，在明确用户动作中取得保存 handle，再将分块写入 `FileSystemWritableFileStream`；否则使用浏览器支持的下载／保存流程。`showSaveFilePicker` 不是全部目标浏览器的共同保证，必须检测；不要等远端长耗时传输后才尝试使用已经消失的短暂用户激活。关闭可写流的完成语义也应区别于网络接收完毕。[Chrome 文件系统访问指南](https://developer.chrome.com/docs/capabilities/web-apis/file-system-access)、[Mozilla showSaveFilePicker 参考](https://developer.mozilla.org/en-US/docs/Web/API/Window/showSaveFilePicker)

OPFS 是 origin 私有存储，可作暂存，不是用户可见的“下载目录”。其数据受站点存储和配额约束，清除站点数据也会删除；不能把网络接收到 OPFS 等同于用户已经保存一份独立文件。请求 persistent storage 可以改善自动回收风险，但需要浏览器许可，用户仍能主动清除数据。[Mozilla OPFS 参考](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system)、[WHATWG Storage 持久化](https://storage.spec.whatwg.org/#persistence)

**推断／待验证**：使用 `File.stream()` 只证明输入可以分块。WASM／JS 边界复制、协议队列、接收暂存、Blob 下载方案仍可能随文件增大而占满内存。需要一个大于可用内存的真实文件验证流式落盘、取消和失败结果；缺少可控保存路径时，应明确该浏览器的能力边界，不能用“支持 PWA”代替大文件证据。本研究没有选择断点续传、长期目录授权或后台下载架构。

### 端口转发：P2P stream、系统 socket 和界面权限不是同一接口

**推断**：浏览器可以在 ACL 允许时向原生节点发起业务请求，或者管理原生节点建立的转发；也可以把某种客户端协议在 WASM 中编码后经现有 stream 发送。这些不会自动生成 `127.0.0.1:<port>`，供浏览器之外的 SSH／数据库客户端连接。提供端连接目标 `host:port` 和使用端监听本机端口，应由具备系统 socket 的运行环境承担；传输中继搬运的是流或报文，也不自动成为目标 LAN 的服务网关。普通 Web 的 socket 限制与 IWA 区别仍适用。[Chrome Direct Sockets](https://developer.chrome.com/docs/iwa/direct-sockets)

因此端口模块可以共享 UI，但应返回方向、实际 provider、可用端口／协议能力等事实。是否允许调用者传入任意地址，仍由模块范围和统一有效 ACL 决定；本研究不把“固定白名单目标”或“任意地址”任一方升格为既定政策。

### 终端：浏览器适合渲染，tmux 在远端保存会话

**平台事实**：xterm.js 是前端终端模拟器，不负责创建远端 shell。tmux 在远端管理会话，客户端断开后会话可以保留，再 attach；这不包括远端重启或 tmux server 退出后的进程恢复。[xterm.js 官方文档](https://xtermjs.org/)、[tmux Getting Started](https://github.com/tmux/tmux/wiki/Getting-Started)

**建议**：浏览器节点流与原生节点流都可接同一终端视图；输入、输出、尺寸变化和重新附着由各自 adapter 接入同一业务语义。不要因为 xterm 示例用 WebSocket 就增加必需的中心 shell 网关，也不要把 demo／attach addon 的接线当生产授权。终端输出及标题、链接等元数据视为不可信内容，不插入 `innerHTML`；页面内任何脚本可接触终端 I/O，因此终端与不可信内网站点必须分离。[xterm.js 安全指南](https://xtermjs.org/docs/guides/security/)

**待验证**：`term.write()` 的返回不是消费完毕；应利用处理完成回调和高低水位，把背压传回 stream／远端 PTY，避免无限接收队列。高速输出时还要验证 Ctrl-C、输入响应和恢复；QUIC 有流控不代表 UI 解析队列已受控。终端对象宜由组件生命周期显式持有和销毁，输出按块直接写入终端，不逐字节转成 Vue 响应式 DOM。[xterm.js 流控指南](https://xtermjs.org/docs/guides/flowcontrol/)

## 模块四：为什么能搬运 HTTP 不等于能打开任意网站

### 首先需要一个真实的浏览器接入点

**平台事实**：导航、资源加载和 WebSocket 由浏览器按 URL、origin 和标准网络接口处理。普通网页没有一个注册任意 TCP 连接工厂、把地址栏对 `http://nas:8080` 的连接替换为现有 iroh stream 的通用接口。Service Worker 能在其受控客户端的 Fetch 流程中提供 Response，但注册脚本和 scope 必须同源；它不是系统路由器，不能为任意远端 origin 安装控制器。[Service Workers 注册与 scope 规范](https://w3c.github.io/ServiceWorker/#register-algorithm)、[WHATWG WebSockets](https://websockets.spec.whatwg.org/)

**推断**：可以设计“请求方法／路径／headers／body 经 P2P → 原生 provider 访问指定内网服务 → 返回状态／headers／body”。也可设计更底层的字节流隧道，另行处理 HTTP 或 TLS。二者都有现有传输基础，但还需要 adapter、取消和背压、目标身份／证书处理、ACL 执行。浏览器原生 `fetch(remoteURL)` 不会因此自动走该协议。若由 provider 终止目标 HTTPS，浏览器到 provider 的加密与 provider 到站点的 TLS 信任应分别说明，不能声称浏览器直接验证了原站点证书。

### 候选路径及其代价

以下均是可比较的候选，未选定。一个入口可以按站点声明兼容范围，不能把不适配的站点归为 UI bug。

| 候选路径 | 可以解决什么 | 仍须解决什么 | 与现有方向的关系 |
| --- | --- | --- | --- |
| 限定 HTTP 请求／为选定应用做适配，在 miopunch 自有 UI 中展示结果 | 简单请求、特定 API 或受限页面可经现有 stream 工作 | 不等于完整原网页；脚本、导航和登录范围要明确 | 最容易局部验证；不能偷偷把原网站 HTML 注入高权限页面 |
| 隔离 origin 的 Service Worker 虚拟 HTTP 入口，经消息把请求交给独立节点 | 可由真实浏览器引擎加载部分页面和资源，P2P 仍由浏览器节点承担 | 隔离 origin 的可信启动／SW 交付、节点页存活、请求授权、cookies、WebSocket、原应用 SW 和跨 origin 资源 | 有研究价值；不是已存在的通用代理，也不天然要求数据服务器解密流量 |
| 浏览器可达的 HTTP(S)／WebSocket 反向代理入口，后端接 P2P | 浏览器使用常规网络栈，代理可处理站点转发 | 代理部署、可达性、TLS／域名、认证、cookie 与 URL 变换、服务内容信任 | 改变访问模块拓扑；不是既有 opaque 数据 relay 自动附带的能力，也不能作为未确认的必需中心服务 |
| 安装版 Rust 开本地端口／代理，交给外部浏览器或无 IPC 的独立视图 | 对 TCP、HTTP、WebSocket 的承载更接近常规访问 | 本地端口是变更后的 origin；原站点绝对 URL、证书和登录仍需适配，不能保证全兼容 | 可作为安装版能力；不能充当纯普通浏览器无需辅助的证明 |
| 原生系统路由／代理配置、扩展或其他网络接入 | 可把更多原生导航请求交给隧道，较少改写站点 | 分发、系统权限、DNS／TLS 与平台支持；浏览器本身不能设置任意系统路由 | 涉及尚未选定范围；IWA 不在本次候选内 |
| 远端运行浏览器，仅传画面与输入 | 原站点逻辑在可访问 LAN 的远端执行，可避免部分 URL 改写 | 增加远端浏览器、流媒体、输入及剪贴板／下载语义，改变产品体验和资源成本 | 单独产品路线，未选择；当前首版不因这一候选纳入串流 |

上述接入路径是依据规范边界的工程比较，没有运行证明。Service Worker 不是永久后台节点：其执行由事件维持，浏览器可终止它；WebRTC 的 `RTCPeerConnection` 标准暴露在 Window 环境，不能假设把现有 Window 端节点整体搬进 SW 就可维持 RTC。因此经节点页面桥接的方案还需要证明冻结、重开和首个请求的行为。[Service Workers 生命周期](https://w3c.github.io/ServiceWorker/#service-worker-lifetime)、[WebRTC RTCPeerConnection 接口](https://www.w3.org/TR/webrtc/#rtcpeerconnection-interface)

### 必须逐层核对的网站语义

**资源与重定向。** `src`／`href` 的相对路径、`/` 根路径、绝对 LAN URL、`<base>`、CSS URL、脚本动态生成的 URL、form action、Location 重定向都可能从代理入口逃回原始地址。单次 HTML GET 成功不证明这些路径。站点需配置外部 base URL，或由 adapter 重写；重写 HTML 本身也不覆盖动态 JavaScript。这里是待验证的站点行为，不是所有网站必定失败。

**Origin、CORS 与登录。** 代理 URL 会改变浏览器看到的 origin；origin 由 scheme、host、port 决定，路径不同不形成隔离。普通脚本跨 origin 读取受 CORS 约束，`no-cors` 不能变成读取任意响应正文的通道；顶层导航本身则不要求目标返回 CORS，不能把两者混为一谈。[HTML origin 规范](https://html.spec.whatwg.org/multipage/browsers.html#origin)、[Fetch CORS 规范](https://fetch.spec.whatwg.org/#http-cors-protocol)

Cookie 的 Domain／Path／Secure、HttpOnly 以及现代 SameSite／第三方 cookie 策略可能因改域、改 scheme 或 iframe 嵌入而改变行为。Fetch 将 `Cookie` 等请求头和 `Set-Cookie` 响应头置于受限处理范围；不能假设 JS 从 P2P 收到任意 headers 后构造一个 Response，就能得到与真实 HTTP 网络响应相同的浏览器登录状态。保留原生 cookie 语义或另做会话 jar，各有边界；JS `document.cookie` 不能替代 HttpOnly。应验证实际登录、刷新、注销和 CSRF／Origin 检查，避免只看首页。[Fetch headers 与 cookie 处理](https://fetch.spec.whatwg.org/#forbidden-response-header-name)、[RFC 6265 HttpOnly](https://www.rfc-editor.org/rfc/rfc6265.html#section-4.1.2.6)、[Mozilla Cookie 指南](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies)

**CSP 与 iframe。** CSP 的 `connect-src`／`script-src`／`form-action` 等可限制资源和交互；`frame-ancestors` 与 `X-Frame-Options` 可拒绝嵌入。iframe 的 `src` 仍使用浏览器自己的网络栈。直接打开受限站点或随意剥掉全部安全头，不是一个可统一承诺的 iframe 解决方案。[W3C CSP3](https://www.w3.org/TR/CSP/)、[RFC 7034](https://www.rfc-editor.org/rfc/rfc7034/)

**WebSocket。** 标准 WebSocket 握手的 `service-workers mode` 明确为 `none`。因此“SW fetch 能返回 HTML／API”不能证明站点原有 `new WebSocket(...)` 也能被 SW 转进 iroh。需要真实可达的 ws/wss 网关、站点配合或另行适配；替换全局 WebSocket 构造器还涉及 worker、脚本执行顺序和对象语义，不是通用兼容保证。[WHATWG WebSocket 握手 §2.2](https://websockets.spec.whatwg.org/#opening-handshake)

**HTTPS 与 mixed content。** 安全上下文内嵌 HTTP 资源／页面、请求 HTTP API 或连接不安全 ws 时，受 mixed content 等规则限制；加密的 P2P payload 不会自动改变浏览器看到的 URL 安全属性。顶层 HTTP 导航与 HTTPS 页面的不安全子资源不是同一情况。若采用 HTTPS 代理 origin，需要分别处理其证书、原站点 HTTP／HTTPS 及绝对 URL；不能统一关闭浏览器安全检查。[W3C Mixed Content](https://www.w3.org/TR/mixed-content/)

### 不可信网站与高权限客户端必须分离

**源码与平台事实**：浏览器节点目前按 origin 保存密钥和状态。同源页面脚本通常处在同一权限域，WASM 内存不是对同源恶意脚本的安全保险箱。因此把任意内网站点挂到客户端 origin 的 `/proxy/...`、`srcdoc` 或带原 origin 的 Blob 页面，并允许其脚本运行，会把网站内容提升为可接触节点身份／终端／管理入口的代码。不同 path 不是所需隔离。[现有 storage.js](../../browser-probe/src/storage.js)、[HTML origin 规范](https://html.spec.whatwg.org/multipage/browsers.html#origin)

**建议**：网站在独立 origin／独立无特权上下文运行，多个不互信站点也需考虑彼此隔离。高权限节点页只接受经过来源、会话、服务和当前 ACL 约束的有限请求，不能把持有密钥的 WASM 对象、全能命令接口或永久授权令牌交给站点。严格 sandbox 的 opaque origin 可提供部分隔离，但会改变存储、cookie、CORS 等能力；它也不能单独证明复杂站点兼容。隔离 origin 的 bootstrap 与受控消息桥是需要验证的架构成本，不应藏在组件实现里。

**Tauri 特别风险**：官方默认仅向 bundled code 开放 API，但远端来源可被 capability 显式授权；并且文档明确警告 **Linux 与 Android 无法区分 embedded iframe 与窗口本身的请求**。不能假定高权限 Tauri 主 UI 内放一个 iframe 就安全。候选包括外部浏览器、独立无 IPC 的视图；具体配置与自定义 command 暴露也须验证，不能仅依赖“没配远端权限”。这项风险会影响 Tauri 内网页面承载方式；若安全隔离在目标平台无法成立，才可能需要换这一部分 shell 方案。它不直接否定 Vue 管理页面。[Tauri 2 capabilities 官方说明](https://v2.tauri.app/security/capabilities/)

## PWA 和统一权限不能消除生命周期差异

页面可被隐藏、冻结或丢弃，冻结期间任务停止执行；关闭或回收后，不能声称仍在线核销邀请、提供文件、接收 shell 输出或维持转发。安装为 PWA 本身不是始终运行的后台服务承诺。[Chrome Page Lifecycle 官方说明](https://developer.chrome.com/docs/web-platform/page-lifecycle-api)

持久状态与持续运行应分开。IndexedDB／OPFS 可以跨重新打开保留状态，但配额、回收和用户清除仍要处理；persistent storage 许可不等于防丢备份。按 [ADR 0014](../adr/0014-browser-role-parity-with-contextual-reminders.md)，这些事实通过能力和情境提醒表达，不新增浏览器管理员禁令。浏览器 UI 可继续使用同一套角色与 ACL 操作；在线、能力可用、OS 许可和业务已完成是不同状态。

## 最少但有决定作用的后续验证

下面是未来恢复实现时的验证建议，本轮没有执行，也不是要求同时构建多个完整方案。

1. **先选一个真实目标站点，定义最低可接受访问结果。** 记录它实际使用的 HTTP／HTTPS、登录、资源、重定向与 WebSocket；用一条候选接入路径完成首页、静态资源、登录后操作、刷新、注销及双向 WebSocket。浏览器必须实际运行自己的 Rust/WASM 节点，不能由原生节点偷偷代持其身份。若失败，记录是站点可配置问题、adapter 缺失还是普通 Web 的接口边界。一个站点成功只形成该类站点证据。
2. **同一次站点验证必须包括隔离攻击。** 让目标页尝试读取客户端 IndexedDB／节点凭证、调用节点管理／shell API、调用 Tauri IPC，并测试未授权目标与撤销后访问。确认独立 origin 和桥接范围真实成立，特别覆盖 Linux／Android；不能把隔离留到“功能完成后”。若本轮候选依赖 SW，同时检查首个导航、节点页冻结／关闭、worker 终止后恢复、cookie 和 WebSocket 的真实路径。
3. **浏览器基本交互各做一个边界用例。** 在实际受支持浏览器及 Android 上，核对一次明确剪贴板读／写与失焦行为；传一个大于内存预算的文件并验证低内存落盘、取消和保存完成含义。若没有共同的大文件保存路径，先收敛支持范围，不把整份 Blob 堆入内存称为通过。
4. **终端与生命周期共用一次验证。** 浏览器连接远端 tmux，检查输入／尺寸变化、持续高速输出与 Ctrl-C、断网再附着，以及服务撤销后的拒绝。冻结或关闭页面时 tmux 继续并不代表浏览器仍在线；不自动重放断线期间的输入。

决定框架前仍需有目标平台运行成本证据；本报告不提供 Vue／Tauri 的内存合格结论。上述浏览器功能验证主要回答模块与桥接边界，不宜被包装为 UI 框架性能比较。

## 对 Vue／Tauri／Browser 分工的建议

**建议保持一个共享 UI、两个节点 adapter。** 页面和操作流程只消费统一节点状态、错误、服务能力和完成事件；安装版调用原生 Rust 核心，普通 Web 调用本节点 Rust/WASM。安装版不额外启动一份 WASM 节点，Linux headless 也不依赖 WebView。这个结构与 Tauri 的 Rust core／WebView 分工相容，且 Tauri 前端框架无绑定；它不是本轮采纳框架的决定。[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)、[Tauri 前端配置](https://v2.tauri.app/start/frontend/)

文件字节、终端输出和 HTTP 数据需有专门的流式 adapter，避免把大数据变成响应式业务状态或 JSON 事件；浏览器中的 WASM／JS 边界与安装版 IPC 需要各自验证。能力可按模块分别表达“能使用／能提供／当前许可／需要用户动作”，不为共享界面强造各端对称能力。五个模块可在同一 Rust 库内保持清楚边界；本研究不引入运行时动态插件系统。

**真正可能影响外壳选择的风险**是：目标 WebView 的页面隔离／IPC 权限不能达到要求、关键站点只能靠某个外壳不具备的网络入口才能工作，或实测 WebView 加 UI 成本超出最终预算。**属于独立功能边界的风险**则包括：普通浏览器后台剪贴板、原始 TCP 监听、PWA 页面冻结、下载与持久化，以及任意站点透明兼容。它们普遍不会随 Vue 换框架而消失，应先以实际产品范围和最小验证澄清。

资料核对方式：本轮通过 Context7 检索 xterm.js 与 Tauri 官方文档，再对照官方网页／规范；浏览器 API 主要采用 WHATWG、W3C、浏览器厂商和 Mozilla 文档。文中的候选设计均未获产品选择，也没有新增已接受的 ADR。
