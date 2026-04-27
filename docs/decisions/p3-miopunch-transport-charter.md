# P3 miopunch 传输层纲领

## 文档状态

- 本文档定义 `P3` 的目标、边界、约束与关键原则。
- 本文档不展开实现细节，不替代后续 OpenSpec change。
- 本文档不包含具体数据结构、字段名、函数签名等；这些应在后续 OpenSpec change 的 `proposal / design` 中定义与收敛。
- 但本文档会固化 `P3` 的关键对外语义与验收口径（例如：传输选项的命名与 CLI 选择面、以及实验台回归对 `payload exchanged` 证据的要求），避免后续实现阶段反复漂移。
- 后续实现前，应基于本文档与 `docs/roadmap.md` 创建并收敛对应 change。

## 背景

- `P0` 已提供可复现的单 VM NAT 实验台，`P1` 已抽离最小打洞内核，`P2` 已补齐 `IPv6 / dual-stack / UPnP / NAT-PMP` 等连通性增强。
- 现阶段 `miopunch` 已能解决“能否打通”的主要问题，但“打通之后如何稳定传输”仍与端侧流程耦合，没有形成独立的传输层抽象。
- 当前代码与文档中仍存在 `xtcp` 与 `miopunch` 的命名割裂；进入 `P3` 后，需要逐步收敛为 `miopunch` 统一命名，避免后续能力演进继续放大割裂。
- 后续希望接入的不是完整 `Hysteria2` 产品协议，而是其 `QUIC + brutal` 调度思路；因此 `P3` 需要先把传输层边界抽清楚，再把 `brutal` 作为新的传输选项纳入。
- 真实网络验证与公网协调通道尚未形成闭环；这些工作放到 `P4`，不作为 `P3` 的验收前提。

## 核心决策

- `P3` 聚焦 `miopunch transport`：将“打洞产出的可用 UDP 通道”与“后续数据传输会话”解耦。
- `P3` 同时承担仓库结构与命名收敛任务；该迁移属于 `P3` 正式范围，而不是独立于 `P3` 的旁支整理工作。
- `P3` 的结构与命名迁移必须先于 `brutal` 接入和传输层抽象落地执行，避免在旧命名与旧目录上继续叠加新能力。
- `P3(v1)` 的数据面选择面收敛为：
  - `--data-proto kcp|quic`
  - `--quic-cc bbr|brutal`（仅 `data-proto=quic` 时生效；默认 `bbr`）
- `brutal` 是对外名，表示“`QUIC` 下的 `brutal` 调度/拥塞控制模式”；它不是完整 `Hysteria2` 产品协议兼容层。
- `P3` 采用方案 A：全仓 QUIC 统一迁移到 `HY2` 最新 release 对应的 QUIC fork，并在 `miopunch` 侧钉死版本（仅在我们显式变更或追随重大修复时升级）；`control plane` 与 `data plane` 共用同一 QUIC 栈。
- `P3(v1)` 暂不引入配置文件与复杂 UX；实验台与回归优先通过 CLI 或代码常量写死参数，先闭环“能跑通 + 可回归”。
- 每次会话只选择一个传输协议；传输协议由本地配置或显式参数指定，不做“传输失败后自动切换协议”。
- `connectivity` 层只负责 `gather / exchange / attempt` 并产出已打通的 UDP 通道；不负责后续传输层选择、切换与调度。
- `dataplane`（传输层）负责：
  - 校验本次会话的传输选择与必要参数
  - 基于已打通的 UDP 通道建立数据会话
  - 输出传输层握手、收发与统计观测
- `P3(v1)` 不做中途传输迁移、动态协商或自动降级；只做单次选择、单次建立、单次运行。
- `P3` 仍处于探索阶段，优先做“可运行 + 可回归 + 可分层”；不以对外兼容与长期 API 稳定为目标。
- 命名收敛原则：
  - 新文档、新 change、新对外 CLI 文案统一使用 `miopunch`
  - 新增能力命名统一使用 `brutal`，不再对外使用 `HY2`
  - 不再新增新的 `xtcp` 命名；现有 `xtcp` 代码路径允许按阶段重构收敛，但不得继续扩散

