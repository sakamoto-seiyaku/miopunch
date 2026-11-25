# MioCube 的 Cloudflare Pages 地址回显实测

2026-11-25，向 Cloudflare Pages 的公网观察端点分别发出 TCP 上的 HTTP/1.1 请求和 UDP 上的 HTTP/3 请求。两次均返回 HTTP 200、正文 `ok`，并回显 Cloudflare 看到的 IPv4 地址与源端口。MioCube 打洞入口还未接入这项探测。 [实测端点](https://observe.jasper1024.com/observe.txt)

## 部署配置

Pages 上传目录只有 `observe.txt`，正文为 `ok`。自定义域名 `observe.jasper1024.com` 经 Cloudflare 代理。域名上的响应标头转换规则匹配以下请求。

```text
(http.host eq "observe.jasper1024.com" and http.request.uri.path eq "/observe.txt")
```

| 操作 | 响应头 | 值 |
| --- | --- | --- |
| 设置动态 | `X-Observed-IP` | `to_string(ip.src)` |
| 设置动态 | `X-Observed-Port` | `to_string(cf.edge.client_port)` |
| 设置动态 | `X-Observed-Protocol` | `http.request.version` |
| 设置静态 | `Cache-Control` | `no-store` |

Cloudflare 在返回静态文件时按当前请求填入观测值。纯静态 Pages 请求按公开规则免计量，免费计划支持这条转换规则。本次检查了实际响应，未核对账户用量面板。 [自定义域名规则](https://developers.cloudflare.com/pages/configuration/custom-domains/#zone-settings) · [动态字段](https://developers.cloudflare.com/rules/transform/response-header-modification/reference/fields-functions/) · [免费规则](https://developers.cloudflare.com/rules/transform/#availability) · [静态计费](https://developers.cloudflare.com/pages/functions/pricing/#static-asset-requests)

## 请求与回包

TCP 用 curl 强制 HTTP/1.1。UDP 用临时 Chromium 配置发起请求，从开发者工具协议独立确认该请求实际走 `h3`，再读响应头和正文。两次请求均禁用客户端代理。

| 实际传输 | HTTP 状态 | 正文 | `X-Observed-IP` | `X-Observed-Port` | `X-Observed-Protocol` |
| --- | ---: | --- | --- | ---: | --- |
| TCP | 200 | `ok` | 有效 IPv4，省略具体值 | `58689` | `HTTP/1.1` |
| QUIC／UDP | 200 | `ok` | 有效 IPv4，省略具体值 | `26589` | `HTTP/3` |

两笔都返回 `Cache-Control: no-store`。端口数值来自 Cloudflare 回包。实验未在公网出口独立抓包核对 NAT 映射，也未用 MioCube 的业务 socket 发起这两笔请求。

同日追加验证了同一域名的两个 IPv4 入口。Chromium 分别拨到 `104.21.40.101:443` 和 `172.67.184.88:443`，两次均确认实际远端、HTTP/3、HTTP 200、正文 `ok` 及有效的 IPv4 和端口回显。一个 Pages 项目已经能提供两个可拨目的地。原生客户端应按当轮 DNS 结果选择两个不同的目的地址，并用原域名做 TLS 校验和 HTTP 请求。独立 Chromium 请求的证据覆盖入口可达性，原生增强轮次还需在同一实际 UDP 绑定上完成两笔取样。

## 在原生取样和浏览器 ICE 中的用途

- **原生 iroh** 控制实际打洞的 UDP socket。同一 socket 发出 HTTP/3 请求后，回显可以作为这个 socket 发往该观察点的一笔公网地址样本。现有增强 IPv4 打洞按实际目的 `IP:端口` 去重，收齐两个独立样本才推进。两个域名如果最终拨到同一目的 `IP:端口`，只算一笔。现有 HTTPS 观察地址会交给 QAD 客户端，接入 HTTP/3 探测需要独立入口。 [取样规则](../adr/0019-miopunch-application-on-p2panda-and-iroh.md#L28) · [观察资源配置](../../src/native_network.rs#L1046)
- **浏览器 WebRTC** 自己管理 ICE socket。网页访问多个 HTTP/3 回显站，测到的是 HTTP 连接的映射，无法提供 ICE socket 的映射。给 `iceServers` 配置自建 `stun:` 服务时，ICE 会从自己的 socket 发起查询，也可以配置多个。MioCube 当前的显式 ICE 配置最多接受两个数字 IPv4 的 STUN 地址，也允许留空。已有可互通的本地候选时可以省掉外部观察服务器，ICE 的对端连通性检查仍使用 STUN 报文。 [MioCube ICE 配置](../../src/rtc_ice_profile.rs#L51) · [浏览器 ICE 入口](../../browser-probe/src/rtc.rs#L71) · [WebRTC ICE URL](https://www.w3.org/TR/webrtc/#rtciceserver-dictionary) · [ICE 连通性检查](https://www.rfc-editor.org/rfc/rfc8445.html#section-2.2)

原生接替现有观察服务的验收需要从实际打洞的 socket 取样，让异网成员使用候选完成经验证的直连和真实业务往返。完全接替 QAD 时，还需要覆盖换网后刷新地址的流程。 [QAD 地址更新机制](https://www.iroh.computer/blog/qad)

## 原生部署模式

这两种模式用于 MioCube 原生节点。节点用本地资源替换表显式选择允许的地址观察、查找、信令和数据中继设施。替换表只列本模式允许的地址，即使邀请码或缓存带有 number0 等公共设施地址，实际访问也受本地许可约束。 [资源替换及拨号门控](../notes/2026-10-01-resource-policy-validation.md)

- **只使用 Cloudflare 的外部设施。** 方案用 Pages HTTP/3 做地址观察。首次联络可用已知且可达的成员地址，或另配实现现有联络协议的 Cloudflare 信令入口。直连验证通过后，成员业务在对端之间传输。若仍需要中继，须有获准的 Cloudflare 数据载体，否则本次业务连接按不可达处理。原生 HTTP/3 观察接入、两端冷启动联络及直连仍待验证。 [受限联络契约](../adr/0005-side-channel-control-bootstrap.md) · [Cloudflare WebSocket 能力](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)
- **只使用用户自己的机器。** 双方可达的 VPS 或具备公网 IPv4／IPv6 入站能力的自有节点，可用作首次联络地址，也可按需部署自有观察站和数据中继。已知可达的成员可直接建连。直连失败时，只走用户许可的自有中继或报告不可达。公网地址对应的地址族和端口需要对端可达。 [自托管资源验收](../notes/2026-10-01-resource-policy-validation.md) · [iroh 自托管中继](https://docs.iroh.computer/concepts/relays)

## u22sz 与 r_proxy 的 P2P 优化实验

用办公网 Windows `u22sz` 和家宽 Linux `r_proxy` 的正常原生业务入口，先记录现有配置下首个匹配回复、首次选中直连及两端业务往返。随后在同一对网络上验证受限资源配置，将首次联络成功、双样本取齐、直连验证和原连接业务往返分别计时。仅当受限配置中实际观察、联络和数据路径都符合许可时，才计入对应部署模式的验收。 [既有双端现场证据](network-demo-140-2026-11-18.md)

2026-11-25 14:33–14:36（台北时间），用已部署的 `046059ad` 原生 demo 开始基线。通过本机诊断入口重新开启 `u22sz` 的节点，`r_proxy` 在线接续。两端先经 number0 的 EU 中继恢复业务，再分别在各自的原 QUIC 连接上选中 IPv4 UDP 直连。Windows 节点就绪到首个有效回复为 5.509 秒。表中从各端首个回复到直连的时间使用各自主机的单调时钟。

| 节点 | 首个有效回复 RTT | 首个回复到选中直连 | 首段有效回复 | 直连后追加有效回复 |
| --- | ---: | ---: | ---: | ---: |
| `u22sz` | 558.9 ms | 47.521 s | 46 | 103 |
| `r_proxy` | 628.9 ms | 48.612 s | 48 | 103 |

`r_proxy` 在业务恢复前增加一次失败。独立读回两端状态时，追加采样均为 0 次失败，连接标识一致，选中的 UDP 路径的 STREAM 发送和接收计数都增长。这轮量到了现有配置从首个业务回复到直连接管的间隔。后续受限配置需要从实际打洞的 UDP 绑定看到两笔 Pages 回显，并核对首次联络和业务路径都符合所选模式。

### 双端诊断复跑

同日用已部署的诊断版复跑，两端沿用原身份和资源配置。按同一轮次 ID 对齐的三轮内，两端各收到两笔 STUN 样本，取样耗时 215–359 ms。u22sz 从发出首个探测包到匹配对端包耗时 5.875–9.328 秒。两端随后都拿到同路径的新鲜 ACK，并完成双端确认。

三轮的退出点都在 u22sz 保留临时 UDP socket 时。它的候选路径以 `Backup` 创建，自动维护分支交换 `SELECTED` 控制消息后立即提交 socket，提交时仍要求该 socket 已经承载选中的路径。提交返回 `winner requires its validated original selected path`，随后本端主动撤销路径，r_proxy 记为对端撤销。r_proxy 使用原有 UDP socket，本端三轮均走到保留阶段正常结束。采样窗口结束时，双方业务在线，仍选中 EU 中继。 [自动维护提交顺序](../../vendor/iroh/src/endpoint/member_fusion/udp/execute.rs#L311) · [临时 socket 提交条件](../../vendor/iroh/src/socket.rs#L498)
