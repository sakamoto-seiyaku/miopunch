# MIOPUNCH-159 Linux 原生成员 TCP 首拨

Linux 产品端点在没有 `tcp_listen` 覆盖值时绑定临时 IPv4 TCP 端口，沿用显式 `tcp_direct_disabled` 和资源许可。安装 TCP 载体时保留调用方提供的 QUIC 传输配置。受限成员准入先交换 TCP 能力，通过既有监听端口观察、`MIOTCP01` 候选交换和双端互拨取得实际通道。融合端点验证原 QUIC 连接的新鲜 ACK，双方确认后持有通道并交付业务。现有 UDP 受限建路继续使用原接口。

新增的受保护能力阶段使用原生融合 ALPN `/4`。`MemberFusionAdmission::prepare_member_bootstrap_direct` 为显式融合提供尚处于准入阶段的准备入口，默认返回不支持。存续连接改善继续使用已有入口。首次联系沿用原调用期限、联系预算和既有 TCP 对端尝试所有权。

正式 `NativeClient` 的双 NAT 实验仅配置 MQTT 信令和 TCP STUN，关闭 UDP 数据及数据 relay。两个独立 Linux 网关采用真实 conntrack 和源地址转换，未配置静态端口转发。正向、反向均从默认通配监听端口观察映射并互拨，独立服务端读出 `first request`，调用端读回 `first reply`。两端原始 TLS 准入标识与业务连接一致，选中的 TCP 路径有新鲜 ACK 和实际 STREAM 帧。

- 正反向首拨、封锁 TCP、禁用 TCP、禁止观察资源、禁止目标地址共 10 组通过。许可拒绝组没有相应观察或互拨报文，失败组未交付业务且在原拨号期限内结束。日志在 `/tmp/miopunch-159-final-tcp-nat/`。
- 双端同时首拨的两组通过。每端两条业务流共用本端同一原连接，双方使用相同的两条流。信令联系地址来自实际 broker 订阅及获准设施的服务标识。日志在 `/tmp/miopunch-159-mutual-green/`。
- 既有受限 UDP 双 NAT 的正反向成功和阻断四组通过。日志在 `/tmp/miopunch-159-final-udp-nat/`。
- 共享结果检查将原 TLS 身份、准入、首笔业务和原流关联用于 TCP 与 UDP 两组实验。对上述既有日志的检查均通过。
- 普通库消费者 2 项、受限端点 19 项及受影响的 UDP-only active 用例 4 项通过。维护监听器的原 host exact 用例通过，日志在 `/tmp/miopunch-159-contact-fixture-host.log`。隔离 netns 下该旧夹具的入网前置超时，普通 host 下完成相同断言。
- TCP 物理断链用例显式提供原有的 10 秒 QUIC idle 配置，保留 13 秒断开、原流失败及新连接重拨断言，定向通过。日志在 `/tmp/miopunch-159-tcp-idle-fixture.log`。
- WASM 检查通过，回执在 `/tmp/miopunch-159-wasm-check/receipt.json`。

完整 root 测试的构建通过。直接运行全部测试时，多组宿主、剪贴板、IRC 等夹具缺少专用驱动提供的私有目录、relay 或服务环境。已保留原始失败并结束该次运行，回执在 `/tmp/miopunch-159-full-suite/receipt.json`。面向 UDP 的旧定向夹具在相应业务阶段明确禁用 TCP，保留原路径和业务断言。完整套件的原始运行尚未全部通过。独立 raw TCP 融合 allowlist 用例还可复现 `Explicit admission failed (closed path)`，日志在 `/tmp/miopunch-159-allowlist-exact.log`。该用例不消费应用资源配置，未进入新增受限联系或直接候选流程；未保留可执行的旧基线副本。

实验入口为 [TCP 双 NAT 驱动](../../scripts/check_member_tcp_nat.py) 和 [正式业务工人](../../tests/support/native_service_tcp_nat.rs)。驱动接受已构建的 `native_service_streams` 测试程序，在一次性 user、mount、network namespace 中运行。普通库消费者、受限端点和受影响业务使用既有测试入口。