## 仓库结构与文档迁移原则

- `P3` 应借机把仓库结构向更符合 Go 习惯的布局收敛；重点是去除历史遗留的 `xtcp` 命名空间割裂，并按职责重组核心包。
- 该迁移以“先统一命名和目录，再继续新增能力”为原则；不得在旧布局上继续叠加 `brutal` 或新的 `miopunch transport` 抽象。
- 推荐的目标布局应遵循：
  - 顶层只保留稳定的核心领域包，例如 `connectivity`、`dataplane`、`nat`、`event`
  - 协调端流程、线协议、TLS 工具、peer 端胶水和内部网络辅助逻辑放入 `internal/`
  - 不再保留新的 `xtcp/` 顶层命名空间
- 历史报告与历史决策文档不做机械重写；保留其历史语境即可，包括其中出现的旧术语与旧目录表达。
- 需要持续维护或面向后续阶段/对外表达的文档，应按新标准收敛，例如：
  - `docs/roadmap.md`
  - 后续新增的使用文档、接口文档、发布文档
  - `P3` 之后新增的决策文档、报告和 OpenSpec change/spec
- 文档迁移原则是“新文档按新标准写，旧文档只在必要时局部修正”，而不是一次性重写整个历史文档集。
- 已归档的实现来源与历史演进可保留“从 `xtcp` / `frp` 路径迁移而来”的说明，但不应继续作为新结构命名的依据。

## 包重组建议

- `P3` 的目录重组建议以如下映射为目标：
  - `xtcp/connectivity` → `connectivity/`
  - `xtcp/obs` → `event/`
  - `xtcp/control` → `internal/control/`
  - `xtcp/coord` → `internal/coordinator/`
  - `xtcp/msg` → `internal/wire/`
  - `xtcp/netutil` → `internal/netutil/`
  - `xtcp/peer` → `internal/peer/`
  - `xtcp/stun` → `stun/`
  - `xtcp/transport/message.go` → `internal/wire/`
  - `xtcp/transport/tls.go` → `internal/tlsutil/`
- `xtcp/util` 不应整体保留为新的 `util/` 包；应按职责拆散：
  - `xtcp/util/util/auth.go` 中的鉴权与常量时间比较逻辑 → `internal/authutil/` 或直接并入拥有者包
  - `RandID` 一类仅服务于会话/事务 ID 的工具，应优先并入实际拥有该语义的包（例如 `internal/coordinator/`），而不是继续保留通用桶
  - `xtcp/util/log`、`xtcp/util/xlog` 中的旧式日志辅助，应优先向 `event/` 的结构化事件模型靠拢；如短期仍需保留前缀 logger，可收敛到 `internal/logutil/`
- `xtcp/nathole` 不应整体原样迁移；应按职责拆分：
  - `analysis.go`、`classify.go` → `nat/`
  - `controller.go` → `internal/coordinator/`
  - `nathole.go` 中的打洞内核与探测流程 → 对外语义保留在 `connectivity/`
  - `nathole.go` 中更底层的打包、收发、探测细节可放入 `internal/punching/`
  - `discovery.go`、`utils.go` 中与 `STUN`、本地 IP、辅助网络工具强相关的部分，应按职责并入 `connectivity/`、`nat/` 或 `internal/netutil/`，不再保留“nathole 杂项桶”
- `P3` 对 `nathole` 的拆分原则应为：
  - 对外领域语义留在 `connectivity/`
  - 低层实现细节收进 `internal/punching/`
  - 避免让 `connectivity/` 重新变成过宽的“新杂物包”
- `P3` 对 `util` 的处理原则应为：
  - 不新增新的泛化 `util` 包
  - 工具函数尽量归属到真正拥有该语义的领域包
  - 能直接并回拥有者包的，就不再额外抽象出微型工具层
