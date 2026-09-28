## Context

当前仓库已经完成多批 POC、Door-2、TCP/wire/coordinator/dataplane 相关变更，`master` 上 Go 包数量和跨模块交互明显增加。用户明确要求本阶段只创建 review change：真正审核代码、运行自动检查、生成问题清单，都必须等到后续 apply 阶段。

本 change 使用 `$dev`、`$go-code-review` 和 `$go-concurrency` 的约束组织 review 计划。由于全量 Go review 会覆盖 goroutine、channel、mutex、WaitGroup、context 和共享状态，apply 阶段必须显式使用并发审查规则。

## Goals / Non-Goals

**Goals:**

- 创建一个 apply-ready 的 OpenSpec change，专门承载当前 Go 代码全量 review。
- 将 review 执行边界冻结为：发现问题、记录问题、复核问题，不修复问题。
- 定义 `findings.md` 的必需格式，确保后续报告可直接进入独立 fix change 的输入。
- 把“创建 change”和“实际审核代码”分离，避免本阶段提前开始 review。

**Non-Goals:**

- 不修改 Go 源码、测试、脚本或 runtime 行为。
- 不在创建阶段运行 `gofmt -d .`、`go test ./...`、`go vet ./...` 或逐文件代码审查。
- 不新增 lint 配置、不引入依赖、不执行 formatter 写回。
- 不在本 change 内修复 `findings.md` 中列出的问题。

## Decisions

### 1) Review-only change，fix 另开 change

**Decision:** 本 change 的 apply 阶段只生成 `findings.md`，不做任何代码修复。

**Why:** review 与修复混在同一个 change 中会让问题发现、风险分级和修复取舍耦合，难以复核。独立 review change 可以先完整暴露问题，再按严重级别创建后续 fix change。

### 2) 创建阶段不开始代码审核

**Decision:** 创建阶段只写 OpenSpec artifact，不运行自动检查，不读取代码做逐文件审查，不生成 findings。

**Why:** 用户明确要求“什么时候 apply，什么时候才真正开始”。这让 OpenSpec change 先成为可讨论的执行计划，而不是隐式执行 review。

### 3) Apply 阶段使用全量覆盖

**Decision:** 后续 apply 阶段覆盖当前 `master` 的全量 Go 代码、测试、`cmd/`、`tools/` 和关键执行脚本。

**Why:** 用户选择全量 Go 代码 review；当前仓库已经跨 control plane、connectivity、dataplane、local API、desktop、lab/tooling 多域累积代码，只看最近 diff 容易漏掉早期引入但未 review 的风险。

### 4) Findings 必须可证明

**Decision:** `findings.md` 中每条问题都必须有严重级别、`file:line`、规则类别、影响说明；无法用具体位置证明的问题不写入。

**Why:** review 报告应服务于后续修复和验收。没有可定位证据的问题容易变成主观意见，无法转成可执行任务。

## Risks / Trade-offs

- [全量 review 范围较大] → apply 阶段按包域分组审查，并先用自动检查缩小机械问题范围。
- [报告过度包含风格偏好] → findings 必须绑定具体规则、影响和行号，无法证明的不写入。
- [review 后修复范围失控] → 本 change 明确不修复；后续按严重级别另开 fix change。
- [创建阶段误触发审核] → tasks 明确把所有自动检查和代码阅读放到 apply 阶段。
