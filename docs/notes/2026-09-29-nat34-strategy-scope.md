# NAT3/4 UDP 打洞增强范围与验证边界

日期：2026-09-29。状态：用户已明确确认的产品范围，加上待实施时验证的策略假说。工作状态和原生依赖以 Plane [MIOPUNCH-14](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189) 的子票为准。本文不将旧 Go 算法或旧实验台参数变成新基线决议。

## 已确认的目标

用户明确指出，miopunch 的核心是针对 NAT3/4 的不同条件采用不同策略，提高实际 UDP 打洞成功率。这补充了原规格「建立并改善成员间数据路径」的范围：不能只凭一个稳定端口的双 NAT 正例，就视为 NAT3/4 策略已经交付。目标是可测的成功率改善及有界成本，而非保证任意 NAT 均可直连。失败后仍按已获准的数据路径和节点维护预算处理；消息旁信道仅用于受限握手与控制，不承载正常业务。[产品主线](2026-09-27-product-mainline-draft.md)、[网络场景](2026-09-28-network-scenario-walkthrough.md)、[ADR 0005](../adr/0005-side-channel-control-bootstrap.md)。

旧实验台的兼容标签：NAT3 = endpoint-independent mapping (EIM) + address-and-port-dependent filtering (APDF)；NAT4 = address-and-port-dependent mapping (APDM) + APDF。NAT4 还需分可预测的端口变化与不规则/随机变化。测试报告须同时写出映射、过滤和端口分配实测行为，不只写 NAT3/4 标签。NAT1/2 用作基线和回归，而不是本次核心增强的替身。[旧分类约定](../../archive/legacy-go-openspec-2026-09-28/docs/decisions/p0-nat-lab-charter.md)。

## 当前交付边界

- [MIOPUNCH-27 的运行证据](2026-09-29-udp-nat-validation.md)在两个内核 NAT 后，以 EIM + **地址相关过滤** (ADF) 完成真实 MQTT 控制、同一 QUIC UDP socket、同一连接的 IP 路径及业务；禁 UDP、错/旧候选和控制中断可有界失败。它没有验证 APDF、APDM、端口变化或跨运营商成功率。27 已 Done，保留原验收和证据，不追溯扩大验收。
- 当前 [UDP probe](../../src/bin/udp_nat_probe.rs)交换各端一个新鲜 QAD 地址、发送一次同 socket prime、由拨号侧对单个远端地址开 QUIC 路径。`UdpObservation::Fresh` 给出地址、有效期及 `mapping_varies_by_dest: Option<bool>`；它不产生端口步长、方向或给定 peer 的公网端口预测。[观测定义](../../vendor/iroh/src/socket.rs)。现有 iroh QNT 只基于已知候选及 client 连接触发；新策略需要受控协调，不能把任意猜测候选直接变成自动 QNT 输入。[peer actor](../../vendor/iroh/src/socket/remote_map/remote_state.rs)。
- [MIOPUNCH-39](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/34bb9fcc-6b2e-4dc0-b10b-c5d26a805c60) 是真实网关端口映射，不替代穿透无映射的 NAT4；[MIOPUNCH-42](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/99acd13e-9980-45b0-a3ae-c29ec6babd2c) 是正式 MQTT 载体/正常数据路径集成；[MIOPUNCH-43](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/0198d57b-cacd-423d-80f2-06e46efe6859) 是 TCP 打洞；[MIOPUNCH-47](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/acdfed6c-5c61-40c6-a9a9-bca18644afa3) 是多路径质量及失败回退。它们与此 UDP 策略工作相关，但不重开或重定义。新策略可从 27 的真实探针和 [29 的准入契约](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7e563c99-863d-4ed7-9c79-7cd64268fae0)开始，之后被 42 的正式载体复用。

## 旧 Go 事实与可复用的经验

| 旧路径 | 已存在的能力 | 不能直接迁移的限制 |
| --- | --- | --- |
| `nat/classify.go:42-103` | 从多目的观测地址判断外部 IP/端口是否变化，输出 Easy/Hard、`PortsDifference`。 | `RegularPortsChange` 仅由同一公网 IP 且端口最大最小差 1–5 推断；不检查样本顺序、递增/递减、固定步长、过滤行为，且 IP 变化时不会给出端口规律。Easy/Hard 不能等同 NAT1–4。 |
| `internal/punchdecision/nathole_analysis.go:26-123,183-219,275-304` | 根据两端 Easy/Hard 和“规律”选择 mode 0–4、发送/接收角色、延时、低 TTL、范围或随机尝试；成功后记分。 | 常量如 10 个范围、1000 个随机目标、256 个随机监听端口、TTL 4/7 是旧尝试表，不是当前产品预算；两端不规律时仍可能回落到并无针对性证明的 mode。 |
| `internal/punchdecision/decision.go:625-735,767-790` | 样本不足时有基础回退；双方策略带总时限，扫描范围由最后观测端口和跨度生成。 | 候选区间只是 `last ± min(difference+5, maxNumber)`，未预测有符号步长；观察目的地的端口未必是 peer 映射。 |
| `internal/punching/punching.go:347-395,413-502,700-850,934-1003` | 先收后发、延时、重复普通目标探测、一次范围扫描、一次随机端口抽样、取消与时限；低 TTL 写入时保存并恢复 socket TTL。 | `ListenRandomPorts` 会额外绑定 UDP socket；随机目标从 1024–65534 均匀抽取，命中率可能极低；低 TTL 需要在共用 QUIC socket 上证明逐包隔离，避免影响 QUIC/其他连接。旧逻辑有界于时间，却未给当前移动设备的包量/并发预算。 |
| 旧 [F-001 排障](../../archive/legacy-go-openspec-2026-09-28/docs/notes/mainline-network-test-findings.md) | NAT2/3 的控制下发次序曾造成普通 sender 包先到受限 NAT，后续低 TTL 包未及时配合；后来补 receive-first 与有限重复探测。 | 这是时序经验，不是新 Rust NAT3 已验证；需要用新同 socket QUIC 路径重新证明。 |

