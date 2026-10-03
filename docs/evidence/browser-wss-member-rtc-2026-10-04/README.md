# STEP 6：已保存成员 MQTT/WSS Control-M → RTC

实现选择和复现入口见[实施说明](../../notes/2026-10-04-browser-wss-member-rtc.md)。回执只表示已知当前成员的受控资格；正式默认后端/设施、IRC 成员路线和后续 STEP 7–11 没有由本次定案。

`validation.json` 连接固定 review base、StageA 整合提交、最终源码 SHA-256、执行构件 SHA-256、原始 `/tmp` 回执身份与检查。`wss-member-rtc.json` 保存正常、真实 open-channel 数据丢弃、caller/cancel、材料重载和独立 replica M 的实际结果；`legacy-no-joint.json` 保存严格旧端无共同路径的明确失败；`legacy-permitted-relay.json` 保存 MQTT 起步同 M 的旧端获准 relay 兼容。S5 普通 relay/RTC 和 S2 受限 A 的最终回归各有独立回执。

运行只使用自有 loopback MQTT、正常 TLS/WSS 与 private NSS trust。材料准备独立走既有获准 relay，并在测量前关闭准备端点；这不是 STEP 10 WSS 首次入网资格。正常 WSS M 的原生 IP/relay 地址为零，最终普通路径只有精确 RTC；broker 仍 ready 不代表该 M 可以传 signaling 数据。

持久回执删除 ICE username fragment、原 SDP、邀请/秘密/exporter 等内容；保存 exporter/稳定身份比较布尔值、秘密已保存布尔状态、当前世代和实际 transport 计数。不会从不同端的 stable ID 数值相等推定同连接。

最终独立 Standards/Spec 固定从 `ca3be8bad6887c96022297081ddeafa2e2514364` 审查，包括 StageA IRC/RTC 整合与 STEP 6。必修 findings 已关闭；Standards 保留一项可选浏览器 Control/Ordinary 协调前半段重复，后续可复用，当前不阻塞。原始报告和同 reviewer 最终 recheck 被复制为 Markdown，完整 native 基线与最终受影响重检的 scope 分别列明，不能把不同源码阶段的测试次数相加成一次全量运行。

## 实际最终结果

| 检查 | 最终回执 |
| --- | --- |
| Fresh native 4 bins / WASM / bindgen | exit 0；30.253 / 11.091 / 2.126 秒；53 个源码和 6 个实际构件 identity 当前匹配 |
| WSS 原 M → RTC / cancel / 12 秒 / reload / replica | PASS；38.355 秒总 runner；双方新鲜 ACK、放行前普通帧为零、同 M 私密 stream 和应用 stream 连续 |
| 旧端无共同路径 | PASS；4.976 秒总 runner；严格 v2、无 PC、明确不可达、无句柄、Complete 保留 |
| S5 Ordinary relay → RTC 全故事 | PASS；29.936 秒；包括丢 probe、2 秒 caller、driver/pump 故障及旧 relay 可继续 |
| S2 MQTT 受限 A | PASS；20.965 秒；原受限控制交换及身份拒绝仍成立 |
| 旧端共同允许 relay | PASS；独立 fresh-build 2.871 秒协议运行；两端新鲜 probe ACK、同原 M relay 私密/应用 stream、无 RTC |
| 完整 native 基线 | 271 PASS、0 FAIL、2 个既有 ignore；在最终 legacy helper 修改前冻结运行 |
| 最终修改后的受影响 native | 131 PASS、0 FAIL、1 个既有 ignore；完整 member relay 14 + forward 9、Control/caller/wire/gate、真实自有 IRC/MQTT/resource，native all-targets、WASM RTC/default 全 exit 0 |

最终四故事 wrapper 在所有 runtime 命令 exit 0 后错误索引 S2 旧 schema 中不存在的 `sourceUnchanged`，因此原 aggregate `passed=false` / failure 保留。各故事原始 `passed=true`、outer 53 source 前后相同并当前匹配、6 构件当前匹配；[validation.json](validation.json) 明确记载这个聚合修正，而不是修改原始失败回执或重跑已经通过的故事。

最终真实 open-channel 丢包例在 12.0046 秒返回无句柄；取消例 2.6 ms 返回无句柄，分别实际丢弃 6 / 1 条 open channel QUIC datagram。四个 browser PC 均关闭，迟到检查 callback 已拆除/活动 pump 为零；仍运行 native 的晚期 socket 数恢复到起始 6。预创建 2 秒 caller 在等待 2.2007 秒后不能得到新 TLS 预算。同 IndexedDB 页面重载仍为同身份、Complete、authorization/secret 已保存。
