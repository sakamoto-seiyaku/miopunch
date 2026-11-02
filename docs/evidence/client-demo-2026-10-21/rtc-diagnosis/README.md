# 用户 RTC 建路反馈与重复探测修复

2026-10-21，用户报告 Edge 经 VPN 回到单位网络时，实际业务仍选中 Relay。
任务为 [MIOPUNCH-111](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/3bfb70d1-8c0b-4c5b-9c9f-916c3a05bcdd)。
此前 [MIOPUNCH-110](../rtc/README.md) 的跨网络成功回执仍有效，但不能证明该用户的
Edge/VPN 网络也能建立直连。

## 用户证据可以确认的内容

- 成员身份及授权有效，实际普通成员路径是 Relay。
- `rtcCapability.effective: true`：所需 PeerConnection/DataChannel API 检查通过。
- `activeRtcPrepareProgress.stage: data-channel-open` 且 `completed: false`：
  正在等待通道打开，并未完成。随后候选已清理，没有 RTC 业务包传输。
- Linux 当时只保留 `unified-active-result` 的 `invalid-data` 分类；supervisor
  丢弃了更具体的 RTC 失败日志，无法从旧记录确定 ICE、DTLS 或 SCTP 的具体失败。
- 用户确认使用 Edge 和 VPN、出口 IP 在国内；尚不能据此认定 VPN 是原因。

后续[用户重试的双端记录](user-confirmed-ice-stage.json)已进一步确认阶段：浏览器两次均在
218～303 ms 内收集完成，实际存在 UDP 公网 `srflx` 候选，随后进入 checking，
约 16.3 秒后连接失败；Linux 同时具备公网候选，但一直 checking 到 caller 超时，
未选中候选对，DTLS 仍为 Unspecified。所需 API 和 STUN 地址观察均有效，问题范围
已缩小到 ICE 候选间连通性；尚不能定位到特定 NAT、VPN 规则或具体网络跳点。
浏览器数组首个始终 new、无候选的条目是能力探测 PC，不应与后续实际协商混淆。

用户再次点击探测期间的[定向 STUN 头部观测](user-stun-header-summary.json)记录到 Linux
向其实际公布的公网候选发出 64 个 Binding 请求，未在该接口看到来自该公网地址的
STUN 请求、成功响应或错误响应。观测开始前，用独立 STUN 请求/响应验证了双向捕获；
过滤只保留目标 IP 的 62 字节 STUN 头，临时容器完成后自动移除。
这确认 Linux 已使用交付的候选进行检查，但本轮没有建立往返，尚不能据此指认具体
VPN、防火墙或 NAT 跳点，也不覆盖对端改用其他出口 IP 的情况。业务继续使用 Relay。

Native webrtc 0.21 的统计实现只对所选候选对补充部分计数，SDP 直接加入的远端候选
也未统一进入统计注册路径。因此失败统计中没有 remote-candidate/pair 条目本身，
不能证明信令没有交付候选；定位继续依据真实状态和定向 STUN 头部往返记录。

## 已复现并修复的独立缺陷

| 缺陷 | 修复与验证 |
| --- | --- |
| 已有健康 RTC 时再点探测，重复申请同一成员会话的关联并报 DataPathUnavailable | 复用当前所选 RTC；同一 Native stableId、同一关联继续工作，界面无错误 |
| 探测失败后显示一直停在“正在重新探测直连” | 无论成败都刷新真实路径和最近尝试；完成后恢复按钮，故障用例显示 Relay 且成员 RPC 仍可用 |
| 实际失败原因被过度压缩 | 浏览器新增有界、只读且不持有 PC 的观察；Linux 在普通失败和外层取消两条路径记录真实 ICE 事件，失败统计最多额外等待 250 ms，路由先退休 |

[修复前正常 RTC 的重复探测](before-healthy.json)及
[修复前人为限制 ICE 的界面](before-unavailable.json)保留实际错误。
[候选正常检查](candidate-healthy.json)和
[候选故障检查](candidate-unavailable.json)通过。
[原 u22s 入口的上线检查](live-healthy.json)也通过，包含实际 RTC 和重复探测。
故障用例在隔离测试浏览器内要求 relay-only ICE、同时不配置 TURN；API 仍可用，
但实际无可用 ICE 路径。它验证失败处理，不用于断言用户 VPN 的实际策略。

Native [真实故障回执示例](native-failure-sample.json)为 `ice: checking`、候选收集完成，直到 deadline 超时；
现在可以保留这些阶段，而不是只剩 `invalid-data`。
正常 SDK 模式和故障测试的界限由检查中的 case 字段记录。

## 浏览器日志导出

按用户要求，连接区加入“导出诊断日志”，无需开发者工具即可下载 JSON。
记录最近 200 条连接元数据、时间戳、探测原始错误、浏览器环境、ICE 状态/候选及
两端当前路径；不采集业务 RPC 正文、剪贴板、文件或终端内容，不导出访问凭据、
邀请、密钥、完整 SDP 或 ICE 凭据。IP/端口与公开节点标识保留以便匹配网络观测。
未连接可导出；采集超时最多等待 2.5 秒，然后下载最近已有记录并标明采集错误。
浏览器记录只在本次页面内存中保存，复现后应直接导出。

[下载检查](browser-log-export.json)验证未连接、真实 ICE 无路、broker 失联的导出，
并验证原服务上线后的 JSON 下载及成员 RPC。人为 ICE 无路可能返回
`DataPathUnavailable` 或 caller `DeadlineExceeded`，日志保留实际错误分类。
最初下载检查正确导出了 `DeadlineExceeded`，但检查器仅接受前一分类而失败；
放宽为两个实际有效分类后通过，未为检查改变产品失败行为。
本次仅更新浏览器静态资源；Native 容器/启动时间不变，无 Cargo、SDK 重编或节点重启。

## 更新与边界

更新保持原 `u22s:18443` / 内容端口 `18444`、Native 身份、访问凭据及用户状态。
节点密钥和原用户文件/剪贴板仅比较更新前后哈希，未复制内容。
旧应用备份为 r_proxy 的
`/home/js/miopunch-demo-before-M111-20261021/old-app.tar.gz`。
更新后需关闭全部旧演示标签页，重新打开原完整链接，才能加载新窗口内的诊断。
[最终健康检查](health.json)确认临时测试浏览器、候选和隧道已停止，交付服务及原转发
正常运行，部署哈希一致。

只修改 demo 探测调用/显示及只读故障观测，没有更改协议、SDK 默认策略、
Cargo profile/锁文件或部署 TURN。WASM 固定产物及 3162 项输入复用不变。
两次 Native 源变更各自构建一个图；第二次补齐外层取消的观察，每次均为
526 Fresh，仅本包库、demo、中继三个目标重编。运行用例复用最终固定产物。

来源与状态见 [inputs.json](inputs.json)。原始日志、源码快照及构建记录保留在
`/tmp/miopunch-rtc-user-failure-20261021/`。
本记录不宣称用户 Edge/VPN 直连问题已修复；已定位为该对端点的 ICE 往返未成功，
具体网络跳点仍需网络侧对照。
运行与检查方法见 [demo 说明](../../../../demo/README.md)。
