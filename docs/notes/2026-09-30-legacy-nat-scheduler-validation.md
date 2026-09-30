# 旧 mode0/1/3 完整执行验证

2026-09-30，对应 [MIOPUNCH-63](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/f943cc89-199e-447f-b353-a0bead63892a)。实现依据为[旧行为迁移裁决](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md)，承接 [62 的真实特征、完整选择及保护计划](2026-09-30-legacy-nat-selector-validation.md)。这次交付完整单物理 socket 执行；模式 2/4 的随机及监听池仍由 61 接续，正式 auto 入口仍由 60 接续。

## 实现

[`nat_scheduler.rs`](../../src/nat_scheduler.rs)直接适配旧实际 MakeHole 的阶段语义。范围取对端最后样本 p 和旧特征跨度 d，闭区间 `max(p-d-5,p-radius,1)..min(p+d+5,p+radius,65535)`。radius10 最大 21 个端口，radius2 最大 5 个。sender 显式目标先 assisted 后原 candidate；receiver 有范围时不另发 explicit candidate。只合并相邻重复地址/IP；不排序、去掉非相邻重复，或改写原样本。

接收注册在本端延时前完成，期间能够选中胜者。延时后立即首次 burst；范围仅首轮按 IP 顺序、端口递增发送，每包后 2ms；之后只按 200ms 重复显式目标。sender 忽略 request；receiver 对 request 回复三次，间隔 50ms，默认 TTL。总阶段预算是双方延时最大值加 5000ms，各自 read budget 扣自己的延时。范围、延时和角色由 62 的真实 mode/index 参数直接进入执行，没有固定 server-first、单 next-port 或小预算替代。

逐包 TTL 通过 vendored netwatch 0.19.3 的同 socket 发送接缝实现：Linux/Android 使用 `sendmsg IP_TTL` ancillary metadata，不改共用 socket 的默认 TTL；不支持的 metadata 使用默认 TTL best-effort 并记录 degraded。其他平台走默认 TTL 降级，尚无本票平台抓包实证。原 QUIC/QAD sender 不变。完整迁移接口与 fork 依据见 [`MIOPUNCH-PATCHES.md`](../../vendor/netwatch/MIOPUNCH-PATCHES.md)。

[`Attempt::execute`](../../src/nat_punch.rs)绑定原 ControlSession 的实际 scope、网络/socket 代际、round 与 stable ID。raw 注册容量 32，lease drop 同步撤销；独有 live 锁让关闭与每次实际提交互斥，旧 sender clone 不会因相同 round 重新注册而复活。提交在 actor/ready 等待后再次核验资格；延时及 burst 期间还持续守护 scope/连接状态和总预算。所有任务由执行 future 持有，没有 detached loop。禁止 IPv4 transport 的 Endpoint 在注册前拒绝。

raw 请求/回执使用原 Connection TLS exporter、fresh round 和发送节点方向的 keyed BLAKE3 MAC；旧 round、别的连接密钥、反射请求不能选中胜者。实际收到的 remote source 回填给原 Connection UDP 路径确认。raw winner 仅是临时遍历结果；正常路径 commit 和获准业务收讫后才允许一次反馈。失败、取消、额度耗尽、撤销、网络/凭据变更均不能借另一个已有路径记成功。candidate-only fallback 有完整默认 TTL、无延时的五秒阶段，但没有推荐记录，因此成功也不加分。显式保留的 execute_partial 只供原 62 协议回归，本票 legacy 探针调用完整 execute。

## 实网验收

全部原始证据、实际 fixture、三侧抓包和独立规则/conntrack 见 [`mio63-legacy-scheduler`](../evidence/mio63-legacy-scheduler/README.md)。最终 [`kernel-verification.json`](../evidence/mio63-legacy-scheduler/verification/kernel-verification.json)要求完整 74 个 case cells、精确退出/失败原因、身份配对、每端唯一原 stable ID、完整 pcap records/capture drain，以及真实目标、TTL、节奏、预算、winner 和 selected PathId。74 是验收数量，不是连接成功数。

