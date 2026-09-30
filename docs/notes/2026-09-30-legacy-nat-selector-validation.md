# 旧 NAT 特征及完整评分选择器验证

2026-09-30。对应 [MIOPUNCH-62](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7985934c-21a1-41a7-a717-6f097b1dada8)。本记录交付特征、完整候选/评分和受保护连接闭环；完整打洞执行由 63/61 接续，正式入口由 60 接续。用户裁决及旧源码逐项依据见[迁移对齐契约](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md)。实施状态以 Plane 为准。

## 实现与边界

[`nat_selector.rs`](../../src/nat_selector.rs)保留至少两个有效映射的 Easy/Hard、四种变化行为、仅 port-change 跨度 1..5 的规律特征，以及真实本地接口 IP 匹配的 PublicNetwork。规律不要求等差、单调或有符号 next-port；映射也不证明 APDF。六种场景保留完整 mode/index 插入表及所有 30 对参数，推荐 first-max 后减 1，无 -10 下限；成功加 2、封顶 10。原始映射顺序不被执行目标选择改写。

进程内共享 Analyzer 的内层 key 保留有方向的双方首映射 IP、类型、行为和规律，不含端口、跨度或 PublicNetwork。外层取实际已认证节点、网络、秘密代际/凭据标签、授权视图代际及 UDP/TCP；闲置严格大于两小时才清理，每十分钟机会清理。端口/span/PublicNetwork 改变保留学习，身份、网络、秘密、授权或协议改变隔离学习。凭据标签仅作本地作用域隔离，不经信令发送。

[`nat_punch.rs`](../../src/nat_punch.rs)通过已完成准入证明、当前双方成员资格的原 ControlSession 交换 Hello/Challenge、新 round、双方原始 snapshot、完整 Plan、Start/Ack 和 Established 收讫。错网络/旧 round 在披露接受端 snapshot 和创建评分记录前拒绝。准入拒绝发生在额外 S 采样之前。QAD adapter 允许 2..64 个实际不同目的；本地接口信息来自 Endpoint 的 netmon。新快照的 60 秒有界寿命允许旧 3/5/10 秒延时及 38 秒计划，网络/socket 代际变化仍立即失效；原预测器的两秒 `is_fresh` 保留给历史实验入口。

缺分类样本时保留默认 TTL、无 delay 的 mode0 基础计划：双方有出站目标默认 c sender/v receiver，仅 v 有出站目标才交换；双方无目标 disabled。这里 c 是接受端、v 是访问端。仅 assisted 的快照同样携带实际 Endpoint 代际，receive-only 一侧不因无出站目标而拒绝整个计划。63 将恢复其完整五秒调度。

显式入口为 `MIOPUNCH_NAT_STRATEGY=legacy`。当前 mode0/1/3 使用已有原 Endpoint socket 的一次受门控 UDP prime，明确报告 `PartialExecution`；尚未恢复 range、逐包 TTL、delay、200ms repeat 和旧请求/回执调度。mode2/4 保留 random1000/listen256 等完整计划，明确 `ExecutionUnavailable` 且零 peer prime，不用一次抽样冒充完整旧 mode。既有 `auto` 留作 56–59 的冻结实验基线，60 接正式完整入口。

prime 本身不认证身份或授予业务权限。成功仍须原 Connection 的当前已验证、已选中目标路径、正常 data-path commit 及获准业务确认；反馈绑定原 stable ID、当前作用域/代际，并仅消费一次。未执行、额度耗尽、取消、撤销或秘密变更，即使存在另一个已验证路径，也不能记这轮成功。

## 验证证据

所有原始记录在 [`mio62-legacy-selector`](../evidence/mio62-legacy-selector/README.md)。十二个最终内核用例的独立核验在 [`kernel-verification.json`](../evidence/mio62-legacy-selector/verification/kernel-verification.json)，通过数是分类/计划/身份/生命周期/失败验收数，不是十二次打洞成功。