- `stun` server 建议保留为顶层包：它既是实验台和 CLI 的独立能力，也可能继续作为 `miopunch-lab stun` 子命令存在；不必因为当前主要服务于实验台就下沉到 `internal/`。
- `stun` client / mapped-address discovery 属于 `connectivity` 的 `gather` 语义，不单独抽成顶层公共包。
- 实验入口统一收敛为 `cmd/miopunch-lab`；`cmd/miopunch` 预留给 POC/产品 CLI（详见 POC-01）。

## 抽象边界

- `P3` 应形成最小且清晰的数据面边界：输入为打洞阶段产出的“可用 UDP 通道”，输出为“可读写的数据会话”及其可观测性。
- 数据面必须支持显式选择 `data-proto=kcp|quic`；当 `data-proto=quic` 时必须支持 `quic-cc=bbr|brutal`（默认 `bbr`），并允许携带该模式所需的最小参数集合（具体 schema 在后续 OpenSpec change 中定义）。
- `P3(v1)` 优先保持最小可用模型（单会话、单传输、单流），不预设复杂插件系统，也不提前承诺“完整多流框架”。
- `P3(v1)` 允许不同传输在内部维持不同实现细节，但外层端侧流程必须通过统一入口接入，避免再把 `kcp / quic(bbr|brutal)` 写成散落分支。

## 消息交换约束

- `P3(v1)` 不单独增加新的传输协商往返；传输选择复用现有 `exchange` 消息流。
- `visitor` 与 `client` 两端都必须显式声明本次会话的传输选择（`data-proto` 与必要参数；当 `data-proto=quic` 时包含 `quic-cc` 与必要参数）；协调端负责校验并回传最终选定结果。
- `P3(v1)` 不做 capability negotiation、优先级协商、自动降级或多候选传输列表交换；每次会话只有一个已选中的传输。
- 消息层不再使用纯字符串协议字段表达数据面选择，应升级为结构化的“传输选择”表示；是否需要兼容旧字段由后续 OpenSpec change 决定并写入 `proposal / design`。
- `visitor` 与 `client` 的传输选择在规范化后必须一致；不一致时应在 `exchange` 阶段直接失败，不进入 `attempt / transport`。
- 协调端在 `exchange` 阶段至少需要区分：配置非法、对端不支持、双方不一致、已选定成功（具体事件名在 change 中固化）。

## 目标

- 完成仓库结构与命名的第一轮统一，为后续 `miopunch transport` 抽象与 `brutal` 接入清理基础。
- 提供独立的 `miopunch transport` 抽象层，明确“打洞成功”与“后续传输”之间的边界。
- 将现有 `kcp / quic` 数据面重组为可独立替换、可独立测试的传输适配器。
- 引入 `brutal` 传输选项，并以最小配置集完成实验台内的可用性闭环。
- 让传输层失败可机读、可定位、可区分于连通性失败。
- 在 `P0` 实验台中建立 `P3` 的传输层回归基线与基础指标产物。
- 开始纠正 `xtcp` / `miopunch` 命名割裂，为 `P4` 的对外发布收敛术语与接口表达。

## 非目标（明确推后）

- 不在 `P3` 引入公网协调通道或真实网络闭环验证；这些属于 `P4`。
- 不在 `P3` 兼容完整 `Hysteria2` 产品协议、认证模型与产品语义。
- 不在 `P3` 自动探测带宽上限或自动学习 `brutal` 参数；`P3` 仅支持显式配置。
- 不在 `P3` 做传输失败后的自动切换、自动重试到其他传输或会话中途迁移。
- 不在 `P3` 引入 `relay / fallback`、`overlay / mesh`、`TCP punching`、`VPP` 或其他后续方向能力。
- 不在 `P3` 一次性完成整仓库机械化重命名；但必须停止扩散旧命名，并为后续收敛留出路径。

## 传输参数原则

