# u22s ↔ r_proxy 当前集成版现场运行

2026-11-21，`94db0fa0` 正式 `miopunch-network-demo` 在两台实际 Linux 主机运行。默认 TCP 和显式关闭 TCP 两组均完成双向成员业务：先由公网 relay 建立原连接，再由公网 UDP 承载；各组约两分钟内原连接保持。默认组实际完成 TCP 地址观察并向对方公网映射互拨，但两端 TCP connect 均超时，没有 TCP 路径验证、保留或选中。

## 产物与环境

- 复用 `/tmp/miopunch-tcp-field-94db0fa0-build/receipt.json` 成功构建：457 Fresh、10 rebuilt，约 56.5 秒。恢复任务后没有重构建。固定产物为 `.artifacts/runs/tcp-field-94db0fa0/miopunch-network-demo`；仅剥离独立部署副本的调试信息。两端上传件均核对为 `ec32df2afdcaa8a0349c93e95d296adc43b77154b7dee602d286e524f8118c0c`。
- u22s：Ubuntu 22 Linux，`172.16.3.11`，默认网关 `172.16.0.1`。r_proxy：Debian Linux，`192.168.4.14`，默认网关 `192.168.4.1`。这是 u22s Linux，不是历史 u22sz Windows。
- r_proxy 开始时根分区已满，恢复后为 7.5 GiB 可用、57% 使用。原 demo 容器已不存在；本轮复用既有镜像 `9cd0fe4e994b` 的 glibc 2.36 运行时，创建自有、限时 900 秒的 host-network 容器，只挂载本轮目录。宿主 glibc 2.31，不能直接代替该运行时。
- 两端均使用新目录 `~/miopunch-network-demo/tcp-field-94db0fa0-20261121/`。正式 demo 的 bootstrap/join 创建并实际准入了独立身份，没有复制在用节点。两端配置只给对方四个公网 relay 提示；LAN discovery、端口映射均关闭。
- SSH 只控制进程、传邀请及读取本地诊断；应用业务由 NativeClient 成员连接发送序号请求并核对对端身份与回复序号。实际 UDP 目标为双方公网地址，不是 SSH 转发、VPN 或本机回环。HTTP 诊断仅监听 `127.0.0.1:18472/18473`。

## 同二进制 TCP 开关对照

复用既有 u22s↔r_proxy `field.py` 正式 demo 助手，调整独立目录、诊断端口和有界采样。先默认运行，再正常停止两端；保留相同身份，仅把两端资源的 `tcp_direct_disabled` 从 `false` 改为 `true`，重新运行。保存的资源 JSON 已核对只有这一项不同。两组顺序运行、第二组保留先前学习状态及运行环境缓存，因此以下时延是单次观察，不是 TCP 开关的性能因果结论。

| 配置 / 业务发起方 | 首个有效回复 | 最后成功数 | 首次回复承载 | 后续承载 |
| --- | ---: | ---: | --- | --- |
| 默认 TCP / u22s → r_proxy | 6.344 s | 116 | euc1 relay | 约 10 s 起 UDP |
| 默认 TCP / r_proxy → u22s | 5.433 s | 117 | euc1 relay | 约 10 s 起 UDP，短回 relay 后再回 UDP |
| 关闭 TCP / u22s → r_proxy | 5.555 s | 114 | relay | UDP，期间短暂切回 relay |
| 关闭 TCP / r_proxy → u22s | 5.112 s | 112 | relay | UDP |

默认组两端均为 0 次业务失败。关闭组正常业务阶段未记录失败；收尾先停止 u22s 时，r_proxy 的在途第 113 次请求收到 `peer_application_close`，约 19.9 ms 后记录一次 `read error: connection lost`，故其原始最后样本为 `failures=1`、`phase=recovering`。保留这条原始记录；没有把它解释为 3 秒网络超时或验证停止之后的恢复。

各端每组只打开一个业务原连接，计数始终为 `connections=1`。默认组本地 stable ID 为 u22s `139656499487696`、r_proxy `139730184994288`，关闭组为 `139707434523520`、`140653334274928`。每组路径切换前后保持本地原 ID，序号继续增长，选中 UDP 路径的 STREAM 收发计数也增长。demo 每次请求使用新的双向流；此次证明原连接延续，不是单个长流恢复。两端公网 relay 一直可用，短暂 relay 选中期间仍收到业务回复。

## TCP 实际运行

默认配置允许 TCP direct、未禁用 TCP 地址，并含获准观察资源 `stun+tcp://turn.cloudflare.com:3478/`。`ss` 记录了数据监听器；正式日志记录如下：

| 节点 | 真实监听 / 观察源 | 观察目的 | 返回公网映射 | 实际互拨目标 |
| --- | --- | --- | --- | --- |
| u22s | `0.0.0.0:46411` / `172.16.3.11:46411` | `141.101.90.1:3478` | `182.48.107.243:2222` | `111.197.236.230:10988` |
| r_proxy | `0.0.0.0:29311` / `192.168.4.14:29311` | `141.101.90.1:3478` | `111.197.236.230:10988` | `182.48.107.243:2222` |

两端均有 `TCP_OBSERVED`、`TCP_PUNCH_START` 和 `TCP_CLOSE ... TCP connect timeout`；路径验证阶段约 6 秒后失败，没有 `TCP_PUNCH_VALIDATED`，业务路径没有 TCP。TCP 观察成功不等于公网互拨可达；本轮没有两端 WAN 抓包，不能确定失败在主机策略、哪层 NAT 或途中网络，原因保持未知。后台 TCP 尝试失败期间默认组原业务没有中断。

关闭组没有数据 TCP Custom 地址、TCP 观察或互拨记录；只保留 HTTP 诊断所需的回环 TCP 监听。没有为取得 TCP 结果修改系统路由、防火墙、观察服务或延长产品期限。本轮没有封锁 UDP，因此没有重演受控双 NAT 的 UDP 故障后新增 TCP 恢复，也不作为 Windows MIOPUNCH-160 验收。

## 附带现象与证据

r_proxy 两组都记录 `contact-learning-failed: node data is incomplete or invalid`。demo 将回复中的完整地址交给 `offer_contact`；u22s 默认公告 13 个地址、关闭 TCP 后 12 个，而 `src/network_maintenance.rs` 的 `offer_contact` 拒绝超过 8 个地址。该方向的主动线索学习没有成功，但本轮已建立业务继续工作。本轮仅记录，没有修改产品实现。

本地原始证据目录为 `/tmp/miopunch-tcp-field-94db0fa0/`：`artifact.json`、`comparison.json`，以及 `default/`、`disabled/` 下各端完整 stdout/stderr、`status.jsonl`、`status.json`、资源配置、socket 观察和 `summary.json`。四个正式 demo 业务进程均正常退出；仅本轮自有容器已停止并自动移除，独立上传产物与节点目录保留。在用身份、系统网络、Plane、Goal 均未修改；没有 push。
