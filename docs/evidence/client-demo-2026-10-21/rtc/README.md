# HTTP demo 的实际跨网络 WebRTC 验收

2026-10-21，按用户要求启用并实际测试 WebRTC，任务为
[MIOPUNCH-110](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/3a9c20c1-8f4a-47d6-9d5f-e73d7f4a739e)。
原入口 `http://172.16.3.11:18443` 已更新，内容端口仍为 18444。
访问凭据、原 Native 身份、用户上传文件和剪贴板保留，更新前后选定哈希一致。

[上线结果](result.json)包含 18 项通过、零页面异常；
[独立候选检查](candidate-result.json)也全部通过。
浏览器实际运行在 u22s，Linux 节点运行在 r_proxy；普通 HTTP、无安全上下文或
证书例外。ICE 记录显示两个不同公网地址之间的 UDP srflx 路径，没有 TURN。

| 验证 | 实际结果 |
| --- | --- |
| WebRTC 数据路径 | Browser 与 Native 普通成员会话均选中 RTC；同一 Native stableId 的业务流量计数增长 |
| 排除中继代传 | 关闭浏览器中继 WebSocket，禁止重连；同一成员连接仍返回正确文件字节 |
| 五项业务 | 100123 字节文件上传/下载哈希、实际手动剪贴板复制粘贴、TCP/UDP socket、隔离 Web 导航与登录/Cookie/100KB 响应、真实 PTY/tmux 与 xterm 输入均通过 |
| 身份与持久性 | 两页面复用同一节点；脱离一页后另一页仍可用；全部脱离后重开保留身份与成员资格 |
| 终端回归 | 历史 PTY 的旧设备查询曾将响应误输入 shell；[修复前](terminal-before.json)失败，[修复后](terminal-after.json)真实键盘执行得到 42 |
| 能力及宿主退出 | [检查结果](lifecycle.json)：缺少 RTC API 明确报告 api-missing，成员 RPC 可回退；拥有 SDK 的 Window 丢失后其他页面停止访问并拒绝接管 |
| 已有维护入口 | [固定测试产物运行](maintained-runtime.json)5 项通过；不把其中未验证 RTC 数据的检查算作本次 RTC 成功 |

截图：[应用内网页](connected-web.png)、[真实终端](terminal.png)。
临时浏览器、候选容器、测试网关和测试隧道已停止；
[最终健康检查](health.json)确认交付服务与原 u22s 转发仍正常运行，部署哈希一致。

SDK 在唯一 Window 中运行，固定 SharedWorker 仲裁该宿主。更新后需要关闭全部旧演示
页面，再用原完整链接打开。直接关闭实际宿主 Window 时，其余页面需全部关闭后重开；
本次没有实现自动宿主接管。缺少 SharedWorker 的浏览器仍会拒绝该 HTTP demo。
终端历史回放完成前不转发终端响应或用户输入，随后才启用实时输入。

[inputs.json](inputs.json)记录源文件、Native/WASM 产物来源、部署哈希及保留检查。
正常 Native 图使用共享缓存，526 Fresh，仅本包库、demo、中继三个目标重新编译；
WASM 保持 M103 固定产物，并验证 3162 项原始输入。NativeNetwork 新增工厂公开既有
active 控制处理器供应用 Router 使用；未改变 SDK 默认策略、Cargo profile 或锁文件。

一次根包 integration-test 构建意外带出自动二进制和 dev-feature 图，
[调查记录](dependency-rebuild-review.json)保留了原因与空间变化；
[构建收据](maintained-build.json)记录 523 Fresh/22 重编。调查后运行固定 test pin，
后续驱动只校验并复用产物。没有清除缓存或因重试重复构建。

原始构建、旧版源码快照、红绿检查和部署回执保留于
`/tmp/miopunch-client-demo-rtc-20261021/`；公开证据不含访问令牌或 SDP 凭据。
旧应用程序备份保留于 r_proxy 的
`/home/js/miopunch-demo-before-rtc-20261021/old-app.tar.gz`，未复制私有节点状态。

验证范围仍是 Linux 与桌面浏览器基础 demo；完整产品 ACL、来宾、创世确认、
任意目标网站/WebSocket 兼容、Tauri 与 Android 验收尚未完成。
运行方式及边界见 [demo 说明](../../../../demo/README.md)。