- `P3(v1)` 的目标是最小可用闭环：参数模型尽量小，避免过早优化。
- `kcp` 与 `quic` 保持现有最小可用参数集合，不因为抽象层引入而扩展大量调优项。
- `brutal` 在 `P3` 阶段只要求支持“显式速率上限”与可选的报文大小约束；不做带宽自动探测、自动学习或上下行分离配置。
- `P3` 的实验台回归使用保守的最小参数集（强调联通性与可回归性，而不是吞吐极限）：
  - `brutal` 固定使用较小的 `up/down` 上限（例如 `1mbps/1mbps`）来跑通闭环
  - 报文大小约束与 MTU 以“不超过 `IPv6` 最小 MTU（1280）”为原则（实验台可取更小值，例如 `1250`），优先避免分片导致的干扰
- 参数的具体 schema、边界规则与单位表达在后续 OpenSpec change 的 `proposal / design` 中定义；本文档只锁定“显式上限、保守 MTU、固定参数跑通回归”的口径。
- `P3` 阶段对 `brutal` 的主要验收目标是“在统一抽象下能建立并交换 payload”，而不是追求吞吐极限。
- 实验台回归可使用较小的速率上限，重点验证抽象成立与可观测性完整。

## 可观测性约束

- `P3` 必须暴露并可机读记录至少以下类别的观测：
  - `exchange`：传输选择结果与失败原因（配置非法 / 不支持 / 不一致）
  - `transport`：握手开始、握手成功/失败、payload 交换成功、传输统计
- 失败必须可解释：能够明确区分“连通性已成功但传输握手失败”与“连通性本身失败”。
- `transport` 层的事件、错误和统计输出应与 `signaling / gather / attempt` 同样具备结构化、可机读、可回归的属性。
- 事件命名与 payload schema 在后续 OpenSpec change 中固化；本文档只约束“必须有这些观测点”。

## 2026-04-26 补充：peer transport session 与 logical stream

MNT-01 的 `data_proto=kcp` specialty 暴露出 P3 早期 stream 抽象的方向问题：当前 dataplane 把 punching 后的 carrier 直接暴露为裸 `io.ReadWriteCloser`，一次 `ping` 操作返回时可能关闭底层 KCP/UDP socket。KCP 最先暴露这个问题，但根因属于 transport session / logical stream 分层缺失。

P3 后续设计应从“单会话、单流”的早期模型升级为：

```text
punching path
  -> secure peer transport session
    -> mux/native stream layer
      -> generic logical stream(kind, metadata)
        -> payload protocol
```

协议模型：

- TCP：`TCP carrier -> TLS 1.3 identity binding -> yamux -> logical streams`。
- KCP：`UDP punching path -> KCP carrier -> TLS 1.3 identity binding -> yamux -> logical streams`。
- QUIC：`QUIC native TLS 1.3 identity binding -> native QUIC streams`。

安全口径：

- KCP 不使用 kcp-go optional block crypto 作为主安全层。
- QUIC 不再额外套一层 TLS，而是使用 QUIC native TLS 1.3 并补齐 identity binding。
- TCP 与 KCP 应尽量共享 TLS 1.3 identity binding、session 和 mux 代码。

生命周期口径：

- daemon 采用 on-demand live session：按需打洞建 session；session 活着时复用 logical streams；空闲、认证失效、配置变化或 transport fatal error 后关闭。
- 不照搬 FRP proactive tunnel 作为本轮硬要求。
- 不持久化 session endpoint、candidate、mapped addr 或 winning target；session 死后从当前网络状态重新 gather/exchange/punch/secure-session。
- 关闭 logical stream 不关闭 peer transport session；关闭 session 才关闭所有 logical streams、mux/QUIC session、secure session、carrier 和底层 socket。

logical stream 必须是通用抽象，不得写死成 `shellproto ping/sh` 专用通道：

- 每个 stream 打开时声明 stable `kind` 和小型 structured `metadata`。
- stream-open 阶段完成 peer membership、revocation、kind、target、session 等授权。
- `shellproto` 只是当前 payload protocol，可作为 `kind=shell.v0` 的上层内容继续存在。
- 未来 `socks5.v0`、`http-forward.v0`、`file.v0` 等业务不需要伪装成 shellproto hello/ping/sh。

timeout 与诊断口径：

