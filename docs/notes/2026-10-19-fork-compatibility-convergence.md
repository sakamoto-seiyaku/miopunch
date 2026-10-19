# 固定原版消费者与当前 SDK 的最终汇合

2026-10-19，MIOPUNCH-106。运行源码固定于 `5302ac8a11a8363d530c5048bb362cafe835aff8`
（M105 完成点）；本项只增加验收说明和证据，没有再改运行代码、依赖或默认配置。
原始固定上游仍为 M91 指定版本，取舍沿用 ADR 0010／0011／0012。

最终回查核对了 22 次本项成功构建及 16 个可继续使用的既有构建的实际选定输入、
源副本、selector、工具和原始构件。已变更输入的旧图重新构建；未变更图复用原始 pin。
其中 Native／WASM SDK 的实际编译输入没有变化，故复用 M103 的最终构件，
在本项重新执行完整故事。构建时间早于本项，不等于使用旧源码；全局源码摘要
`d16af3fa218b14845928b8e6b801a001409f5c2354b6776f86025e1729d8bade`
及每个实际输入哈希共同限定资格。

公开回执、原始日志和完整索引见
[汇合证据](../evidence/fork-compatibility-convergence-2026-10-19/README.md)。
历史各组的 RED、初次不合格、原版基准及独立审查保持原有标记。

## 原版消费者与默认行为

以下消费者均保留建立各组固定基准时的源码字节；本项没有改写它们使其通过。
源码保存清单另外包含历史 fork 夹具和私有函数观测夹具，不能把整个清单当作
“所有文件都在最终上游兼容矩阵运行通过”。

| 契约 | 最终直接检查与结果 |
| --- | --- |
| 原版 early-data、hooks、路由、公开结构与旧 socket trait | 两个既有消费者与永久 fixture 字节相同；实际入站早期数据、恢复后的出站 0-RTT、Accept／Reject、路由和普通流通过。M92 的原消费者也单独对最终 Iroh 编译成功。 |
| 独立受限接口 | 18 项编译契约：两个允许的业务入口编译成功，16 个提前取得普通端点／连接、raw stream／datagram、weak upgrade、旧 Router 绕行或 Native control 误用被指定诊断拒绝。19 项实际端点测试验证顺序、否决、撤销、取消、drop、期限和错连接 proof。 |
| 旧 auth 与 checked error | 旧 process／穷尽错误匹配消费者重新运行成功；checked policy、因果前驱、拒绝不入图、持久化隔离及回放检查通过。原有处理算法未复制。 |
| 原 native sync features | 原消费者在 defaults、sync 自身 defaults=false、显式 test_utils 三图均实际使用 SQLite 并重开、关闭和移除夹具。11 个最终 feature 图同时核对 native、browser、workspace／dev 及 WASI 边界。 |
| portmapper 默认与显式产品配置 | 未改原消费者在自有网关实际建立、使用并删除其 UPnP mapping；外部 owner 的 mapping 保留。没有编译 portmapper feature 的图实际零网关协议流量。产品 UDP Disabled、TCP 正例及保存／恢复配置检查通过。 |
| pkarr redirect 与设施许可 | 原生两个原入口均在自有设施完成有跳转的签名 PUT／GET。最终独立 WASM 消费者实际验证普通 Fetch 以及受约束入口拒绝；九个 Native 许可／重连／跳转负例通过。 |
| p2panda 缺地址与显式 resolving | 未改原消费者保留即时 TransportInfoMissing；配置了 lookup 也不意外查询。显式 resolving、真实 stream、本地 store 错误、真实握手失败与本地拒绝分类、backoff 边界共八项通过。 |
| 旧 selector 与新连接级 selector | 原有状态消费者保留 peer 范围和两次回调，没有额外连接级回调。最终真实 socket、统一连接和维护用例继续核对原 connection、获准 path、association、socket／generation、退休及撤销。 |
| 普通 Noq／Iroh 与 ACK | Iroh 的 IP、relay、旧 custom sender 三模式通过。Noq 原普通消费者五场景、独立 probe 消费者四场景、实际丢包消费者两场景均按原字节编译并运行；具体边界见下文。 |
| 诊断、CID、附加能力及分发 | 默认 off、显式 on、finished、callback／容量失败的原诊断契约重新运行；23 个协议／CID／路径用例通过。最终 Noq all-targets 和五个 examples 入口重新构建，原 main 的五连接及端口释放重新运行通过。 |

