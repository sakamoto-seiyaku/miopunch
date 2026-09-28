## Context

当前 `P3.5` 已引入 internal STUN cn/global 采样与更细的 gather 事件（`gather.stun.start/view.result/result`）。与此同时，lab 的 `xtcp-connectivity-selftest` 仍保留了旧的派生用例 expectation（例如认为 “未配置 STUN” 会出现 `gather.stun.skip`）。

由于 `mlab-xtcp-run --disable-stun` 目前只是“不给 peer 传 `--stun` 参数”，会触发 internal STUN 的默认采样路径，从而不会产出 `gather.stun.skip`，造成 lab gate 失败（尽管实际已能完成 `transport.payload_exchanged`）。

本变更聚焦在“测试/回归与当前实现对齐”，不修改 punching/transport 的主线行为。

## Goals / Non-Goals

**Goals:**
- 修复 lab runner `--disable-stun` 的语义，使其真正显式禁用 STUN（包含 internal defaults）。
- 让 `xtcp-connectivity-selftest` 的派生用例在当前代码下全绿，并继续显式校验 `payload_exchanged`。
- 以最小改动保持事件证据链可读、可机读、可复现。

**Non-Goals:**
- 不改变 `connectivity/` 的 STUN/internal STUN 采样策略本身（仍按既定 P3.5 逻辑）。
- 不调整 `P0/P1` NAT punching baseline 与用例定义。
- 不引入新的外部依赖或对外 CLI/配置的大改动（仅限 lab runner 内部行为与 expectations）。

## Decisions

1) **修复点放在 lab runner，而不是改 expectations 适配 internal STUN**
- 选择：当 lab runner 指定 `--disable-stun` 时，对 peer 命令显式传入“空 STUN 配置”（使 `StunExplicit=true && StunServers=[]`）。
- 理由：派生用例的语义是“验证 direct path 在无 STUN 条件下仍能成功”，因此 runner 的 `--disable-stun` 应禁用 internal STUN defaults；维持 `gather.stun.skip` 作为稳定证据链也更清晰。
- 备选：更新 expectations 去匹配 `gather.stun.result(configured=false)` 等 internal STUN 事件；会让“无 STUN”语义变得模糊且降低用例信号强度，暂不采用。

2) **仍以 ordered event evidence + payload evidence 作为通过条件**
- 选择：保持 `lab/guest/cases/expect/*.events.json` 对关键阶段的 ordered 校验，并继续要求 `stage=transport kind=ok` 的 payload evidence。
- 理由：避免仅凭 exit code 通过，保证回归对“路径选择 + 数据可达”都有硬证据。

## Risks / Trade-offs

- [Runner 语义变化] 可能影响未来新用例对 `--disable-stun` 的直觉 → Mitigation：该 flag 仅存在于 lab harness；并在 spec/文档中明确其行为。
- [事件链稳定性] 后续若 gather 事件名再次演进可能导致 expectation 再次漂移 → Mitigation：尽量让 disable-stun 的证据链只依赖少量稳定事件（如 `gather.stun.skip` + `attempt.*` + `transport.payload_exchanged`）。

