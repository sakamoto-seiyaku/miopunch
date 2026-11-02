# r_proxy Linux / browser HTTP demo

当前服务已完成 [MIOPUNCH-110 的 WebRTC 升级与跨网络验收](rtc/README.md)：
原 `u22s:18443` 入口 18 项检查通过。以下保留 MIOPUNCH-109 最初 HTTP 交付记录，
其构建和宿主实现描述对应当时版本；当前运行机制见上述升级证据和 demo 说明。

2026-10-21，按用户后续明确授权实现并部署基础 demo；任务为
[MIOPUNCH-109](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d694bf68-f4da-427a-9158-aaed804d2184)。
用户选择 HTTP。未创建公网隧道，不要求导入证书或改变浏览器安全设置。
此前取消的 MIOPUNCH-108 保持取消；这份证据对应新的 Vue/Rust demo。

最终运行使用本机桌面 Chromium 访问另一台主机 `r_proxy` 的
`http://192.168.4.14:18443`。浏览器实测 `isSecureContext === false`。
完整访问链接只保存在私密交付文件和宿主私有状态中，不进入本证据或 Plane。

## 实际通过的边界

[result.json](result.json) 包含 15 项通过、零页面异常：

| 边界 | 观察结果 |
| --- | --- |
| HTTP 浏览器节点 | 不使用 TLS/安全上下文例外，独立浏览器身份完成真实 precode 入网、保存和成员 RPC |
| 交付入口 | 无访问凭据的配置请求返回 401 |
| 文件 | UI 上传 100,123 字节，Native 完成持久保存与哈希校验，UI 下载后独立 SHA-256 一致 |
| 分块重试 | 相同分块可重试，内容改变的重试被拒绝 |
| 剪贴板 | Linux 无桌面文本缓冲往返；HTTP 手动复制实际写入浏览器系统剪贴板，并以粘贴读回确认 |
| TCP / UDP | 分别创建真实 Linux 监听，经过目标 socket 回显，核对返回字节 |
| 目标范围 | 提供端拒绝公网目标地址 |
| 网页隔离 | 父管理 DOM 不可读；网页 JSON 形似 RPC 的 POST 仍只是目标 HTTP 请求；无凭据不能消费邮箱 |
| 应用内 Web | 页面 Fetch、导航、表单 303 登录、节点侧 Cookie、100 KB 响应均成功；浏览器没有直接请求目标回环地址 |
| tmux / PTY | 两个工作区、真实命令执行、UID 1000、不能读取节点凭据；脱离再附着保留 shell 状态 |
| xterm | UI 键盘输入进入真实 PTY，返回的计算结果 `MIO_UI_45` 显示在终端 |
| 浏览器身份独占 | 同浏览器两个页面复用一个身份；脱离其中一个后另一个仍能通信 |
| 重开 | 身份、成员材料及已保存业务状态保持 |

截图：[应用内网页登录](connected-web.png)、[实际终端输入和返回](terminal.png)。
检查用浏览器、转发和 tmux 工作区已清理；部署的 demo 保持运行。

## 来源、构建与部署

[inputs.json](inputs.json) 固定源文件、验证脚本、产物和部署来源；
[manifest.json](manifest.json) 固定本证据文件的哈希。

- 最终 Native 图：Linux x86_64、Rust 1.98.1、已提交 dev profile、`rtc-probe`；
  527 个产物 Fresh，仅演示宿主目标重新编译。此前修订也只重新编译该目标，
  没有因收据目录或浏览器代码变化重建 SDK 依赖。
- Native 使用共享 `target/` cache lease，按 `pin_artifact.py` 固定不可变产物。
  部署程序来自独立副本；只转换该副本，未修改 pin。
- Browser SDK 复用 MIOPUNCH-103 合格产物，使用前验证 3,162 个选定输入、锁文件、
  清单及产物哈希。已有中继程序同样验证来源后复用。
- 未修改现有 SDK、Cargo 清单、锁文件、profile 或默认行为。HTTP SharedWorker
  是 demo 的显式宿主适配，不替换 SDK 默认的 Web Locks 存储实现。
- 远端容器为 `miopunch-demo`，重启策略 `unless-stopped`，独立状态与工作区，
  日志 `10m × 3`。最终逐文件部署哈希及 HTTP 健康检查通过。
- 运行镜像在本机从已有 Ubuntu 22.04 基础镜像构建后导入，约 115 MB。
  远端不安装 Rust / Node，不改其 Docker 网络设置。

原始收据、源快照、编译输出和早期失败记录保留在
`/tmp/miopunch-client-demo-20261021/`。最终检查目录为 `check-http-final/`；
完整访问链接文件为其中父目录下的 `entry-url`，权限 0600，不属于公开证据。
Native pin 保留在仓库 `.artifacts/`，来源见 `inputs.json`。

## 验证范围

这是 Linux 无界面节点与桌面浏览器的基础功能 demo，未完成完整客户端 PRD 验收。
HTTP 下不声明自动系统剪贴板访问，无桌面 Linux 不声明系统剪贴板。
终端是普通容器用户；浏览器不提供通用本机监听端口。
Web 的请求体/响应有界，目标 WebSocket、任意站点兼容、完整产品服务 ACL、
来宾、创世确认、Tauri 及 Android/桌面生命周期仍在本次验证范围之外。

运行与停止方法见 [demo 说明](../../../demo/README.md)。
