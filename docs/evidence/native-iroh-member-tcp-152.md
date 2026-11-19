# MIOPUNCH-152 原流迁移到已有原生 TCP

两端先经真实 HTTP relay 完成 TLS 和成员准入。原业务流开始发送数据时，融合端点在该 QUIC 连接的单向流上交换已安装且获准的 TCP 地址。通配监听可通过现有成员联系入口提供具体可达地址。原 QUIC 拨号侧使用现有路径所有者发起验证。该物理绑定拿到原连接的新鲜 ACK 才可承载业务并进入现有分档选路。候选验证超时会释放本轮路径，业务继续在获准的健康 relay 上收发。

[双端用例](../../tests/support/member_fusion_tcp.rs)关闭 UDP 直连，对照以下六种场景。每组核对原连接与原流标识、连续的请求和完整回复字节。双端拨号计数始终为 `[1, 0]`，额外业务及控制拨号为零。

| 场景 | TCP 实际观察 | 原流结果 |
| --- | --- | --- |
| 获准且可达 | 双端读回各自实际 TCP 监听元组、PathId 和新鲜 QUIC ACK，后续业务的 QUIC 路径与两个真实载体的接收字节均增加 | 由接受端开启的原流迁移，随后完成 16 轮已选 TCP 上的字节往返 |
| 一端禁用 | 在两侧仍有测试用 TCP 监听器的情况下，两个载体的收发均为零 | 继续经健康 relay 返回完整字节 |
| 地址明确不可达 | 独立 TCP 连接和实际载体的拨号均收到 `ConnectionRefused`，没有接收帧 | 原流在候选截止后仍经 relay 收发 |
| TCP 前缀匹配，原路径验证失败 | 两个真实 TCP 套接字交换路由前缀及后续帧，测试在拨号端将回帧留在 TCP 层，原 QUIC 候选无新鲜 ACK | 原流在候选截止后仍经 relay 收发 |
| QUIC 路径已建立，候选 ACK 暂缓 | 两端读到同一候选的 `PATH_RESPONSE`，候选 ACK 和普通 STREAM 帧均为零，释放后同一路径出现真实 ACK 和 STREAM 帧 | 暂缓期间继续通过健康 relay 完成 16 轮完整往返，释放后原流经 TCP 继续 |
| 两端通配绑定，提供明确地址 | 在原业务启动后通过成员联系入口提供该监听器的实际回环 IP 和端口，双端取得 TCP 路径 ACK | 复用同一连接，继续完成 16 轮原流往返 |

初始红测在两端通过 relay 取得首个业务回复后，等不到原连接上的 TCP ACK，且两个 TCP 载体的收发计数均为零。审查后的两项红测分别复现了通配地址无法迁移，以及已收到 `PATH_RESPONSE` 的候选在没有 ACK 时提前替换 relay。日志位于 `/tmp/miopunch-152-red/runtime.log` 和 `/tmp/miopunch-152-review-red/runtime.log`。修复后六个场景的日志位于 `/tmp/miopunch-152-review-green/tcp-fusion.log`。

融合 TCP 六组、既有原生 TCP 12 项、普通端点 2 项和受限端点 19 项通过。WASM 检查复现 [MIOPUNCH-149](native-iroh-slow-relay-149.md#复现) 中相同的 E0433 和 E0061，两条诊断的消息、文件与行号一致。回执位于 `/tmp/miopunch-152-review-wasm/`。

## 复现

构建后，在只启用回环的临时网络命名空间内，用 Cargo 回执中的 `resources` 可执行文件运行 `member_fusion::tcp:: --test-threads=1 --nocapture`。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-152-build -- \
  test --locked --offline -p miopunch-native-probe --test resources --no-run --jobs 2
```
