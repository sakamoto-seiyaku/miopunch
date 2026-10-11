# IRC CID 与受限确认：完整 SDK 实测

本轮对应 [MIOPUNCH-89](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/f0f95306-bafe-4bae-9a21-8784b0c55d82)，
比较基线 `05f99e3d078e17c69c53419faf681dec6cadec5a`。
实现与限制见 [实施记录](../../notes/2026-10-11-irc-cid-and-confirmation.md)。

| 公网完整 SDK 初次受限请求 | 实测秒数 |
| --- | --- |
| Base64 基线，两轮 | 60.488、59.986 |
| 仅缩减活动 CID，三轮 | 35.400、48.080、48.105 |
| 合并确认，收紧条件前的三轮 | 43.614、43.043、43.107 |
| 最终当前 SDK，显式 60 秒 caller | 43.081 |

最终请求相对基线平均值约快 28.5%，完整故事约 77.1 秒。公共网络的波动
仍可见，不能把仅 CID 和合并版的范围重叠解释为独立的确认收益。
自有慢设施另按“仅 CID、合并、合并、仅 CID”执行完整受限请求和自然清理：
29.139 / 24.109 / 23.640 / 28.642 秒。仅 CID 平均 28.891 秒，合并后
23.875 秒，少 5.016 秒，约快 17.4%；这是该自有设施下的两组重复测量。

公网均执行正常 TLS/WSS 和真实 HTTPS Origin 下的当前 Native/WASM SDK。
初次请求严格 version1/purpose2、预期 TLS 对端、受限 A、真实 Native 未批准
请求；实际审批、authorization 与 admission secret 保存、双方持久身份重开、
获准 relay 上 member1/fresh ACK/原绑定/普通 STREAM PING/PONG 和自然清理
均通过。初次请求只有 IRC 控制路径，IRC 没有被认证为普通业务路径。

最终 caller/attempt 为显式 60 秒，assembly8s、queue30s 和配置 fallback600ms
仍是比较用设置，实际双方 SAFERATE 生效。因此**未认证完整默认 IRC profile**。
最初 500ms WSS 观察为 false；最终公网追加约 2 秒、自有慢设施追加约
1.0–1.5 秒后，实际关闭事件在 context 销毁前到达。Native 自然退出、驱动为
零，没有靠强制结束或先销毁浏览器获得成功。

## 文件

- [公网各轮](public-cohorts.json)与[最终完整故事](final-public-story.json)：
  对等切点的请求末尾 frame/datagram 统计、累计 SDK 阶段计时和完整清理。
- [自有慢设施对照](controlled-confirmation-pairs.json)：相同固定镜像、TLS
  设施、CID2 和配置的 ABBA 对照。历史 SDK 绑定其原源码图，未标为当前构件。
- [当前六项 SDK 回归](final-affected-regressions.json)：无效预算、2s caller、
  长期监听、取消、禁用 IRC、MQTT；均通过自然清理。
- [当前构建](sdk-builds.json)、[41 项 SDK 测试](rust-regressions.json)及
  [432 项 Noq 测试](noq-regressions.json)：共享 target、固定 lock、构件与实际
  所选源码图；最终 Noq 图重核验未变，复用已有执行结果。
- [诊断与失败迭代](diagnosis-and-prior-runs.json)：先出现 stream index2 的
  红测，以及误选配置、错误负测夹具和关闭观察不足；失败回执保留，不重标为成功。
- [源码范围](owned-source-scope.json)：13 个最终文件、完整锁图不变、显式
  探针设置及 234 份既有 WIP 不变。
- [审查汇总](review-summary.json)与[双轴报告](review-report.md)：初版条件
  过宽的发现、修复后的独立报告，以及报告之后完成的实际资格检查。
- [探针源文件清单](probe-sources/manifest.json)：执行的 controller、helper、
  固定 selector、构建前置核验与源码/图核验，按 SHA256 保留。
- [输入复用说明](probe-input-reuse-notes.json)：执行时沿用的公开流量授权
  声明保持原字节；实际 60 秒预算和应用握手顺序变化以当前回执及源码为准。
- [生成材料清理](owned-generated-material-cleanup.json)：八个容器独立确认
  缺席后，删除八个生成私钥和 24 个自有 browser/NSS 目录；保留全部原始证明。

CID 默认上限仍是 5；缩为 2 时，七个未来路径的初始池由 35 降至 14，
八个并发路径槽位和轮换保留。自动调节只用于条件明确的独占 IRC 配置；
自定义 Native builder 保留调用者设置，公网探针显式选择 2。确认合并只用于
装有统一协调 hook 的新版 SDK 受限请求，成员证明和独立 gate 流程保留。
本轮没有修改 QUIC 拥塞或丢包/重传算法，PTO 等重复发送仍在。

gate 旧快照在统一确认前，新快照在统一确认后，不能把这些不同切点直接
比较。请求末尾是各轮相同的切点。frame 次数包含重发，不是不同 CID 数量；
Noq 的 UdpStats 在这里计数逻辑 QUIC datagram，实际载体仍为受保护 IRC。
计时包含 TLS/hooks/统一确认，不能当作纯 TLS 耗时或直接净省 22 秒。

公开内容仅为标量和 hash 白名单；原回执、源码副本、原始 depfile 与固定
SDK 构件保存在列明的私有会话目录。未复制实际身份、CID 内容、ciphertext、
grant、秘密、私钥、node store 或浏览器/NSS profile。M84/M69 的实机全矩阵
和默认 profile 验证继续由 Plane 记录，本组证据不替代它们。
