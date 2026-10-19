# 普通路径放行与显式收发 guards 的验收

2026-10-19，[MIOPUNCH-103](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/102f6353-93ef-4093-a614-057b3a1326fa)
依据 [ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md) 验证新增过滤
的普通兼容边界，以及 [ADR 0011](../adr/0011-native-local-socket-identity.md) 的
显式 socket 所有权。本项补充固定原消费者、实际收发回归和工作量记录；
普通默认路径无需删除 guard。实际负例另发现 application predicate 撤销后，
已经编码的 STREAM 可绕过该 predicate 的重查，向对端交付禁止的明文。

## 暂存封包的最终应用权限

修补前，真实旧 UdpSender 返回 Pending 后暂存 47 字节 STREAM 密文。
只将已安装的 application predicate 从 true 改成 false，不安装 packet
restriction，再恢复 sender 容量：相同密文被再次交给 carrier，对端实际收到
`buffered-forbidden`。负例的断言在连接、endpoint 和 UDP 端口均回收后失败。

局部实施决定：由现有 PacketBuilder 按实际 Data-space frame 记录私有许可位，
在同一次 transmit 的 coalesced packets／GSO datagrams 间取并集，随
SocketTransmit 保留；每次新 transmit 重置。Noq 的实际 sender 调用前，
按该位重查当前 application predicate。混合批次整体拒绝；PING、ACK、路径
验证、地址 observation 和 close 保留原有例外。未安装 predicate 时仍允许。
原 Transmit 公开字段、普通构造器、sender 必需方法和 QUIC 线格式不变。
丢弃的已编码普通数据仍按现有丢包恢复机制等待允许路径；不另建队列或调度器。

回归还让撤权后的 PING 在同一个旧 sender 上 Pending，恢复后确认实际收到
PING 和认证 ACK，防止把所有暂存封包一律禁止或沿用前一批 STREAM 的许可位。

## 安装与允许边界

Iroh 注册连接时安装 custom packet gate、ordinary-frame gate，Native 还安装
本地 socket gate。前两者对已知普通 IP／relay 路径返回允许；旧 custom sender
未选择 guards_application_data 时保留其原有收发与终止错误处理，不附加
connection-owned qualification。Native socket gate 对未指定 LocalSocketId 的
普通路径返回允许，不查询 pool；指定标识时才核对原连接、peer、live／pinned
资源。未能反查的内部映射仍拒绝。

这些 hook 仍有内部映射、Option 判断和回调工作，不能把“没有调用 RTC policy”
写成“没有过滤成本”。保留每条 Native 连接上的 socket gate，是为了防止之后
引入的 pool 包按 CID 落入另一条原连接；不能因目标连接尚未登记额外 socket
就省略最终入站检查。普通 Noq Endpoint 自身没有安装限制时，packet 和
application predicate 均为空，保留其原调度与普通 stream。

显式 RTC 地址才进入 association owner 和 qualification 方法；原连接、资源
revision、退休状态和 ordinary-data 放行继续求交。池中标识、远端 custom token
及一次旧探测均不授予另一条连接权限。发送检查保留到实际调用 sender 前，
ordinary-frame 检查也不替代收包前的 packet admission。

## 固定消费者与实际负例

首次新增回归前，在 published Iroh 1.2.0 与当前 fork 上固定并重复运行相同源码。
普通 IP、自有 HTTP relay、只实现原必需方法的 custom UDP sender 各完成三条
真实 TLS stream；每条双向各 32,768 字节，发送确认、关闭和端口释放均核验。
原版／fork 的 Noq 单路与多路普通 stream 也以同一既有固定消费者重跑。Iroh
默认 features 保留，原版锁不变；fork selector 的完整外部依赖行保持基线。

新增持续集成回归包含该固定消费者的逐字节副本。另一个回归在已安装 RTC
factory 的同一连接上先经 IP 交换，再使 RTC 成为唯一可用候选：未 qualified
时排队字节不能交付，实际 STREAM frame 计数为零；双方取得各自 fresh ACK
并 retain 后，原 stream 继续双向交换各 98,304 字节。错误原连接与对侧 probe
不能借用 association；退休后排队的普通字节不能交付，两个 pump 自然结束并
被 join，未以 abort 代替资源验收。