M93 的 `fixed-ordinary-wrapped-consumer.rs` 是类型迁移前 **fork** 的历史夹具，
使用新增 builder hook，并依赖当时 fork 的隐式 identity-only lookup。它不是在官方
原版建立的消费者。本项保持其字节并验证最终公开 API 仍可编译；不声称其旧默认运行
行为继续成立。M100 恢复原版缺地址即时失败后，依赖此前 fork fallback 的调用者须
显式选择 resolving；相应成功行为由原有 resolving 真连接用例验证。
M99 的 private-builder consumer 使用只在观测 overlay 暴露的私有函数，属于历史
实现观测；没有把该观测函数当成分发的公开接口。

原版入站直接返回和出站 Result 契约、可穷尽 Accept／Reject，以及受限句柄不可
绕行的结论还由最终编译／运行证据交叉验证。对旧过渡 outcome／hook 接线的精确
符号搜索无残留，但没有只凭搜索零匹配推断安全性。

## SDK、路径与生命周期

本项三个完整 SDK 场景都使用同一对最终 Native／Browser 构件：

1. 在自有 loopback TLS／WSS 设施实际发起受限请求，一次审批，保存授权和
   admission secret，双方按同一 persistent identity 重开，再取得普通 member
   connection 并完成 PONG。
2. 重新完成材料交付后验证资源禁用、RTC API 不可用、取消、期限及资格成立后
   释放前撤销五个拒绝场景；允许的 RTC member 流仍可使用。
3. Browser 作为审批／precode／replica 服务方，在原 member connection 上完成
   主动 RTC 改路；五条 replica scope 完整保存、应用、同身份重开、重复回放，
   应用视图保持一致。

另外，11 项 Native 准入测试、原有 control worker 所有权测试及固定旧 Native SDK
counterpart 的实际协议互操作通过。运行了七个同会话维护用例，覆盖有效 TCP→UDP
切换、两侧发起、物理业务路丢失、过时同地址、已关闭连接以及真实时延变化。
12 个真实 socket 边界、15 个统一连接用例和四个最终发包 guard 用例继续成立。

所有 SDK、浏览器、relay、设施容器、工作线程和相应 socket 均按各自回执正常退出；
保留立即与晚到的 WebSocket 关闭观察，没有以强杀补成合格。生成的 TLS private keys
和 NSS 在确认 actor 已退出后移除。私有 SDK receipt、invitation、store 与 actor logs
不进入公开归档，仅保留哈希及经过筛选的结果。

138 个报告的外层 harness 入口通过，其中一个诊断入口另有十个独立子进程场景。
18 项受限编译检查、额外的原版／历史 wrapper API 编译检查、普通 main、feature 图和
三个 SDK 故事分别记账，不重复相加成一个“全面兼容”数字。

## 保留的明确边界

- **ACK 分类。** 普通 Backup ACK 保留原路由。黑洞仍 Available 的主路下，普通消费者
  保留原四秒 ACK 观察期限及两秒 close 观察边界，随后恢复原连接；这不是把旧超时
  称为新的成功。显式 probe 的四场景均实际取得 fresh authenticated ACK；黑洞用例
  确认普通排队 STREAM 不从备路泄漏。独立 loss 消费者实际注入一次并观测到一次
  client backup 丢包，再完成恢复。此策略是显式 probe 能力，不声称普遍适用的 ACK 修错。
- **features。** Native store defaults 保持；Browser 正常／workspace-dev 为 traits-only。
  Browser 显式 test_utils 仍会加上 SQLite，属于原 native fixture 的边界，不能声称用
  feature 减掉已经统一开启的依赖。WASI 保留原默认，不构成新的 WASI 构建承诺。
- **诊断。** observer 默认没有安装；启用后的记录、hash、copy、allocation 与锁内
  callback 成本保持明确说明。doc-hidden 不代表编译隔离，计数也不等于物理发送证明。
