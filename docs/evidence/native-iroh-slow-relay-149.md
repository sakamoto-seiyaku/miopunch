# MIOPUNCH-149 慢 relay 的首拨备用信令

融合端点在首条显式物理 relay 的 QUIC 拨号启动时取一次时钟。它按获准且去重的 relay 候选数和当轮错峰间隔计算 `(N + 1) × Δ`，保留至多两路 relay 的机会。窗口届满且目标公钥尚未通过 QUIC 验证时，端点启动一个获准的受限联系。所有物理 relay 明确失败时提前启用。受限联系、relay 和候选切换共用原绝对截止。完成准入且取得获准数据路径后交付一次，回收其余尝试。

## 物理 relay 与双端业务

以下时间以调用端第一条 relay 起拨为零点，单位为毫秒。两个端点分别读回连接标识，校验对端身份、请求和回复。热 relay 样本在起拨前确认调用端注册，其他样本重绑端点后开始。

| 样本 | 第二条 relay | 备用入口 | 目标 QUIC 身份 | 首笔正确回复 |
| --- | ---: | ---: | ---: | ---: |
| 健康 relay | — | — | 14.0 | 389.9 |
| 第一候选静默，第二条健康 | 252.0 | — | 277.1 | 697.1 |
| 热 relay 在 500 ms 窗口内认证，准入仍进行 | — | — | 217.3 | 1652.0 |
| 迟到 relay，备用 MQTT 静默 | — | 501.5 | 765.8（relay） | 1270.2 |
| 四条均静默，`5 × 250 ms` | 252.3 | 1251.8 | 1265.4 | 1720.2 |
| 排除重复和未获准地址后剩两条，`3 × 250 ms` | 252.2 | 751.9 | 765.1 | 1219.6 |
| 两条、原预算 960 ms，`3 × 约 240 ms` | 241.7 | 721.8 | 734.4 | 到期 |
| 两条物理 TCP 拨号均明确拒绝 | 251.6 | 253.0 | 265.4 | 701.6 |
| 热 relay 本机已注册、目标无回应 | — | 501.6 | 514.7 | 957.5 |

960 ms 到期组的四条 relay 均在原期限内启动，备用窗口未到，期限后新增起拨为零。100 ms 取消组只启动第一条 relay，取消后继续观察 800 ms 没有新候选。撤销组在 14.5 ms 验证了正确目标公钥，收到明确的 `PermissionDenied`，本轮未启动备用信令或交付业务。

正式 NativeClient 转发入口的四静默组使用相同的真实 MQTT 和透明 relay 代理。旧实现 4 秒内未启动 MQTT，转发目标未收到请求。新实现在约 1.25 秒启动 MQTT，在可用 UDP 上交换正确业务字节，整个转发用例耗时 2.155 秒。禁用数据 relay 的重启组独立读回此前持久化、已验证的 IP 和端口。旧端口收到了 1200 字节 QUIC Initial，MQTT 同时参与并实际发布 53 个 QUIC 报文，目标通过新的实际 UDP 路径在 1.123 秒返回 PONG。

## 备用入口

有活跃转发链路的第三个成员优先于同时就绪的 MQTT。两端读取同一个 TLS 导出 attempt 以及原连接上的实际 UDP 路径和新鲜 ACK。禁用接收端 UDP 时，受限联系结束为缺少获准数据路径。

公共入口使用真实 Ergo 和受控 MQTT broker。用例统一采用预锁的 35 秒业务回复截止，先单独运行 IRC，再启用两种信令。逐组核对目标和 PONG。

| 公共入口 | 正确 PONG | 该次 MQTT QUIC 发布数 |
| --- | ---: | ---: |
| 单独 IRC | 1.301 秒 | 0 |
| MQTT 和 IRC 同等就绪 | 0.555 秒 | 61 |
| MQTT 就绪且服务原点与目标不匹配 | 1.136 秒 | 0 |
| 首次 MQTT 发布后断开 broker | 1.306 秒 | 1 |
| MQTT 保持就绪且静默，IRC 建路 | 31.558 秒 | 16 |

公开的原生融合构建器也直接安装 raw MQTT 和 IRC。透明 TCP 门让拨号端 MQTT 停在订阅确认前，远端的合法 MQTT 和 IRC 地址均已发布。修复前先启动未就绪的 MQTT，2501 毫秒后才启动 IRC。修复后只启动 IRC，校验双端身份并收到正确业务回复，MQTT QUIC 发布数为零。

## 复现

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-149-build -- \
  test --locked --offline -p miopunch-native-probe --jobs 2 --no-run \
  --test resources --test native_service_streams --test contact_member_priority \
  --test ordinary_endpoint --test restricted_endpoint
```

在允许真实回环 TCP、UDP 的宿主运行 `member_fusion::backup::`、`member_fusion::`、`fused_member_forwarding` 和原生业务默认组。另用仓库固定的 Ergo 与 `tests/fixtures/irc/ergo.yaml` 将实际监听地址传给 `MIOPUNCH_IRC_SERVER`，运行原生业务中的 `public_contact_selection_preserves_the_confirmed_irc_reply_deadline` 及 `only_configured_irc_contact` 忽略组。

同一个 Ergo 环境可运行 `resources` 的忽略用例 `ready_irc_precedes_a_raw_mqtt_sender_whose_subscription_is_pending`。红绿日志分别在 `/tmp/miopunch-149-raw-mqtt-red-real/` 和 `/tmp/miopunch-149-raw-mqtt-green/`。

初始红组见 `/tmp/miopunch-149-red/real.log`。最终报告在 `/tmp/miopunch-149-acceptance-final/`，完整物理矩阵、10 项原生业务默认测试、3 项真实 IRC 测试、2 项真实成员转发、8 项既有融合、2 项普通端点和 19 项受限端点均通过。WASM 检查复现 MIOPUNCH-148 的两项已有诊断，分别为 `src/connection_coordinator.rs:1043` 的 E0433 和 `browser-probe/src/member_connection.rs:145` 的 E0061。诊断的错误码、消息和位置与 148 回执一致，见 `/tmp/miopunch-149-wasm/`。
