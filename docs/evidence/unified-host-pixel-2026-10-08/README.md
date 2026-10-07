# Host / Pixel 完整 SDK 回执

[full-sdk.json](full-sdk.json) 记录 Linux Host Native 与 Pixel Android Browser 的实际完整 SDK 故事。Browser 发起连接；应用 PING/PONG 在原 M 上双向传输。这不是 Native 发起方向的证明。[results.json](results.json) 保留此前只验证准入、重开与业务的首个成功切片，未增加该次运行没有执行的副本结果。

完整故事通过公共 EMQX 的正常 TLS/WSS 信令，沿用一个已配置 STUN 服务且未配置 TURN。错误 TLS 身份 claim 被实际 Native provider 拒绝，账本未改变；未批准节点的业务和副本操作被拒绝。准入在只保存授权、尚未保存 secret 时中断，同身份重开后继续原 grant，随后两种材料均保存成功。

独立 SDK 作者生成实际非空签名控制记录后退出，provider 和 Browser 都重开持久化目录。Browser 通过严格新版 Replica 用途同步、验证、保存并应用 4 条记录，包括此前没有保存的离线作者记录；再次重开后目标记录仍存在。随后新版成员 gate 与 fresh ACK 放行原 M 的 RTC 业务。实际移除成员后，新业务和副本请求均被拒绝。整例约 61.98 秒，Replica 调用约 7.89 秒，普通成员建连约 7.58 秒。

Native fixture 曾在接受 durable receipt、发送最后确认后立即关闭 M，导致 Browser 的同步调用失败。本次修正让已排队确认继续传输，在原 deadline、取消或 endpoint 关闭范围内等待连接结束。`replica-finished` 报告 durable receipt，`replica-ended` 独立报告退休结果；完成事件不再等待退休。SDK 的协议、RTC 算法和应用预算没有因此改变。原失败及随后观察器错误均保留在回执所列私有原件中。

Native 使用固定 ELF，构建仅重编译一个 bin；6578 个实际编译输入、291 份仓库源码副本和 477 份捕获的 depfile 经构建前后校验。Browser 复用当前固定 WASM/package。预码短回归使用原固定 selector 的版本、来源、checksum 和 lockfile，无重新解析，验证同 grant 续传与重复使用约束；其 5716 个编译输入在最终 fixture 修改后仍字节相同，该 fixture 不属于这个测试图。相关 Python 检查共 19 项通过。

两端在 SDK 启动前开始有限接口抓包，原始数据仅留在私有文件。选中 tuple 上的双向完整 ICE 请求 payload 匹配通过；这是两端包关联，并非另一次消息完整性密码验证。Native 的 PID、FD、inode、socket UID、SO_MARK 与存活原 M 对应，选中路由使用 Host Linux 的默认接口；Pixel 的选中路由也已读取。所有接口包并不都归属于 SDK。

物理公网资格保持 **false**：Host 的 Windows 上游路由未知，且本次 Pixel 活动网络回执没有显示 `VALIDATED`。不把 Linux 路由、两端包关联或 SDK 业务通过换算成无 overlay 的公网 NAT 成功。OP13 未使用，透明代理问题按用户要求暂缓调查。该回执也不证明完整四方向矩阵。

Native 自然退出、即时与晚期进程不存在、读取线程结束；Browser SDK、PC、pump、存储、页面和自有转发均回收。抓包自然停止且自有进程、容器和目录清理成功。源码、输入与构件在清理后再次核对；公开回执仅保留标量事实、计数和 hash，不发布原始身份、地址、SDP、exporter 或秘密。
