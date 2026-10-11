# M88：IRC Base64、完整行预算与 SAFERATE

[MIOPUNCH-88](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/38a04837-0f72-4b88-9dbb-efe47c590730)
依据用户重新修改并测试、随后明确使用 Base64 的指令实施。
[实施说明](../../notes/2026-10-11-irc-base64-saferate-implementation.md)
记录协议、用途和资源边界；服务研究另见
[MQTT/IRC 限制核查](../../notes/2026-10-11-mqtt-irc-pacing-and-size-research.md)。

当前正式 Native/WASM SDK 已通过严格 purpose2 请求、实际未批准请求保存、
审批、材料保存、双方持久身份重开、获准 relay 上的 member/1 PING/PONG 与自然清理。
初始浏览器没有 data relay 或 RTC；IRC 只承载受限控制。

| 同一公网设施与显式 caller/profile 配置 | 首次完整请求 |
| --- | ---: |
| M87，MIR1/128 B/600 ms | 228.69 s |
| M88 首轮，仅 SAFERATE 取消等待 | 268.03 s |
| MIR2/Base64 动态分片，修正 Control 评分探测 | 60.488 s |

相较 M87 本次少约 73.5%。这是合并变更下各一次公网观测，不保证公共服务时延，
也没有分别隔离每项变更的性能贡献。首轮更慢的结果和原始失败仍保留。
成功采用显式 300 s caller、30 s queue TTL、8 s assembly TTL，配置间隔仍为 600 ms，
只有双方实际 SAFERATE 广告使该额外等待取消。默认 caller 仍为 60 s；
本次耗时超过它，不能称为默认预算成功。

双方自己的实际 `nick!user@host` 前缀均为 40 B，目标名为 25 B。
按完整转发行预算得到每片 252 B；首个 1200 B datagram 确实分为 5 片，
客户端发片跨度约 0.24 ms，不代表服务器转发也没有延迟。
CDP 实际收到的最大 MIR2 行为 509 B；发送最大行加服务器前缀后也是 509 B。
132 个发送片、108 个接收片均为规范 Base64，未观察到 MIR1 片或畸形编码。

39 项原生短回归通过：IRC 16、共同连接 13、准入故事 4、接受上下文 3、成员路径协议 3。
真实 QUIC/TCP 回归在修正前的 1100 ms 窗口观察到 Control 的 5 个额外评分 PING，
修正后为零；Ordinary 的评分探测保留。协议自己的 QUIC 心跳、重传和拥塞算法未改变。
Base64 回归覆盖任意二进制字节、乱序、重复 TTL、冲突片、超限、非规范 padding/尾部位、
完整行长和前缀变更。原生实测输入图在最终浏览器依赖补齐后完全未变，
经逐个 `.d` 输入和固定 lock 核验复用原测试构件与结果，未重复复制或编译。

当前 Native/WASM 的六项正式 SDK 自有设施回归全部通过：

| 场景 | 实际观测 |
| --- | --- |
| 无效超大预算 | timer/I/O 前拒绝，无浏览器 WSS |
| 显式 2 s caller | 2007.5 ms 后失败，没有 Native 请求 |
| maintained listener 闲置 3.1 s 后请求 | 658.4 ms，严格 purpose2，真实未批准请求 |
| 显式取消 | 258 ms 生效，没有 Native 请求 |
| 禁用 IRC | 没有浏览器 WSS |
| MQTT 严格受限请求 | 100.6 ms，真实未批准请求 |

各 case 的 SDK/driver/socket 自然关闭，两个自有容器已经删除。
公网最终 Browser close/free 返回约 1002 ms；500 ms 的 socket 观察仍为 false，
追加约 2006 ms 后三个 WSS close event 全部到达，之后才销毁 context/browser。
首次 Native SDK close 自然返回 13 ms，最终 Native close 也自然返回，driver 为零、
即时和晚期进程观察均缺席，没有强制终止。整次公网故事约 97.8 s。

文件：

- `final-public-story.json`：严格身份/用途、Base64 完整行、审批、持久重开、relay 业务和自然清理。
- `final-affected-regressions.json`：六项实际 SDK 回归和各自回收。
- `rust-regressions.json`：39 项测试、exit 与实际当前输入图复用依据。
- `sdk-builds.json`：固定构件、实际 Fresh/rebuilt、所选输入和源码副本数量。
- `diagnosis-and-prior-runs.json`：等待和评分探测的红测，以及保留的慢公网结果。
- `owned-source-scope.json`：固定基点与自有源码 hash、依赖边界。
- `probe-sources/`：原样保留测量驱动、私有测试 helper、selector 与 lock；不包含生成的 store、secret 或原始报文。

发布内容采用标量与 hash 白名单。身份、grant、secret、ciphertext、临时频道和生成的
持久材料不写入公共回执；原始记录、不可变构件、源码副本和 depfile 保留在所列私有目录。
每次构建和运行均在启动前及结束后核验源码、工具、lock、controllers 和实际所选输入。
新增 Base64 依赖沿用 lock 已有的 0.22.1；只有两个本地 package 记录增加依赖边，
所有 registry package 的版本、source 和 checksum 均未重新解析。

该资格属于上述服务、当前构件和显式配置，不替代 M84 的物理平台组合验证。
用户随后提出的整次握手开销盘点将进一步区分路径管理、阶段确认和必要准入数据，
不能把本次 60.488 s 当成后续优化已完成。
