# 原生本地 socket 身份与真实地址分离

2026-10-12，用户在接口评估后授权增加必要 API，以现有 demo 正常运行为本切片验收重点；尽量让普通旧调用方无需修改。这里记录该授权对应的局部决定，不将先前研究候选整体定案，也不扩大为全部上游接口兼容承诺。

Noq 的底层 I/O 与协议路径显式认识本地 `LocalSocketId`。该标识是端点内的不透明本地元数据，与 QUIC `PathId`、文件描述符、源端口和 IP 地址不同，不进入 QUIC 报文。真实地址继续保留地址含义，不将 socket 身份编码为虚构 IPv6 地址或新的 custom carrier。

实际描述符继续由现有 iroh socket pool 管理；不建立第二套描述符所有者。pool 分配标识、收包时报告实际 socket，路径和排队的发包结果保留标识，发包时由该标识选回原描述符。关闭、源 IP pin、原连接所有权、轮次与权限检查继续约束同一资源；标识本身不授予权限。旧默认 I/O 使用未指定的标识，继续允许已有抽象 socket 的正常替换行为。

保留旧 `Transmit` 的公开字段、普通构造器及普通 socket trait 必需方法。新增 socket-aware 接收、发包结果和显式 socket 建路 API；未实现该能力的 sender 拒绝显式绑定，不默默改用默认 socket。显式使用底层绑定的驱动须采用新发包接口，使等待可写期间的字节保持编码时选择的 socket；旧底层 `poll_transmit` 对该 opt-in 情形明确拒绝丢弃绑定。现有 demo 的 iroh 建路入口和公开 transport 地址形状继续保留。

一个保留的 QUIC PathId 保持其本地 socket 绑定；收到另一个 socket 的报文不能接管该路径，换 socket 必须在原会话上显式建立另一条路径。远端地址迁移继续遵守原规则。与绑定相关的探测同时记录现有路径 generation，远端地址迁移后不能沿用旧探测证明；新 socket 不从同地址的其他 socket 继承路径验证。业务准入和获准路径仍按 [ADR 0009](0009-unified-session-and-capability-based-paths.md) 执行。

验证与适用范围见 [实施回执](../notes/2026-10-12-native-socket-api-and-demo.md)。本决定不改变密码握手、QUIC 线格式、拥塞算法或成员资格规则；M69/M84 的完整公网和实体设备矩阵独立验收。
