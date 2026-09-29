# MIOPUNCH-26：只剩未知合法成员在线的启动链验证

票面：[MIOPUNCH-26](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/16d83d01-545f-4611-a075-5061bfbcfed4)。本文件记录一个有限的真实 MQTT 验证 profile；不决定默认公共 broker、最终调度策略或产品 UI。既有网络身份、准入秘密和成员授权分别按 `CONTEXT.md`、ADR 0004、0007、0008 处理。

## 可复用流程

`src/mqtt_discovery.rs` 以网络根公钥和秘密代际计算 SHA-256 命名空间。A 在随机 128 位回复 topic 上订阅成功后，向该命名空间的 `query` 发送 `MDQ1 || nonce`。在线 C 收到后，以非 retained、QoS 0 发布 `MDR1 || nonce || node_id || MQTT_route || IPv4 || UDP_port`。A 只在两秒窗口内接收与当前 nonce 匹配的最多 16 个候选；新查询换 nonce，已退出的响应器不再回答新查询。topic 和回复均不是凭证。这个发现 profile 不预置 C 的公钥或全员名单；A 从候选取得目标公钥并对 iroh `EndpointAddr` 固定它。

`src/member_bootstrap.rs` 只在同一真实 TLS 连接上的 `SCRAM-SHA-256-PLUS` 双向证明通过后继续。证明绑定网络及代际，但只授予受限引导资格。`ManagementStore` 将当前代际及准入秘密的作用域指纹保存于 SQLite；历史交付实时核对证明所用凭证与这份持久当前状态，旧代际或同代际错秘密均不能因成功完成过期证明而取记录。准入秘密原文由调用者持有，指纹表不代替其保存。服务端再检查本地持久成员视图中的请求者，开放已验证的 UDP 数据路径，向获准成员发送完整已应用管理记录。接收端以自己已钉住的根公钥和 canonical Create 哈希重放全部签名记录，检查双方当前成员资格，然后保存、应用记录。已知撤销身份在请求前被拒；持有尚未轮换的共享秘密的新身份可看见公开候选、完成证明，但不能凭此取得记录或业务句柄。业务句柄仍受数据路径和成员授权限制。

## 可见字段与权限边界

| 观察者 | 可见内容 | 不因此获得 |
| --- | --- | --- |
| MQTT broker、topic 订阅者 | 命名空间、随机 nonce、回复 topic、候选节点公钥、临时 MQTT 路由、IPv4/UDP 地址、报文大小和时序；MQTT 报文载体上的 QUIC 密文及公开 Initial 元数据 | 准入秘密、邀请码原文、TLS 内控制流、管理历史或业务权限 |
| 纯转交者 | 被转交的上述公开线索与密文报文；若其本身是成员，仍只按自己的成员权限读取记录 | 代替端点证明秘密、签发成员资格或解密另一对端点的会话 |
| 回答查询的候选端 | 查询所在命名空间及 nonce；建立 TLS 后看见对端认证节点身份；证明成功后按自身已应用历史核验对方 | 因收到查询就取得请求者的历史、邀请码或准入秘密 |

命名空间的哈希只避免在 topic 直接写根公钥和代际，并非保密承诺；知道根公钥和可猜代际者仍可推算。随机回复 topic 也不是访问控制。未知候选阶段的公开查询和回复发生在 TLS/SCRAM 之前，因此其真实性仅由后续 TLS 端点身份、SCRAM 和签名成员记录建立。

## 真实检查与失败结果

在隔离的 `eclipse-mosquitto:2` broker（本机 `127.0.0.1:43262`）运行：

```sh
rtk proxy docker run --rm -d --name miopunch-26-mqtt -p 127.0.0.1:43262:1883 -v /home/js/Git/miopunch/scripts/mosquitto-test.conf:/mosquitto/config/mosquitto.conf:ro eclipse-mosquitto:2
rtk proxy env MIOPUNCH_MQTT_BROKER=127.0.0.1:43262 cargo test --locked --test unknown_member_bootstrap -- --nocapture --test-threads=1
rtk proxy docker rm -f miopunch-26-mqtt
```

第一项真实检查：A 的持久视图只含自己和离线 B；B 是签发 A、C 成员记录的原审批者，整个实验没有启动 B 的端点。C 的视图还含自己。A 经真实 MQTT 找到未知 C，双方完成 iroh TLS 身份认证与绑定连接的 SCRAM，C 在数据路径上提供完整签名记录，A 验证并持久应用新增 C 记录。输出 `UNKNOWN_MEMBER_BOOTSTRAP ... discovered_c=1 tls=1 scram=1 signed_history=1 new_records=1`。

优先失败检查：无准入秘密的假候选能发布身份及路由，但证明失败；尚未轮换的秘密持有者用新节点身份可完成证明，却被服务端成员检查拒绝管理历史；已知撤销身份无论作为请求方还是被查询方均被拒。在线响应器仍运行时，错误网络与错误代际的查询没有候选；轮换且重开 SQLite 后，已获授权的 C 用旧代际或新代际配旧秘密完成证明，服务端仍拒绝历史并拒绝把持久代际设回旧值。停止响应器后，抓取旧回复并真实发布到新随机回复 topic，旧 nonce 使其被丢弃。独立通配订阅逐条核对查询、回复的 topic、nonce、节点公钥、路由和 IPv4/UDP 字段以及报文载体封套，断言所有 topic/负载均无准入秘密原文；连续运行抓取量在 50–57 条之间。测试没有生成邀请码，固定格式的公开封套也没有邀请码字段。

此验证使用本机 UDP 直连作为获准数据路径，并保留相同 iroh Connection；它不证明 NAT 成功率、默认资源可用性、多候选抗拥塞能力、成员历史在全网已最新、或最终产品的重试调度。测试固定 QoS 0、两秒收集窗口和最多 16 个候选，只是受限场景参数；后续运行时整合需在资源预算下选择具体配置。