| 已冻结条件 | 验收格数 | 原 Connection 业务 | 完整有界未命中 | 准入拒绝 |
| --- | ---: | ---: | ---: | ---: |
| 双 EIM/APDF，mode0 全 12 index × 两方向 | 24 | 24 | 0 | 0 |
| 规律 Hard×Easy，mode1 全 6 index × 两方向 | 12 | 12 | 0 | 0 |
| 双规律 Hard，mode3 全 6 index × 两方向 | 12 | 7 | 5 | 0 |
| 双不规律 Hard，旧回退 mode0/1/3 × 两方向 | 6 | 2 | 4 | 0 |
| 递减、非单调、spread5、端口 1/65535 × 两方向 | 10 | 10 | 0 | 0 |
| 缺分类样本 candidate-only fallback × 两方向 | 2 | 2 | 0 | 0 |
| 完整 mode3 WAN 阻断 × 两方向 | 2 | 0 | 2 | 0 |
| TTL metadata 不支持、默认降级 × 两方向 | 2 | 2 | 0 | 0 |
| 非成员、已撤销、无秘密、错秘密 | 4 | 0 | 0 | 4 |
| 合计 | **74** | **59** | **11** | **4** |

两侧 WAN 入站 raw PREROUTING 在 conntrack 前丢弃 TTL<8，保留旧 Internet 死亡条件。单侧范围实测涵盖 17、15、21 及边界截取端口；默认 response 和 QUIC/QAD 均为正常 TTL。低 TTL 原始包创建本地 NAT 状态，未出现在对侧 LAN；普通 TTL request 及三次回执才能继续原连接。WAN-blocked 用例证明两方向 DROP 计数和零 peer-LAN delivery，其 packet 在 WAN 已被拦，因此下游 raw TTL counter 正常为零。每格先独立证明 wrong port、wrong IP 与正确 reply 的过滤结果，不能用拓扑标签喂 selector。

模式覆盖通过实验专用 previous-selection 钩子驱动真实 Analyzer 的 first-max/减分历史，完整计划仍经双方校验。每格同时要求实际 mode/index 和钩子回读；它不强制场景外 mode、不改目标分布或参数。困难场景的 11 次完整 miss 保留全部角色与成本，未追加样本或修改条件追命中。规则、源码、driver 和两个 binary 在长实验前冻结，每格前后核验 hash。

首轮冻结曾误取被并行测试编译覆盖的无钩子 binary，导致请求的非零 index 实际仍为零。严格检查器发现后停止、保留为 [historical-wrong-feature-runtime](../evidence/mio63-legacy-scheduler/historical-wrong-feature-runtime/README.md)，不计 NAT 失败或覆盖。纠正后在所有测试完成再显式 feature build、立即 snapshot/hash，并逐格检查真实选择；最终运行未改文件或重启追结果。

## 回归与评审

两项公开 PhasePlan literal 测试覆盖跨度 0/1/5/>5、非等差/递减最后样本、闭区间 1/65535、候选/assisted 相邻 compact 与 total/read budget。十五项[真实 Endpoint 协议回归](../../tests/legacy_nat_runtime.rs)保留 62 的七项，并新增延时前命中且零 probe、延时前取消/lease 清理、期间撤销/换网、额度耗尽后原 control 仍往返且不学习、错 round/别的连接密钥后仍可正确命中、近 observation 期限的 fallback 保留，以及同 Endpoint 共用 socket 的另一个原 Connection 在 normal/degraded TTL 期间完成 256KiB 获准业务。

专用真实 MQTT broker 的完整 [`cargo test --locked --workspace -- --test-threads=1`](../evidence/mio63-legacy-scheduler/verification/cargo-test.log)：**218 passed、3 既有 ignored、0 failed**，没有 MQTT 跳过。`cargo check --locked --workspace --tests`、fmt 与差异空白检查通过。十四项[检查器故障注入](../evidence/mio63-legacy-scheduler/verification/verifier-negative.json)均精确拒绝，包括缺样本、错误退出/无关运行错误、空或截断 pcap、错误 mode/TTL、过滤证明、身份与清理矛盾。

[独立双轴评审](../evidence/mio63-legacy-scheduler/verification/code-review.md)由 GPT-6.1 Sol high 执行：Standards 与 Spec 均独立复核全部 74 格，最终无剩余问题。早期 lease 等待后发包、Tokio nested I/O、fallback 生命周期与 checker 的 fallback/WAN 判据问题已修正并复审。所有本票容器与 broker 已停止。

保留用户原有 dirty 文档与 59 的未提交脚本/证据。59 的两个重叠源码仍在 `/tmp/mio59-preserved-before62-2026-09-30/`，未应用；后续恢复必须基于当前 62/63 提交三方适配，不能覆盖新执行器。本记录没有把 61 的多 socket/原路径 winner 接缝、60 的正式入口或运营商平台验收写成完成。
