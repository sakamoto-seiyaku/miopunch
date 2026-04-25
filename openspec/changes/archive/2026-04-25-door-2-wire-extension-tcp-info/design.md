## Context

Door 2（TCP 打洞）方向已选定“最小入侵”的 wire 演进策略：在既有 NAT-hole 消息中增加并列字段表达 TCP（例如 `tcp_direct_addrs` / `tcp_mapped_addrs` / TCP STUN view），而不是复用字符串前缀或整体替换结构（见 `docs/decisions/door-2-tcp-punching-charter.md`）。

本 change 创建前的仓库事实：

- NAT-hole wire（`internal/wire/messages.go`）仅承载 UDP 相关字段：`direct_addrs` / `mapped_addrs` / `assisted_addrs`，以及 request 侧的 `stun_cn/global` 观测与 response 侧的 `selected_view/reason`、`candidate_addrs` 等。
- coordinator（`internal/coordinator/nathole_controller.go`）会基于 cn/global STUN view 做仲裁，然后派生出 `candidate_addrs` 并下发 `detect_behavior` 等 UDP punching 行为。

本 change 创建时的定位是：**只扩展“信息携带/回传/派生”能力**，不引入 TCP gather / TCP attempt / TCP dataplane。

后续仓库已经通过 Door-2 TCP direct punching 相关 change 落地了 TCP gather / attempt / dataplane 入口的部分行为。因此，本旧 change 的工件只描述早期 wire-extension 边界；若与当前主规范（尤其 `miopunch-tcp-p2p-v0`）冲突，当前主规范优先生效，本 change 不应回退或禁止后续已确认的 TCP 行为。

## Goals / Non-Goals

**Goals:**

- 为 NAT-hole wire 增加可选 `tcp_*` 字段，使 TCP candidates 与 TCP STUN view 能在控制面端到端传递。
- coordinator 在不改变既有 UDP 行为的前提下：
  - 回传对端的 `tcp_direct_addrs`（`peer_tcp_direct_addrs`）
  - 派生 `tcp_candidate_addrs`
  - 在双方都提供 TCP cn/global 观测时，复用现有 view arbitration 逻辑产出 `tcp_selected_view/reason`
- 所有输入视为不可信：统一 `strings.TrimSpace`，并用 `net.SplitHostPort` 做 host:port 过滤；最终 compact 去重。
- wire 兼容性：不新增 message type byte；新增字段全部 `omitempty`，默认缺省。

**Non-Goals（本 change 创建时的历史边界）:**

- 不修改 `connectivity.Gather` 去真实产生 TCP candidates 或 TCP STUN 观测（后续 change 处理）。
- 不实现 TCP punching attempt（simultaneous-open、喷射预算、winner 收敛等）。
- 不实现 TCP dataplane（TCP 路径的数据面协议栈）。

## Decisions

### 1) 字段命名与放置：并列 `tcp_*` 字段

**Decision:** 在既有 NAT-hole message struct 中新增并列字段（snake_case），不引入新 message type。

- request（`NatHoleVisitor`/`NatHoleClient`）：
  - `tcp_direct_addrs` / `tcp_mapped_addrs`
  - `tcp_stun_cn` / `tcp_stun_global`
- response（`NatHoleResp`）：
  - `peer_tcp_direct_addrs`
  - `tcp_candidate_addrs`
  - `tcp_selected_view` / `tcp_selected_reason`

**Why:** 最小入侵、可读、可 diff，且与现有 UDP 字段的心智模型一致。

### 2) 可选嵌套对象使用指针，避免 `omitempty` 输出空对象

**Decision:** 对 `tcp_stun_cn/global` 与 `tcp_detect_behavior` 采用指针类型：

- `*STUNViewObservation`（与 UDP 侧一致的结构复用）
- `*TcpDetectBehavior`（本 change 创建时 reserved；后续主规范可启用）

**Why:** Go 的 `encoding/json` 对非指针 struct 字段的 `omitempty` 不会省略 `{}`；使用指针可以保证“默认缺省时完全不出现字段”。

### 3) coordinator 的 `tcp_candidate_addrs` 派生规则

**Decision:** 复用 UDP 侧 view arbitration 逻辑，但输入改为 TCP 侧字段。

- 若双方同时具备 `tcp_stun_cn` 与 `tcp_stun_global`，则：
  - 计算 cn/global aggregate，选择 `tcp_selected_view/reason`
  - 使用 selected view 对应的 `MappedAddrs` 作为候选源（再过滤/compact）
- 否则回退到 request 侧的 `tcp_mapped_addrs` 作为候选源（再过滤/compact）

**Why:** 使 TCP 信息携带具备与 UDP 一致的“可解释 view 选择”形态，同时不要求本 change 先落地 TCP gather。

### 4) reserved 字段的历史边界：本 change 不启用，后续 change 可启用

**Decision:** 本 change 创建时只冻结 `tcp_punching_enabled` / `tcp_detect_behavior` 等字段（wire 合约），当时 coordinator 不设置它们。当前仓库已经允许后续 TCP punching 规范按 `miopunch-tcp-p2p-v0` 设置这些字段；本旧 change 不再作为“必须保持缺省”的限制。

**Why:** 早期阶段通过缺省字段降低误用风险；后续 TCP attempt change 可以在同一字段集合下继续演进，并由当前主规范定义何时设置这些字段。

## Risks / Trade-offs

- [旧节点兼容性] → 不新增 type byte，保持 JSON “未知字段可忽略” 兼容；新增字段默认为缺省，不影响旧路径。
- [部分实现导致误解] → 本 change 的历史阶段只做携带/派生；当前是否设置 TCP 行为字段由后续主规范定义。
- [消息体积增大] → 全部字段可选且默认省略；在没有 TCP 信息时不会增加额外开销。
