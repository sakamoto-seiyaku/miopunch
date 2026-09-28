# XTCP Fulltest Report (2026-03-18)

## Command

- `./lab/host/labctl xtcp-fulltest`
- summary: `required_pass=8 required_fail=0 allowed_success=10 allowed_fail=2`

规则：
- 非 `NAT4` 组合（不涉及 `nat4-*` profile）必须成功。
- 涉及 `NAT4` 组合允许失败，但必须能从日志中定位失败阶段（`"stage": ...`）。

## P0 NAT 行为自测（case 全覆盖）

- `./lab/host/labctl selftest`
- summary: `pass=10 fail=0`

| case | A | B | result | artifacts |
| --- | --- | --- | --- | --- |
| core-01 | nat1 | nat1 | PASS | `lab/_artifacts/20260318T023628Z-core-01` |
| core-02 | nat1 | nat2 | PASS | `lab/_artifacts/20260318T023812Z-core-02` |
| core-03 | nat1 | nat3 | PASS | `lab/_artifacts/20260318T023955Z-core-03` |
| core-04 | nat3 | nat3 | PASS | `lab/_artifacts/20260318T024200Z-core-04` |
| core-05 | nat2 | nat4-regular | PASS | `lab/_artifacts/20260318T024426Z-core-05` |
| core-06 | nat4-regular | nat1 | PASS | `lab/_artifacts/20260318T024618Z-core-06` |
| core-07 | nat4-irregular | nat3 | PASS | `lab/_artifacts/20260318T024757Z-core-07` |
| core-08 | nat4-regular | nat4-regular | PASS | `lab/_artifacts/20260318T024958Z-core-08` |
| core-09 | nat4-irregular | nat4-regular | PASS | `lab/_artifacts/20260318T025145Z-core-09` |
| core-10 | nat4-irregular | nat4-irregular | PASS | `lab/_artifacts/20260318T025334Z-core-10` |

## Results (core-01..core-10 × data {quic,kcp})

| case | A | B | nat4 | quic | kcp | artifacts (quic/kcp) |
| --- | --- | --- | --- | --- | --- | --- |
| core-01 | nat1 | nat1 | no | PASS | PASS | `lab/_artifacts/20260318T014224Z-xtcp-core-01-tcp-quic` / `lab/_artifacts/20260318T014313Z-xtcp-core-01-tcp-kcp` |
| core-02 | nat1 | nat2 | no | PASS | PASS | `lab/_artifacts/20260318T014354Z-xtcp-core-02-tcp-quic` / `lab/_artifacts/20260318T014437Z-xtcp-core-02-tcp-kcp` |
| core-03 | nat1 | nat3 | no | PASS | PASS | `lab/_artifacts/20260318T014519Z-xtcp-core-03-tcp-quic` / `lab/_artifacts/20260318T014603Z-xtcp-core-03-tcp-kcp` |
| core-04 | nat3 | nat3 | no | PASS | PASS | `lab/_artifacts/20260318T014652Z-xtcp-core-04-tcp-quic` / `lab/_artifacts/20260318T014746Z-xtcp-core-04-tcp-kcp` |
| core-05 | nat2 | nat4-regular | yes | PASS | PASS | `lab/_artifacts/20260318T014828Z-xtcp-core-05-tcp-quic` / `lab/_artifacts/20260318T014912Z-xtcp-core-05-tcp-kcp` |
| core-06 | nat4-regular | nat1 | yes | PASS | PASS | `lab/_artifacts/20260318T014956Z-xtcp-core-06-tcp-quic` / `lab/_artifacts/20260318T015051Z-xtcp-core-06-tcp-kcp` |
| core-07 | nat4-irregular | nat3 | yes | PASS | PASS | `lab/_artifacts/20260318T015139Z-xtcp-core-07-tcp-quic` / `lab/_artifacts/20260318T015241Z-xtcp-core-07-tcp-kcp` |
| core-08 | nat4-regular | nat4-regular | yes | PASS | PASS | `lab/_artifacts/20260318T015341Z-xtcp-core-08-tcp-quic` / `lab/_artifacts/20260318T015430Z-xtcp-core-08-tcp-kcp` |
| core-09 | nat4-irregular | nat4-regular | yes | PASS | PASS | `lab/_artifacts/20260318T015515Z-xtcp-core-09-tcp-quic` / `lab/_artifacts/20260318T015611Z-xtcp-core-09-tcp-kcp` |
| core-10 | nat4-irregular | nat4-irregular | yes | FAIL(punching) | FAIL(punching) | `lab/_artifacts/20260318T015708Z-xtcp-core-10-tcp-quic` / `lab/_artifacts/20260318T015759Z-xtcp-core-10-tcp-kcp` |

## Notes

- 当前环境 `QEMU` 回落到 `TCG`（未启用 `/dev/kvm`），为避免 QUIC 在慢 CPU 下握手超时，提升了 QUIC `HandshakeIdleTimeout`。
- 为模拟 XTCP “low TTL detect packet 应在公网中途死亡”的假设（实验台 WAN 为单一 L2 segment），NAT profile 在 `raw/PREROUTING` 对 `TTL < 8` 的 UDP 包进行丢弃，避免对端 NAT 产生干扰性 `conntrack` 条目。