## 策略矩阵：实施假说和试验顺序

| 双方实测特征 | 首个可验证策略 | 预期增益与对照 | 边界与失败路径 |
| --- | --- | --- | --- |
| NAT3 × NAT3；或 NAT3 × EIM/ADF | 同步一轮、双方先准备接收，再从 QUIC UDP socket 向对方**精确公网 IP:port**发有界探测；交换准备/开始状态，重复包覆盖过滤开放时差。比较角色/时序顺序。 | 减少普通 QUIC 首包先到、被 APDF 丢弃后双方错过窗口的次数；与 27 式单次 prime/路径打开交错重复比较。 | 若仅单侧看到过期候选、控制迟到或过滤未打开，停止本轮，更新观测或走获准备用路径。低 TTL 仅在隔离及实际收益已证明时纳入。 |
| NAT4 端口变化可预测 × NAT3/EIM，随后可预测 × 可预测 | 用至少足够区分“稳定、递增、递减、固定步长”的**同 socket、按目的地及时间标注**观测，给出可信度和有效期。双方按认证控制消息协商角色与开始窗口，围绕实际预测端口做小的双向有界探测，并以 QUIC 路径验证。 | 当观察服务端口不等于 peer 侧映射端口时，提高有限探测命中目标的概率；分别比较一侧和双侧 APDM 的成功率、时延及包成本。 | QAD 的 `mapping_varies_by_dest` 只能提示变化，不能单独作为预测模型。样本不稳、跳变、地址变化、缓存过期或预算耗尽时不得扩大无界扫描。两侧 APDM 的关联误差应分别实测。 |
| 一侧或两侧 NAT4 端口不规则/随机 | 在授权、成本预算与可重复测得收益成立时，尝试有界随机目标/候选窗口或其他可证实的策略；记录命中概率、包数和失败原因。 | 小预算随机命中率可能很低；只有重复试验相对单候选基线显示可接受的增益与成本时，才启用该直接 UDP 策略。 | 不从旧随机多 socket 监听直接搬运：QUIC 会话必须绑定实际使用的 UDP socket。真实随机映射下可能无法改善；必须诚实返回有界失败，并让已获准中继/TCP 等路径继续竞争。 |

所有行均以同一已认证 QUIC 连接的实际数据路径和受控业务数据为成功标准。候选地址不等于可达路径；探测包本身也不等于身份或成员授权。发包目标须来自预期身份之间受保护的控制交换，并受本地网络准入、秘密持有、成员资格、资源许可、候选有效期和尝试预算限制；错误身份、伪造或过期候选不能造成未授权广域端口扫描或业务越过信令隔离。先落实 [MIOPUNCH-29](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7e563c99-863d-4ed7-9c79-7cd64268fae0) 的分阶段准入，再将新探测入口接入相同闸门。低 TTL 的价值与安全性、预测窗口、重试间隔、包/端口/时间上限和不同节点维护档位，均由受控试验选定并记录，不沿用旧常量。

## 共同验收方法与留待实测的问题

每张策略票首先在隔离 VM 或内核 netns 中复现**真实**映射与过滤规则，并用网关两侧抓包/conntrack/规则计数证明实际分类。每个组合在相同版本、相同拓扑和可控随机种子下，交错重复基础策略与增强策略；记录尝试次数、成功率分子/分母、达成路径的时间分布、候选/探测包数、socket 数、控制消息量以及 CPU/持续时间等成本，并保存可重跑入口。至少交换拨号/接收角色；如实报告每种端口分配类别的收益和零收益，不能把 mock、仅本机双进程或单次成功宣称为普适打洞率。随后立即做禁 UDP、错误/旧候选、控制延迟或取消等最小失败检查。成功还须验证真实 TLS/QUIC 对端身份、同一 QUIC UDP socket 与连接连续性、成员准入以及正常业务仅走获准数据路径。

需实测后决定的点：观测服务器的目的地数量和拓扑是否足以识别步长；端口复用/重映射受时间与并发影响多大；NAT3 APDF 是否需要低 TTL 或仅同步重复探测；在不增设第二个 QUIC socket 的条件下如何覆盖不规则 NAT4；低功耗节点可接受的探测预算与获准中继/其他路径竞争规则。测试失败本身是有效结论，但无增益不等于“提高困难 NAT 成功率”的整体目标已完成。若同 socket 有界方案无增益，应保留统计与拓扑证据，给出可执行下一方案或提交明确的支持范围决议；额外 socket、新连接或改变身份/业务路径语义都须另行审查并形成后续实施票。不得据旧 Go 的参数或标签替代证据。

对应 Plane 子票：[56 NAT3 端口级过滤](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/dff8168d-1cd6-42c5-accc-f6b79c584538)、[57 单侧规律 NAT4](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/1ccad192-57a0-40ee-aea6-fc5cd8d1f503)、[58 双侧规律 NAT4](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/841d7c0d-6438-43a2-b476-3927875b604d)、[59 不规则 NAT4](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ef55c03d-841d-4849-96e2-9556ac00da40)。各票的当前状态与原生阻塞关系以 Plane 为准。