| 实际条件 | 拨号方向/数量 | 结果 |
| --- | --- | --- |
| 双稳定 EIM/APDF | a、b 各一次 | 真实 Endpoint S→Easy→完整 mode0 列表/初选0/0→安全交换→各一次 partial prime→原 Connection 业务；协调端 +2 |
| `[42031,42033,42032,42031]` 对稳定端 | a、b 各一次 | Hard/PortChanged/span2/Regular；mode1/0、完整 1→2→0 列表及角色交换；partial executor 有界未命中、原控制存活、不学习 |
| 映射 IP 在 `.2/.10` 间改变、port 不变 | a、b 各一次 | Hard/IpChanged、无规律/span0；完整 2→1→0 列表；双方 mode2/0 `ExecutionUnavailable`、零 peer prime |
| WAN 阻断、错误路径目标 | 各一次 | 无业务/学习，保留原受保护控制失败握手 |
| 非成员、已撤销、无秘密、错秘密 | 各一次 | 无增强 S、无 Plan、无 peer prime，无业务或学习 |

[`legacy_nat_verify.py`](../../scripts/legacy_nat_verify.py)逐条把 S 的映射/目的与 WAN 实际 UDP 发包及 observer 响应匹配，核对初始列表/分数、round、每端唯一 stable ID、明确执行状态和失败结果。Linux `any` 抓包可能在两个接口重复看到同一包，因此 `peer_prime_capture_occurrences` 不能当应用发送数；一次提交由日志与真实预算回归共同核验。故意删除一条 S 的[检查器负例](../evidence/mio62-legacy-selector/verification/verifier-negative.json)被拒绝。

[`legacy_nat_runtime.rs`](../../tests/legacy_nat_runtime.rs)七项真实 QAD/TLS/UDP 测试覆盖两个目的/双方公网初选0/2、原连接实际业务及一次反馈、超过两秒仍可用但换网失效、取消/撤销零预算、错网络/旧 round 不创建评分、错 gate/无目标 disabled、assisted-only receive-only 及实际业务、额度拒绝或秘密变更后拒绝学习。这些是协议接缝测试，不证明 NAT 成功率。

十二项新增算法回归覆盖分类跨度0/1/5/6、反转/非等差、IP/Both 变化、六场景至少两遍轮转、PublicNetwork c 优先、分数到 -15、+2/cap10、未知 index 时间刷新、端口/span/public 缓存复用、两小时边界及每个作用域维度。所有 30 对参数另与[直接执行旧 Go 源码导出的 oracle](../../tests/fixtures/legacy_nat_behaviors.json)比较；[导出脚本](../../scripts/legacy_nat_reference.py)仅适配包/泛型依赖，不改旧算法。

## 检查与评审

`cargo check --locked --workspace --tests`、`cargo fmt --all -- --check`、Python/shell 语法及差异空白检查通过。专用真实 MQTT broker 下完整 [`cargo test --locked --workspace -- --test-threads=1`](../evidence/mio62-legacy-selector/verification/cargo-test.log)：**208 passed、3 ignored、0 failed**，未跳过 MQTT 用例。三项为既有一个 flaky gossip 测试和两个授权文档示例；broker 和本票 Docker 资源已清理。

GPT-6.1 Sol high 独立 Standards/Spec 双轴评审及复审见[评审记录](../evidence/mio62-legacy-selector/verification/code-review.md)。首次评审发现的 fallback 默认角色、assisted-only 代际绑定已修并有红→绿/真实端点回归；内核 IP-change 用例还复现 unavailable executor 过早关连接，已加入 Established 收讫后通过。复审无剩余具体问题。历史失败完整保留，未计入 NAT 负例或最终通过。

## 后续接缝

63 使用 `Attempt::plan/local_behavior/remote_feature/mapped_candidates/assisted_candidates`，保留原样本及完整对端参数，并扩展同一作用域/Connection/round 的实际 executor；不能用当前 partial 状态当完整 mode。范围、预算、接收优先、逐包 TTL 和请求/回执按对齐契约及 63 票据恢复。61 再增加同一 Endpoint 内多物理 UDP socket 及原 Connection 精确路径路由；60 才接正式入口、重新采样和有界重试。当前证据仅为受控 Linux 内核，不代表任意运营商/设备或 Windows 验收，也不证明完整旧困难 NAT 执行已交付。

59 的未提交源码只为必要隔离保存到 `/tmp/mio59-preserved-before62-2026-09-30/`（完整副本、原 HEAD binary patch、SHA-256），未恢复或计入本票。其脚本/证据和用户未提交设计文档保留。恢复须在后续单票代理根据当前提交进行三方应用，不能覆盖 62/63 的新源码，也不能修改冻结 59 试验的归属。