Noq 回归先以旧 sender 必需方法完成未安装限制的真实 stream，再让实际 sender
返回 Pending。此时已有新 STREAM 被编码，47 字节密文停在 connection driver
的 buffered transmit 中。packet restriction 用例安装禁止规则、恢复容量后，
实际 carrier send 计数仍为 6；相反方向的真实加密 stream 同样被入站过滤。
独立 application predicate 用例中，禁止的原密文重试次数为零，对端未收到明文；
PING 等控制封包继续发送，不能要求总 carrier 计数不变。原连接的终止状态经
WeakConnectionHandle::is_open 观察，不重入 runtime lock。

## 工作量单位与限制

[原始工作量记录](../evidence/path-guard-compatibility-2026-10-19/work-unit-measurements.json)
分别保存业务阶段前后快照。固定普通 custom 用例中，原版与 fork 的业务阶段
各侧 is_valid_send_addr 入口数等于当次实际 poll_send 数；这只说明所测无
backpressure 场景，没有给所有 sender 的调用频率作保证。

在安装 RTC factory 的普通 IP 阶段，六种被计量的 custom sender 方法入口均为
零。显式 RTC 业务阶段，客户端 guards_application_data／owner／qualified
分别增加 4,174／3,954／1,867 次，服务端分别增加 3,516／3,296／1,538 次。
这些是包含 actor 和 packet 路径的真实方法入口，不能当作每包调用数；透明
计数器本身也增加工作。Noq 禁止规则用例记录 17 次显式 callback、8 个拒绝的
入站 datagram 和 6 个拒绝的 outbound transmit。独立 application predicate
用例记录 115 次 callback、1 个拒绝的 outbound transmit，禁止密文重试为零，
另有 1 次允许的暂存 PING 重试并得到对端 ACK。各阶段由密文、frame 计数、
回调和实际 carrier 计数共同证明，不能仅依赖拒绝计数器。

所测单位是 callback／method／poll／datagram／transmit 次数及实际字节数，
没有测量 CPU 时间或纳秒开销，也不声明吞吐、延迟或公网 SLA。不同 selector
保留各自固定锁，不把回环执行总时长作为跨依赖图的性能比较。

## 证据与回收

四项针对性回归、17 项协议边界、12 项物理 socket 边界和 15 项 unified
connection 回归共 48 项通过。实际受影响的 Noq、Iroh、Native／Browser SDK
均重新构建并固定实际选中源码与构件；固定原消费者在最终 fork 上也完成全部
普通场景。完整当前 SDK 经自有回环 TLS／WSS 与 relay 完成受限请求、一次批准、
授权和 secret 持久化、双方同身份重开及成员 PONG。请求耗时约 659 ms 仅是本次
运行观测，不是性能承诺。
完整公网、实体设备和全平台矩阵仍按原范围独立验收。

初始消费者准备时误读原版私有 ECN 字段，在原版成功前即修正，并保留失败回执；
之后原版与 fork 的固定源码未改。Noq 新回归第一次仅在立即端口重绑处失败：
最后引用释放只唤醒异步 endpoint driver。修正夹具为一秒内等待真实重绑成功，
仍核验描述符释放，没有修改生产代码或把该准备失败算作产品反例。

构建保留共享 target、原有 profile、固定锁与编译前的源码／工具／产物检查。
缓存整理仅移除已核对 SHA、保留原始 pin 的过期 SDK 缓存副本，以及保存无损
压缩还原记录的过期测试程式；没有压缩、硬链接或修改 SDK 原件，也未删除
依赖库。SDK actor、browser context／process、relay 和容器均自然关闭，未强杀。
完整回执与双轴审查见[验收证据](../evidence/path-guard-compatibility-2026-10-19/README.md)。
