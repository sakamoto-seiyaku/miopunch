# 原生 socket API 与现有 demo

用户授权增加必要 API，停止继续扩大接口选择讨论。对应决定见 [ADR 0011](../adr/0011-native-local-socket-identity.md)，实施切片为 [MIOPUNCH-90](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/52f82fd5-72f9-4046-ab7d-c12a5cb8a223)。

## 实现

Noq `LocalSocketId` 在 I/O 收包、协议 FourTuple、实际 PathData、发包结果和等待发送的字节之间传递。真实源 IP 保持原含义。iroh 使用已有 physical socket pool、原连接权限与退休机制，不再生成物理 socket 的 IPv6 token；公开 transport FourTuple 与现有 demo 建路入口保留。

新 API 包括 `FourTuple::with_local_socket`、`poll_transmit_with_socket`、`Connection::open_path_on_socket` 和 `open_path_ensure_on_socket`。普通旧构造器和 sender/receiver 实现使用默认 I/O，不增加必须实现的方法。显式使用新绑定的底层调用方必须使用新发包接口，支持能力和实际资源存活均会检查。

路径选择保留选中 candidate 的 socket 身份，新增 `PathSelectionContext::is_current` 可区分地址相同的 socket。原 `current()` 和 `network_path()` 返回形状保留。探测记录路径 generation，迁移后的旧探测返回失效。

## 首次失败与修复

新增协议用例复现了同地址、不同接收 socket 未更新路径绑定的问题。首次测试还使用了不允许本地迁移的 server 角色；保留该回执，随后改用允许本地迁移的 client，仍在相同断言失败。初版修复使 socket 变化进入路径迁移，434 项通过；独立审查随后指出这会让 iroh 的路径缓存保持旧 socket。最终收紧为本地 socket 绑定不可被收包接管，换 socket 须在原会话上建立新路径；远端地址迁移继续使旧 generation 探测失效。初版结果保留，不重标为最终结果。

## 构建缓存

本切片所有构建经受源、lock、工具和保留构件前置检查，使用仓库共享 `target/`；输出目录仅保留回执和少量选定构件。首次 Native library 构建复用 520 个构件、重建 7 个；修复后的 Noq 构建复用 145 个、重建 1 个。没有重新解析 lockfile、复制 Cargo target 或改变 profile。

空间不足来自既有缓存占用，包括 `/tmp` 下的历史独立 target。本轮仅按已有清理入口删除经进程检查的历史 incremental IR，释放 20,682,448,896 字节（约 19.3 GiB）；保留原始证据、固定 SDK、库、可执行文件、依赖信息和工具链。后续其他空间变化不计为本轮清理收益。

## 验证

最终 Noq 435 项与受影响 SDK 81 项全部通过，共 516 项。后者包含 11 项实际物理 socket 用例，涵盖原连接上的双向 stream、真实源 IP、跨 socket 回执拒绝、TTL 隔离、descriptor 退休、取消、256 个监听释放和网络变化；也涵盖普通成员连接、审批与材料保存、持久恢复、TCP、RTC 与撤销/取消。新增协议用例使用真实密码报文和合成 I/O 元数据，验证同地址不同 socket 独立建路、错 socket 不消耗报文编号且同一报文在正确 socket 可接收、远端地址迁移使旧 generation 探测失效；没有宣称运行实体转发代理用例。

最终 Native 和 WASM 源/lock/工具/构件核验及构建通过。Native 复用 529 个构件、重建 0 个；WASM 复用 291 个、重建 5 个。真实浏览器与 Native 的完整 demo 在自有 localhost IRC TLS/WSS 和 HTTP relay 上通过：受限请求、一次真实审批、保存授权与 secret、双方重开同一持久身份、member1 业务 stream PING/PONG，以及正常关闭和资源释放。relay 外层 HTTP 与私有测试 CA 是受控设施范围，端点间 QUIC/TLS 和准入检查仍实际执行。

该 demo 显式使用 caller/attempt 60 秒、pacing 10 毫秒、assembly 8 秒和 queue 30 秒；不认证完整默认 profile 或 M69/M84 公网/实体平台矩阵。首次受控运行在发起请求前因沿用公网性能探针的 SAFERATE 断言失败；该设施实际 ready=true、SAFERATE=false。保留首份失败，第二次只修正控制器 readiness 断言及自有设施 pacing，使用相同 SDK 构件通过。

提议的公网复测被自动审批拒绝，原因是当前可见记录缺少向指定公共 IRC/relay 发送具体身份、准入材料和业务数据的授权；没有启动公网 actor，也没有绕过拒绝重试。受控完整 demo 满足本切片范围。两轴独立审查发现的 socket 缓存问题已修复，最终均无未解决发现。

源/构件/退出回执、失败记录、审查和精确控制器见 [证据目录](../evidence/native-socket-api-2026-10-12/README.md)。所有固定 SDK 与原始证据保留；确认容器及 host 进程缺席后，删除了 4 个生成的测试私钥和 2 个私有 NSS 数据库。246 份既有 WIP 保持不变。
