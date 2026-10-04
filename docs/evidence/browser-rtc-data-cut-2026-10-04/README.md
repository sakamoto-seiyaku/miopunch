# STEP7：RTC 数据断路的原 M 维护回执

[实现与兼容说明](../../notes/2026-10-04-browser-rtc-data-cut.md)。当前设计与评审固定基点为 `1434334cfc15d8231257883a6fecd8761bda79d7`；本目录记录其后的 STEP7 实施。范围为一座自有隔离网络命名空间内的 Chromium/Linux native、正常 TLS 和已保存的当前成员，不代表 NAT/WAN、IRC、TURN 或 RTC 自然 idle expiry 资格。

最终 [receipt.json](receipt.json) 的两轮 kernel case 与 review-fix affected suite 具有相同的 50 项 runtime 源文件身份。两轮执行的三个 native binaries、WASM 和 bindgen JS 五项产物 hash 完全一致。较早完整 151 tests 单独保存为 pre-review baseline；其后候选继续规则、Noq 同义 predicate 提取、实际清理失败分类与公开 regression 已按受影响范围重新检查。各轮 `sourceBefore == sourceAfter`，真实 case、命令退出和自有清理均通过。编译与 bindgen 独立计时，不计入 60 秒协议/fixture 总预算。

| 最终当前源码检查 | 实际结果 |
| --- | --- |
| 允许 relay、实际 DROP 选中 RTC UDP | 修复 783 ms / caller 2000 ms，新鲜 watermark 9 + peer confirmation；原 stream 4 的序号 0、1、2、两端各自 stable id 与 exporter 比较布尔值保持 |
| 同轮显式取消 | attempt 总耗时 508 ms；发出 cancel 到观察返回为 5 ms；受限 control 实际关闭，原 M 和材料仍保留 |
| 同轮短 caller | 120 ms caller 在 120 ms 返回 deadline/unreachable；随后同 M 正常维护成功 |
| 没有允许 fallback | 497 ms 返回 no mutually permitted data candidate；原 M `2707760` 仍活，原待完成 stream 不前进，无数据 relay 或原 M 的 signal 数据路径 |
| 两轮 terminal | 明确主动关闭原 SDK 句柄之后撤销 DROP；UDP 返回后旧句柄只返回同旧 stable id 的 terminal；none 旧 stream 实际失败 |
| native review-fix affected | 当前 88 tests，含 19 path maintenance、真实较严格启动策略的 mixed candidate、新鲜 ACK、原 stream、旧 wire、当前 gate/proof、caller/RTC/foreign-M guards；较早完整 151 tests 保留为 baseline |
| vendored Noq | 当前相同源/manifest 的独立副本完整 429 unit + 3 doc tests；实际编译 5.79 秒、unit runtime 7.60 秒 |
| 构建与静态检查 | native all-targets、WASM rtc-probe、cargo fmt、fork rustfmt、git diff --check 均退出 0 |

允许 relay 的最后 native/browser 本地 M id 分别为 `129635132120880` / `2460960`。新受限 control 的 id 可以不同；它的 ALPN 和 restricted scope 单独记录，不能被当成第二条普通 M。repair 与原业务分别按路径计数，业务只能在原 M 的获准 relay 上增长。

| kernel 证据 | relay | none |
| --- | --- | --- |
| 初始实际 DROP | 11 packets / 1619 bytes | 15 packets / 2119 bytes |
| 结束实际 DROP | 58 packets / 7399 bytes | 25 packets / 3344 bytes |
| runtime / 含清理 protocol 总计 | 6.730 / 9.698 秒 | 5.577 / 8.618 秒 |
| native/WASM/bindgen 准备 | 15.374 / 9.689 / 2.171 秒 | 0.465 / 0.365 / 2.177 秒（同一源码与产物缓存，仍实际调用并核对 hash） |
| 实际 decoded pcap | 2 packets，全部匹配 selected IPv4/UDP tuple | 3 packets，全部匹配 selected IPv4/UDP tuple |
| browser PC | 1 created / 1 closed，callbacks detached | 1 created / 1 closed，callbacks detached |
| native RTC | PC closed，TX/RX queue 0，owned pump 0；仍活原 M 上立即/晚期 associations/pumps 均 0、sockets 均 7，原 selected UDP socket 已消失 | 终止后的 native cleanup PC closed、TX/RX queue 与 owned pump 均 0 |
| 自有设施退出/移除 | native fixture 和 relay 退出 0；精确 nft table removed，container/network remove 0 且 actual absent | native fixture 退出 0；准备 relay 在测量前退出 0，无数据 fallback；精确 table/container/network 同样已移除 |

pcap 为 `any` interface、64-byte snaplen、300 包上限、`--immediate-mode`，只解码 Linux SLL2 + IPv4/UDP 头部及截断加密载体；[relay capture](relay-rtc-selected-tuple.pcap.gz) 与 [none capture](none-rtc-selected-tuple.pcap.gz) 的原始 SHA-256 写在 JSON 中。两轮都要求非空，所有实际 packet header 必须匹配该 selected tuple 的一个方向。选中 IPv4/UDP tuple、实际相反的两端地址、精确 nft INPUT/OUTPUT 规则与初始/结束 counter 分开保存；kernel DROP 计数记录断路，pcap 证明实际载体四元组。没有宿主 firewall/route 变更；ENODEV 的 mDNS join 仅通过自有 namespace 的 `224.0.0.251/32` 路由修正，没有放宽 driver、TLS 或 caller。

