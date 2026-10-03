# p2panda 授权讨论基础

日期：2026-09-28。性质：有界源码研究，**不是产品决策**。用户明确要求以 p2panda 已有机制为讨论基础。本笔记重新核对已有研究的授权部分，不从头设计同步或冲突算法。

后续产品取舍见[管理记录决议](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/2c7388f6-d5f5-4c73-9de7-cdc261855096)；本文保留为源码依据，不代替该决议，也不重开已确认的创世权限与日常管理员分工。

证据范围：官方文章，以及本机 `/tmp/p2panda-0.7.1` 已有源码副本中的实现和测试；版本链接固定为 `v0.7.1`。本轮网页工具成功读取官方文章，但未能重新拉取 GitHub 标签页面；未运行测试，未独立重新验证本机副本的下载来源。

## 先分清三个层次

1. **记录传播、保存和补齐**：现有 `core/store/stream/sync` 提供签名操作、存储、依赖处理与日志同步基础。它们不自动决定谁是 miopunch 管理员。现有 [行为缺口研究](2026-09-28-p2panda-behavior-gap-research.md) 已列出发布版接线边界。
2. **授权状态收敛**：`p2panda-auth` 已经实现组成员及权限变更、因果图和默认冲突处理。同一组操作最终形成一致的组状态，是这个模块要解决的问题。[模块说明](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/lib.rs#L5-L70)
3. **应用历史怎么处理**：知道某成员被撤权后，是只拒绝后续应用消息，还是重算之前的应用数据，是另一个集成选择。官方明确区分这两种模式；采用 auth 并不自动要求回滚所有应用历史。“收到后生效、接受传播窗口”也不能直接用来替代组管理操作自身的收敛规则。[官方集成讨论](https://p2panda.org/2025/08/27/notes-convergent-access-control-crdt.html#integration-with-applications)

## 已有机制，而非需要用户发明的规则

**事实：因果依据已经存在。** 操作声明依赖，组成 DAG；`GroupCrdt::process` 先按该操作声明的历史状态校验作者权限，再处理并发影响。依赖需先处理，这不是按到达顺序或墙上时钟决定胜负。这里的“先于”是因果关系，不是必须证明现实时间。[实现与前置条件](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/crdt/mod.rs#L450-L559)

**事实：默认采用 strong removal。** 管理员移除或降权会使其并发管理动作失效；互相撤权存在保留双方撤权的特殊处理，可能导致没有管理员。依赖失效授权产生的下游管理结果也会受到影响。这些是现成 resolver 的行为，不是 miopunch 这轮新提案。[默认规则](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/lib.rs#L61-L70)、[官方实例](https://p2panda.org/2025/07/28/access-control.html#concurrency-resolver)

**事实：重新加入已有实现，不需要先创造“任期协议”。** 源码测试 `remove_readd_dependencies_filter` 中：Alice 移除后重新加入 Bob；Bob 与移除并发批准的 Dave 不在最终成员中；Bob 获知重新加入后批准的 Eve 在最终成员中。Eve 的操作还引用了旧的无效操作，因此不能把“传递失效”简化成“凡是图上后代全部失效”。应以 resolver 的授权重算为准。[测试](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/resolver.rs#L711-L794)

**事实：并发 re-add 也有现成过滤。** `compute_filter` 明确过滤同组中与移除并发的重新添加操作。合法重新加入本身受 `add` 的历史状态校验约束：目标在该历史状态下不能已经是活动成员。因此“管理员批准一个尚未获知移除的现有成员”不能不经核验就当成有效的新入网操作。[过滤实现](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/resolver.rs#L188-L240)、[add 校验](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/crdt/state.rs#L183-L232)

## 真正需要与产品要求对照的缺口

**事实：内建角色不是“创世权 + 日常管理员”。** 内建级别是 Pull、Read、Write、Manage；普通管理变更检查作者是否活动且拥有 Manage。`add/remove/modify` 没有“只有创世权可以任免管理员”的独立检查；`remove` 还允许活动普通成员移除自己。[访问级别](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/access.rs#L12-L47)、[管理校验](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/crdt/state.rs#L183-L325)

**事实：自定义 conditions 不等于已经实现创世权限制。** `Access<C>` 支持条件和权限比较，但组状态修改上述分支主要检查 `is_manager()`，不能仅给 Manage 附上条件便宣称它会限制管理员任免。另一个限制是该版本明确拒绝把 group 添加或提升为 manager，不能直接用“管理员子组”绕过。[条件比较](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/access.rs#L131-L167)、[ManagerGroupsNotAllowed](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/group/crdt/mod.rs#L574-L590)

**推论：讨论顺序应先理解默认行为，再核对差异。** 当前应对照现成模型能否承载创世权保留、日常管理员边界和恢复语义；这不等于已经决定自定义 resolver、修改库或放弃产品要求。自定义 resolver 是扩展点，不是已完成的产品适配。

**尚未验证：** 创世凭证与节点身份分离的具体接法、在默认 resolver 下保留创世权的完整方案、管理员降权后再升权的全部边界、受限成员证明交换与连接准入、实际乱序与重启测试。本笔记不对此作可行性承诺，也不把源码测试的存在当成本项目测试通过。
