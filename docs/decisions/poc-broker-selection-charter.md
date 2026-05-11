# POC Broker 选择与 Invite/Join 收口纲领

## 文档状态

- 本文档定义 POC/RC 过渡阶段的 broker 口径、边界与关键决策。
- 本文档是后续 broker 相关实现与评审的权威基线，不替代后续 OpenSpec change。
- 旧讨论中出现过 `control_plane.brokers` 等未落地口径；当前准则统一以本文档与同步后的 OpenSpec 为准。

## 背景

- 首次启动的空白节点在产品体验上既可能是“准备创建新网络”，也可能是“准备加入现有网络”。
- 便携包和跨网络测试表明，`invite/join/approve` 看起来成功并不代表后续 `ping` / `sh` 一定还能工作；如果 post-join signaling 又退回到各自旧的默认 broker，就会再次分裂。
- 当前仓库里的 broker 语义分散在 `local.mqtt_broker`、`invite_brokers`、`brokers_effective`、桌面端 first-run owner candidate 以及历史讨论文档中，已经不足以指导后续实现。

## 核心决策

- first-run 空白节点可以在桌面端表现为 owner candidate，但这只是 UI 引导语义，不代表已经创建或加入了网络。
- 显式 broker 配置入口继续使用 `local.mqtt_broker`；不新增新的配置字段名。
- `local.mqtt_broker` 的后续实现口径需要兼容旧单字符串与新数组表达；新写回统一为数组。
- 若 `local.mqtt_broker` 有显式配置，则 broker 选择只从该配置列表中进行；不使用内置默认列表。
- 若 `local.mqtt_broker` 没有显式配置，则才允许从内置 broker 列表中选择默认值。
- `brokers_effective` 表示当前网络实际运行的 broker 主/备列表，最大长度固定为 `2`。
- `invite_brokers` 表示 invite/join 阶段专用入口集合，最大长度固定为 `2`，并在条件允许时优先避开当前 `brokers_effective`。
- `membership_bundle` 必须把 owner/admin 当前正在使用的完整 `brokers_effective` 传给加入节点；join 之后的运行时 signaling 必须围绕这组 broker 工作。
- POC 这一轮的最小选择规则是“reachability 优先、来源顺序稳定”；不在本轮引入基于 RTT 的复杂排序或周期性全网 broker 重选。

## 目标

- 为首次建网、invite 生成、join/approve、post-join signaling 提供唯一一致的 broker 语义。
- 保持 POC 在跨网络、便携包和真实机器场景中的可操作性与可解释性。
- 把“first-run owner candidate”“invite 专用 broker”“网络实际主/备 broker”三层语义明确拆开。

## 非目标

- 不在本轮引入全网定时 broker 重选或复杂主备漂移策略。
- 不在本轮引入 MQTT 用户名/密码、TLS、证书分发等 broker 鉴权能力。
- 不在本轮把 MQTT 升级成 data-plane relay。
- 不在本轮通过新增字段名来替代 `local.mqtt_broker`。

## 行为准则

### 1. First-run 与 owner candidate

- 空白首次启动节点可以暴露 create-invite、approve、join 等入口。
- 这个状态只表示“本机可以发起建网流程”，不表示本机已经拥有 `net.json`、governance head、decls 或 `brokers_effective`。
- 成功 `invite/create` 之后，本机才成为真实的 genesis owner/admin。
- 成功 `join` 之后，本机应按加入后的普通 member/peer 语义工作，不再保留 owner candidate 解释。

### 2. `local.mqtt_broker` 与候选来源

- `local.mqtt_broker` 是用户显式 broker 配置。
- 兼容口径：
  - 旧格式：单字符串 `host:port`
  - 新格式：字符串数组 `["a:1883", "b:1883", ...]`
  - 新写回：统一写数组
- 若显式配置存在：
  - 配置 `1` 个，就只使用这 `1` 个。
  - 配置 `2` 个，就使用这 `2` 个。
  - 配置超过 `2` 个，就从这些显式配置中筛选出最多 `2` 个。
- 若显式配置不存在：
  - 才允许使用内置 broker 列表作为默认候选源。
- 内置 broker 列表只是“无显式配置时的默认候选源”，不是伪装成用户配置的默认值。

### 3. `brokers_effective`

- `brokers_effective` 是当前网络实际运行的主/备 broker 列表，最大 `2` 个。
- 选择过程：
  - 对候选端点做规范化与去重。
  - 做 reachability 探测。
  - 从可达候选中按来源顺序选出最多 `2` 个。
- 若只有 `1` 个可达候选，则 `brokers_effective` 允许只有 `1` 个。
- 后续 acceptor、`ping`、`sh`、`bootstrap_more`、seed peer 传播都必须围绕 `brokers_effective` 工作。
- 运行时语义：
  - 第 `1` 个为 primary。
  - 第 `2` 个为 secondary。
  - primary 失败时可以尝试 secondary。
- 节点不得在 join 之后退回到各自旧的默认单 broker 语义。

### 4. `invite_brokers`

- `invite_brokers` 是 invite/join 阶段专用入口集合，最大 `2` 个。
- 候选源与 `brokers_effective` 一致：
  - 若有显式 `local.mqtt_broker` 配置，则只从该配置列表中选择。
  - 若没有显式配置，则从内置 broker 列表中选择。
- 选择规则：
  - 先排除当前 `brokers_effective`。
  - 再从剩余候选中选出最多 `2` 个可达 broker。
  - 若排除后数量不足，则允许回退复用当前 `brokers_effective`，避免 invite 无法生成。
  - 若最终只剩 `1` 个可达 broker，则 code 中只写 `1` 个。
- `join/approve` 在 invite 阶段只认 code 中的 `invite_brokers`，不得再自由切回本地默认 broker。

### 5. membership 传播与 post-join signaling

- `membership_bundle` 必须携带 owner/admin 当前完整的 `brokers_effective`。
- `join` 成功后，加入节点本地的 signaling state 必须更新为完整的 `brokers_effective`，而不是只保存第一个端点。
- `approve` 成功后，approver 保存 joiner peer config 时，也必须保存 joiner 的完整 `brokers_effective` 视图。
- 后续 hello / seed peer / peer config / runtime dial 都必须能表达并消费 primary/secondary broker 语义。
- `invite_brokers` 只服务于入网交换；join 成功后，后续运行时 signaling 不再以 invite broker 作为长期语义。

## 实施约束

- 本文档只定义行为，不规定具体 Go 结构体或 JSON 兼容细节的代码实现方式。
- 但后续实现必须满足：
  - 旧单字符串 `mqtt_broker` 状态可继续读取。
  - 新实现写回统一为数组。
  - peer config、seed peer、membership/hello 相关载荷需要同步扩展到可表达完整 broker 列表。
- 任何继续把 `brokers_effective` 简化成 `brokers_effective[0]` 的实现，都不再符合本文档。

## 与其他文档的关系

- `openspec/changes/improve-first-run-role-ux` 负责 first-run owner candidate 的 UI 边界。
- `openspec/changes/fix-effective-broker-and-topology-status` 负责当前 broker 收口、post-join signaling 修正、desktop target 状态与 acceptor 日志。
- `docs/notes/2026-04-16-alpha-glossary.md` 负责术语定义，并应引用本文档。