- session 必须有 keepalive 或等价活性检测。
- session 必须有 idle timeout。
- 每个 logical stream 必须有独立 deadline，不继承 punching round timeout。
- close reason 必须可诊断，至少区分 idle timeout、daemon shutdown、identity/config change、auth revoked、stream protocol error 和 transport fatal error。

## 测试与验收

- 结构迁移验收至少包括：
  - 新代码不再引入新的 `xtcp` 路径与术语
  - 目录与 import 收敛后，既有测试、实验台入口与回归脚本继续可用
  - `roadmap` 与后续新增文档按 `miopunch / brutal` 新命名继续演进
- 单元测试至少覆盖：
  - 传输选择与参数校验
  - 传输实现的选择与接入
  - `brutal` 参数校验
  - 传输层观测事件完整性
- 集成回归至少覆盖：
  - `core-01`：`data=kcp`、`data=quic(quic-cc=bbr)`、`data=quic(quic-cc=brutal)` 均可成功建立并交换 payload
  - `core-01-loss`：复用 `core-01` 的 NAT 基线，作为 derived loss variant；至少验证 `data=quic(quic-cc=brutal)` 在代表性高丢包场景下仍可完成 payload 交换
  - 至少 1 个包含更严格路径或 NAT 组合的代表 case（建议复用现有 `core-06` 或同类样本），确认传输层抽象不会破坏既有建链路径
  - MNT-01 KCP transport specialty：在已建立 UDP path 后，KCP session 必须连续完成 stream-open/hello 与 ping response，不得依赖 handler sleep 或 KCP 专用 linger。
- 回归约束：
  - 现有 `kcp / quic` 的实验台结果不应因 `P3` 抽象而劣化或漂移（除非 change 明确声明并给出证据）
  - 成功 case 不仅要求退出成功，还要求 ordered event assertions、`payload exchanged` 与 artifacts 完整
- `P3` 的验收环境以 `P0` 实验台为主；真实网络与公网协调能力留待 `P4` 验收。

## 实施顺序

- `P3` 的推荐顺序应为：
  1. 仓库结构与命名迁移
  2. `miopunch transport` 抽象落位
  3. `kcp / quic` 适配器重组
  4. `brutal` 接入
  5. `P0` 实验台回归与指标补齐
- 如果结构迁移尚未完成，不应直接在旧的 `xtcp` 布局上继续叠加 `brutal` 相关实现。

## P4 预留边界

- `P4` 负责：
  - 公网可用的最小协调通道
  - 真实网络中的端到端建链与传输验证
  - 对外开源发布前的文档、报告、已知限制与发布门槛收敛
- `P3` 必须为 `P4` 留出空间，但不提前把 `P4` 的公网依赖、产品语义或发布要求强行塞入 `P3` 实现。

## 开放问题

- `P3` 的新包路径如何命名与落位：是否引入新的 `dataplane/` 包，还是以其他目录表达 `miopunch transport` 抽象。
- 传输选择在消息层中采用何种最小表示，才能既支持 `data-proto=kcp|quic` + `quic-cc=bbr|brutal`，又不把 `P3(v1)` 复杂化。
- `brutal` 在 `P3(v1)` 中应开放哪些最小参数，哪些参数必须明确推迟到后续 change。
- `connectivity`（打洞内核）与 `dataplane`（数据面）的能力归属是否要在 spec 层显式拆开：
  - 已决策：打洞内核只承诺“产出可用 UDP 通道 + UDP self-check”；所有传输协议选择与 `payload exchanged` 验收归属 `dataplane`。
  - 后续在 OpenSpec 中需要落实为：对 `xtcp-kernel` 的数据面 requirements 做移除/迁移，并由新的 `miopunch-dataplane` 承接。
- peer transport session 的接口形状、stream-open envelope 和 per-kind authorization schema 需要在后续 OpenSpec change 中细化；本文只锁定分层和生命周期语义。
- 现有 `xtcp` 代码树的收敛应拆成多少步：哪些属于 `P3` 同步完成，哪些应作为独立机械重构提交处理。

> 具体的接口形状、消息结构与迁移步骤，应在后续 OpenSpec change 的 `proposal / design` 中继续收敛。