## 实际失败与修复阶段

这些阶段保存其真实状态，不作为最终当前源码资格：

- [Noq narrow red](noq-backup-red.log.gz)：在已验证 Backup 可用、原 Available 实际断路、两端普通数据仍排队的真实 Noq fixture，移除 ACK-first 后新鲜 probes 实际失败。最终对应测试同时要求 Backup STREAM/DATAGRAM 不增长。
- [legacy TCP diagnostic red](legacy-diagnostic-red.log.gz)：原 M alive，protected contact 和新 TCP path open 成功；新 selected path 1 的 fresh probe watermark 4 未收到 ACK，而旧断路 Backup 0 的 PATH_ACK TX 从 17 增到 28。真实 exact test 0 pass / 1 fail，4.80 秒。
- [dialing intermediate green](legacy-intermediate-dialing-green.log.gz) 和 [accepting intermediate green](legacy-intermediate-accepting-green.log.gz)：先用较宽 own-ACK predicate 恢复两个 public 原 stream 故事；随后把作用域收窄为 validated Backup ACK-only。**当前资格使用最终 review-fix 88-test affected run、当前 Noq suite 与两轮 kernel run**，不把较宽版本通过算作较窄版本已经通过。
- 首个全 path target 的 7 个 broker `NotPresent` 是缺 fixture 环境的失败，不算产品 red 或 suite pass；后续 proper owned-broker suite 暴露 16 pass / 2 个真实 legacy TCP 回归，再被上述实际修复和 pre-review 151-test baseline 覆盖，本次 review-fix 88 tests 再次覆盖相关修复与 guards。
- 第一次归还独立 Noq red 副本源码时保留旧 mtime，所得旧执行 trace 不匹配当前 line identity；该轮不作绿色证据。最终副本的显式新 mtime、源码/manifest/lock hash 和真实编译/完整通过结果单独保存。
- 捕获修正前，短 none case 的 libpcap 读缓冲只产生 header-only 文件；该轮的真实协议/计数通过不作为合格 pcap。当时捕获修正阶段加入即时读取和 decoded 非空 tuple 校验，重新执行两轮 kernel case，生产 Rust 与五项执行产物保持该阶段前后的相同身份。其后评审修改的 current core/Noq/Python 身份及最终两轮重新资格化另行完整记录。

评审后另有两项真实回归：

- [stricter startup mixed red](strict-startup-mixed-red.log.gz) 为公开 `Builder::hooks` 策略与实际 AdmissionGate 的交集：真实 UDP Member M 的资源仍允许 TCP，但这个原 M 拒绝该 TCP 候选。protected mixed offer 中评审前 core 提前返回、0 pass / 1 fail、runtime 1.90 秒；[对应 green](strict-startup-mixed-green.log.gz) 继续候选后 1 pass / 2.02 秒、同原 M/stream。当前 88 tests 再次覆盖该条件。最初 import、未知 member 地址及 ordinary M commit 三个 setup failures 不作为实现 RED；这里不宣称默认端点必然出现该拒绝。
- `cleanupFaultRegressions` 保存实际完整 owned fixture 后的 absence-query timeout/非零 CLI 故障。修正前 timeout 保存 true 且 outer CLI 0；return-code 故障保存 false 但 outer CLI 仍 0。修正后同两种故障都保存 false 并 outer CLI 1，仍继续后续精确 container/network 移除。这里 GREEN 表示失败分类正确，不能当作普通 qualification PASS；正常当前两轮 case 均 exit 0 且实际 absence 为 true。

Noq 的当前 fix 让验证后的 ACK-only Backup 只发送自己的 PN-space ACK，readiness/encoding 使用同一 predicate，实际 emitted keys 驱动 `acks_sent` 与 timer。Available、pending unqualified RTC 和 handshake 保持原 cross-path ACK 行为。旧 available 路仍可能在 Backup 被现有 pacing/congestion 阻塞时消耗该 ACK；这一条件未单独资格化，本次没有新增 CC 策略或更长 deadline。

## 保存与复现

[affected.log.gz](affected.log.gz) 保存当前 review-fix 88 tests 的逐 case 结果；[151 baseline](affected-151-baseline.log.gz) 与其 source identity 分开保存；[noq-final.log.gz](noq-final.log.gz) 保存 429 + 3 的当前身份副本结果。JSON 中保留各实际命令、源与产物 hash、预算、原 stream/本地 id、用途和资格布尔值、清理、物理 tuple 与失败分类；不保存 exporter、SDP、ICE 凭据、DTLS fingerprint 或入网材料内容。

```sh
rtk proxy python3 scripts/check_browser_paths.py --case rtc-data-cut --cut-mode relay --output /tmp/mio-rtc-s7-relay
rtk proxy python3 scripts/check_browser_paths.py --case rtc-data-cut --cut-mode none --output /tmp/mio-rtc-s7-none
```

完整 source-pinned transient inputs 为 `/tmp/mio-rtc-s7-reviewed-relay-20261004`、`/tmp/mio-rtc-s7-reviewed-none-20261004`、`/tmp/miopunch-s7-review-fixes-affected-20261004` 与 `/tmp/miopunch-s7-noq-review-fixes-provenance.json`。root dispatcher 保留同阶段的独立 readback；本实施不关闭完整十一阶段目标。