- **产物。** 混合默认 rlib／cdylib 继续作为明确例外，以避免 unversioned cache 覆盖、
  错误 Fresh 及既有平台导出限制。显式 Linux 生成入口的两份原始 cdylib 在最终输入
  回查后再次实际加载；不承诺 C ABI 或其他平台产物。relay CLI 原来就存在。
- **源码分发。** Noq／Proto package 的全部成员及包前 source／lock 在最终回查中再次
  核对。包内消费者需要已记录的配套本地 patch 与固定 lock 适配；dependency patch
  不传递，单独 Noq 包配 registry Proto 未获得资格。没有把其他 vendor standalone、
  PQ 可选测试、HTTP file-serving、benchmark 性能或所有平台一并宣称通过。
- **修错与迁移。** Mapping ownership／正确协议清理、本地拒绝不记 reachability failure、
  backoff 当次 clamp 保留各组直接证据。依赖旧 fork 隐式 lookup 的消费者使用显式
  resolving。ADR 0009 已取消的旧产品连接协议不在本次兼容范围。

具体命令、旧基准及迁移说明分别见
[early-data](2026-10-19-ordinary-endpoint-compatibility.md)、
[auth](2026-10-19-auth-error-compatibility.md)、
[store features](2026-10-19-sync-store-feature-compatibility.md)、
[portmapper](2026-10-19-portmapper-default-compatibility.md)、
[pkarr](2026-10-19-pkarr-redirect-compatibility.md)、
[resolving](2026-10-19-p2panda-resolution-compatibility.md)、
[selector](2026-10-19-selector-compatibility.md)、
[ACK](2026-10-19-backup-ack-contract.md)、
[guards](2026-10-19-path-guard-compatibility.md)、
[diagnostic](2026-10-19-packet-witness-opt-in.md)和
[分发产物](2026-10-19-vendor-distribution.md)。

## 失败记录、来源与范围

本项保留而不改标的验证失败包括控制器准备时 tuple 读取错误、网关场景参数误填、
Noq 不受支持的场景名仅走普通分支、资源 selector 的过时本地依赖行，以及容量门槛
在 Cargo 前拒绝。修正控制器／selector 后单独重新验证；不把它们称为产品 RED。
资源 lock 只为本地 p2panda-net 行补上已固定的 mDNS 依赖，所有外部 package 行、
版本、source 和 checksum 保持，没有重新解析。

严格的最终 selector 回查还发现 M105 两次早期 Noq 构建的 manifest 哈希与最终版本
不同。对应 Rust 输入和原始构件仍在，但早期未提交 manifest 的字节未找到；本项
没有猜测其完整差异或把旧 build context 改标当前。最终 manifest 下的 all-targets
和 examples 已以同一固定 lock 实际重跑；13 个实际 Cargo 输出都与保留的原始 pin
字节相同，记录的是这次构建的来源和输入。最终 manifest／lock 字节同时进入归档。

共享 target、稳定 toolchain 与既定 profile 保持。缓存清理仅移除与保留原始 pin
逐字节相同的已完成可执行输出副本；已被后续结果取代的旧测试程序使用无损压缩、
完整 roundtrip 和恢复映射。没有复制 target、删依赖库或改动固定 SDK 原始构件。
249 项原有 WIP 的哈希和 dirty 集合保持。本项所有执行都在已有受控设施／隔离夹具上，
完整实体、公网、Android、全 NAT／平台矩阵以及性能 SLA 不在本次证明范围。

M91 的故事覆盖由实际分组承接：1–16 对应 M92／93／94／95；17–23 对应 M98／99／100；
24–30 对应 M96／97；31–38 对应 M101／102／103／104；39–43 对应 M104／105 及完整
SDK 故事；44 对应已建立的十五个分组和真实 blocking edges。本项逐组结果与对应
故事编号在机器可读矩阵中逐条列出，不能用这一范围概述代替实际回执。

M106 只收敛上述实现与资格；按其明确验收约束，不关闭或修改 M91、M69、M84，
也不以本项结果替代这些任务中独立保留的工作。
