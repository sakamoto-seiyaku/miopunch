## 1. Spec + Design

- [x] 1.1 补齐 `miopunch-dataplane` 的 requirements 与场景覆盖（`kcp`、`quic(bbr)`、`quic(brutal)`）。
- [x] 1.2 收敛 `xtcp-kernel` 与 `miopunch-dataplane` 的能力边界：哪些 requirements 迁移、哪些保留引用关系。
- [x] 1.3 在 design 中固化 QUIC fork 的选择与版本钉法（对齐 HY2 最新 release）。

## 2. QUIC Stack Migration (P3 implementation)

- [x] 2.1 将代码中的 QUIC 依赖统一迁移到 HY2 使用的 QUIC fork，并钉死版本。
- [x] 2.2 确认 `control plane` 与 `data plane` 的 QUIC 都使用同一栈（方案 A）。

## 3. Data Plane Integration (P3 implementation)

- [x] 3.1 `data-proto` 保持 `kcp|quic`，新增 `--quic-cc bbr|brutal`（默认 bbr）。
- [x] 3.2 `brutal` 模式接入 `up/down` 带宽参数（P3 固定 `up=1, down=1`）。
- [x] 3.3 `exchange` 阶段强制双方一致，否则直接失败；不引入自动协商与降级。

## 4. Lab Regression + Observability (P3 implementation)

- [x] 4.1 `core-01` 覆盖 `kcp`、`quic(bbr)`、`quic(brutal)` 三种数据面模式，并显式验证 `payload exchanged` 证据链。
- [x] 4.2 `core-01-loss` 至少覆盖 `quic(brutal)`，验证高丢包下仍可完成 payload 交换。
- [x] 4.3 更新 lab runner 透传 `--quic-cc`，并确保 artifacts / 事件记录能区分 `bbr` vs `brutal`。
