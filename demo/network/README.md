# 组网观测 demo

2026-11-17，按本轮确认的定位，先观察真实组网、直连／中继和换网恢复。
测试网络已有 `u22s`、`u22sx` 两个 Linux 成员、`u22sz` Windows 成员及本机
`Main (WSL)`。前三台处于同一个远端内网；Main 在本地 WSL 中独立入网。
当前只运行 u22sz → Main WSL：u22sz 发起请求，Main 接收并返回本地视图。
u22s、u22sx 已停止组网，保留 HTTP 宿主和原身份；成员总数仍为四。
剪贴板、Android 和独立 WASM 节点留在后续主线；本页是原生节点的本地诊断 UI。

## 使用入口

Main Windows 和 Windows `u22sz` 均在各自机器打开 **http://127.0.0.1:18460/**。
启动器用独立 Edge 应用窗口和专用浏览器目录；可直接手动重新打开这个地址。
两个远端 Linux 节点支持无界面运行，Windows 通过加密成员连接取得对端视图。
HTTP 只监听各自回环地址；SSH 用于部署及独立检查，不承载测试业务或远端视图。

| 节点 | 程序、配置与私有测试状态目录 |
| --- | --- |
| u22s | `/home/u22s/miopunch-network-demo/` |
| u22sx | `/home/zy/miopunch-network-demo/` |
| u22sz | `C:\Users\zy\miopunch-network-demo\` |
| Main (WSL) | 仓库内 `.artifacts/runs/network-demo-main/` |

Windows 可在任务计划程序按需运行 `miopunch-network-demo`，也可在桌面 PowerShell 执行：

```powershell
& "$env:USERPROFILE\miopunch-network-demo\start-windows.ps1"
```

Linux 可重新执行部署目录内的 `python3 start-linux.py`。启动器不配置开机启动。
Main 在 WSL 的仓库根目录执行 `python3 .artifacts/runs/network-demo-main/start-linux.py`；
Windows 浏览器可直接访问其回环入口，无需启动 Windows 原生节点或修改防火墙。
程序、身份和配置保留在目录中，节点日志为 `stdout.log`、`stderr.log`，
当前观测为 `status.json`。页面的“停止组网”关闭 SDK 节点，但保留本地 HTTP
宿主和页面轮询；刷新后仍可点击“开始组网”，用相同身份重新打开存储并恢复连接。
Linux 也可向本地 `/api/stop`、`/api/start` 发送带相同 `Origin` 的 POST。
向目录所拥有的宿主进程发送 SIGINT 才会关闭整个宿主，之后使用启动器重开。
停止保留身份、成员资格和联络配置。

## 看什么

每个节点向 `peers` 中配置的对端每秒发起一次真实请求／响应，复用持续的成员 QUIC 连接。
同一个连接上的路径变化与连接终止后的重新建连分别计数。对端返回其本地观测，
不会把第三方快照再次递归塞入回复。当前 Main 的 `peers` 为空，其接收证据可在
日志的 `network-demo-incoming-ready` 中查看；u22sz 页面显示向 Main 的业务状态：

- 当前选中路径与备选路径、IP／relay 地址、本地 socket、路径 RTT、收发量及丢包。
- 实际业务 RTT、最近成功时间、成功／失败计数、连接 ID、连接次数及恢复耗时。
- 节点自己的成员视图、维护状态、已知联络地址和 relay 注册状态。
- 路径切换、心跳失败、原连接修复及新连接建立的时间线；SDK 原始日志末尾。

恢复耗时从首次观测到不可用开始；首次建连单独保留在时间线中。
停止进程再启动属于同身份重连，不能充当同一连接换路的证明。
时长使用单调时钟；跨节点新鲜度分别相对其自身快照时钟计算，避免机器时钟偏差。

点击“标记一次手动切网”只写时间标记，不操作 VPN、不主动刷新网络或强制重连。
随后在系统里手动启停 VPN、切换网络，观察业务何时恢复、Stable ID 是否改变及
最终选中的路径。“导出当前诊断”包含公开节点标识、IP／端口、快照、时间线及
日志末尾，不包含节点私钥或邀请。候选地址或 relay 注册成功都不等于业务可用。
UDP/IP 直连可能经过现有 VPN；当前测试环境不能据此证明任意公网 NAT 都能穿透。

## 实现与验证范围

工具目标为 `miopunch-native-tools` 包中的 `miopunch-network-demo`。
复用 `NativeNetwork`、`MaintainedNetwork`、既有准入和成员历史同步、路径协调及
原连接修复。资源连接入口修正了将本地 relay 列表混入对端有效 relay 提示的问题；
未新增依赖或修改用户的 VPN 配置。
业务使用 SDK 的 `MEMBER_ALPN`，进入统一协调器的 `member-business` 流程。
Native 主动升级复用原业务连接 M 的身份、socket 和路径，独立控制连接 C 承载
协商消息，避免与 M 上持续打开的业务流争抢。失败回滚保留原连接和已有路径。
`unified-path-ready`、`unified-native-active-start`、`NAT_DIRECT_LOCAL/PEER`、
`NAT_CONNECTION_ROUND` 和 `UNIFIED_ACTIVE_CANDIDATE_FAILED` 可关联同一 M 的
Stable ID；`active-improvement-committed` 记录候选提交，最终选路仍以当前路径快照为准。
诊断请求只在真实操作期间持有业务优先级，两次采样之间允许正常维护。
初始联络提示沿用维护接口的八地址上限，完整地址仍供诊断查看。

`bootstrap NODE_DIR PRIVATE_INVITATION` 仅为全新测试网络提供 SDK 初始化和
短期邀请核销宿主；`invite NODE_DIR PRIVATE_INVITATION` 用已有测试管理员签发
邀请。存在获准 relay 时，邀请导出等待实际注册完成。
`join NODE_DIR PRIVATE_INVITATION` 通过真实准入协议加入；
`resume NODE_DIR PRIVATE_INVITATION` 用原身份续办未完成准入；
`run NODE_DIR CONFIG` 启动维护和诊断。测试初始化不代表正式产品的创建／恢复卡
交互验收，不导入现有产品节点，也不替用户确认恢复卡已经保存。

`CONFIG` 包含 `name`、回环 `listen` 和 `peers: [{name, address}]`。
`address` 使用 SDK 的 `EndpointAddr` 表示；联络提示不授予成员资格。
修改初始配置后需要停止并重新开始节点；正常换网由运行中的 SDK 处理。

构建沿用 `scripts/build_native_client.py` 的冻结输入、共享 Cargo lease、固定
Windows 工具镜像及不可变产物；选择 `--bin miopunch-network-demo`。
运行使用独立部署副本，未修改共享 pin。

真实入口回归曾复现自定义 ALPN 绕过统一协调器，以及主动升级协商抢占 M 业务流。
修复后，本机真实节点持续六次请求仅建连一次，统一流程与业务使用相同 Stable ID，
Native 升级提交期间新旧业务流均可使用。核心 48 项、Native 网络 5 项、受限准入
11 项、带真实 MQTT broker 的维护 11 项及物理 UDP socket 三项针对性回归通过。
构建前置与清理的既有七项回归检查通过，Linux 和 Windows 冻结输入构建通过。
Main Windows 回环访问 WSL 页面成功，Main 的停止、刷新、开始已通过浏览器验证。
u22sz 也已在本机 Edge 中完成停止、刷新、开始：身份与宿主 PID 保留，重新开始后
恢复向 Main 的真实请求，页面脚本错误为零。
当前修复版已部署两端：u22sz 持续向 Main 请求，同一业务 Stable ID 保持，统一
成员准入和后台 Native UDP 尝试均有实机日志。当前选中 APS relay；升级超时后
原业务继续。Main 的完整地址快照仅有私网地址，STUN 观测超时；发送给对端的
直接候选按既有 LAN 优先规则取前 64 项。u22sz 获得公网 IPv4/IPv6 候选，STUN
对两个目的地观测到不同映射端口。以上事实尚不能证明公网 UDP 穿透成功或不可能。
relay 选择回归先复现超时，再通过真实 relay 上的准入验证；资源测试 30/31 通过，
既有 p2panda 高层 mDNS 发布测试单独运行也超时，尚未解决。
实际手动启停 VPN／切网后的恢复速度尚待用这个运行中的 demo 观察，
这些基线结果不构成其完成证明。
