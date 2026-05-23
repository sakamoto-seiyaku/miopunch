## Why

当前主线的控制面消息、招引/批准、以及 punching 协商经常被“JSON + 口头约定”黏在一起，导致：

- 签名输入/验签口径不够稳定（字段顺序、编码差异容易踩雷）。
- 很难把“安全语义”讲清楚：relay/broker 能看什么、不能看什么。
- 后续重组最小闭环分支时，接口与实现容易再次跑飞。

本 change 的目的不是新增功能，而是先把 **POC v1 控制面 wire 与安全语义** 烧死成“只有一种正确做法”，为后续 join/enroll/dial/punch 的 changes 提供稳定地基。

## What Changes

- 定义 v1 控制面 wire：所有 peer-targeted 控制消息统一使用二进制 TLV 编码（MQTT payload 直接发 bytes）。
- 定义 v1 transcript：签名输入固定为 `domain-sep + TLV(fields...)`（字段顺序写死）。
- 定义 `peer_e2e_v1`：sign-then-encrypt 的 sealed-box 语义（X25519 ECDH + HKDF + 单一 AEAD）。
- 定义错误语义：解密/验签/过期/重放等一律丢弃 + 聚合 reason（不引入错误回包）。

## Capabilities

### New Capabilities

- `miopunch-poc-control-plane-wire-format-v1`: v1 TLV wire + transcript + peer_e2e_v1 的固定口径。

### Modified Capabilities

- (none)

## Impact

- 预计新增/收敛的代码落点：`internal/controlplane/` 下的 v1 wire/crypto 封装与单元测试。
- 不包含 join/enroll/dial/punch 的业务流程（由后续 changes 负责）。
