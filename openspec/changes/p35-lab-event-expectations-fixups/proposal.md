## Why

`P3.5` 引入了 internal STUN 采样与更细的 gather 事件后，现有 lab 的 `xtcp-connectivity-selftest`（派生用例）的 ordered event expectations 与实际输出不再一致（例如仍要求 `gather.stun.skip`），导致 lab gate 失败但链路本身已能完成 `payload_exchanged`。

为了保证实验回归“可复现、可诊断”，需要把 lab runner 的 flag 语义与 expectation 文件同步到当前实现，并补齐最小验证，避免后续迭代反复被测试噪声拖慢。

## What Changes

- 修复 lab runner `--disable-stun` 的语义：应显式禁用 STUN（包含 internal STUN defaults），而不是仅“不传 --stun”。
- 修复 / 收敛派生用例的 ordered event expectations，使其与当前事件序列一致且仍显式校验 `payload_exchanged`。
- 跑通对应的验证门禁（至少 `./lab/host/labctl xtcp-connectivity-selftest`），并记录必要的 evidence（run_dir + 关键事件差异）。

## Capabilities

### New Capabilities
- `miopunch-lab-event-expectations`: 约束 lab runner 的 STUN disable 语义，以及派生连通性用例的最小 ordered event evidence 链。

### Modified Capabilities
- (none)

## Impact

- Affected code/tests:
  - `lab/guest/bin/mlab-xtcp-run`
  - `lab/guest/cases/expect/*.events.json`
- No production protocol changes; only lab runner + expectations + verification updates.

