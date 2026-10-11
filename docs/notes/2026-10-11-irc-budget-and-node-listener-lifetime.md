# IRC 显式预算与节点监听寿命

2026-10-11，用户要求继续修复 IRC；若问题来自延迟，可以给足延迟。
实施工作记于 [MIOPUNCH-87](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/baf08657-c7c4-4ebd-9e8c-3d91ddad33e4)。
本记录补充 ADR 0009 的预算与 listener 规则，不指定新的默认公共服务名单。

当前实施保留 60 秒默认总预算，将显式 caller / maintenance 预算上限设为
300 秒。BrowserNetwork 正式启动采用输入或已保存的 maintenance 配置作为
后来独立请求的预算；每次请求、握手、共同协议与材料交付继续消费同一个
absolute deadline 的剩余时间。没有 caller metadata 的握手仍使用 60 秒
有界默认。输入 policy 在创建 startup timeout 前验证，非法大数不能先进入
timer。显式短预算及 cancellation 保持有效。

正式 Browser IRC 联系入口持续监听，独立于某次请求及节点已经运行的年龄。
Native 显式 facility 使用新增的 maintained factory；原有 resource constructor
本来就保持节点级监听。独立 datagram 实验和原 bounded facility constructor
继续受 profile attempt 寿命限制。取消、明确关闭与权限检查仍可以退休入口；
这不是延长已失败请求，也不恢复退休的 generation。

IRC profile 可以显式保存较长的有界 queue TTL 和 attempt 值，上限为 300 秒；
默认 profile、datagram 大小、分片、队列数量、assembly TTL 上限与 pacing 上限
不变。当前 Ergo 验证显式使用 pacing 600 ms、assembly TTL 8 s、queue TTL
30 s、caller 300 s。它们是这项实际服务资格的配置，不能称为全局默认或 RFC
要求。60 秒请求默认没有因此获得同样的慢服务成功承诺。

因果回路保留了 60 秒失败、180 秒仍失败，以及只提高 queue TTL 后握手继续
推进但 bounded listener 提前退休的回执。两端入口 maintained 后，owned 慢速
IRC 实际请求约 218 秒完成；当前公网 Ergo 的两次严格 version1/purpose2
请求约 250 / 249 秒完成，并由 Native 保存真实未批准请求。仅收到 TLS 或
私密控制 gate 不能算成功。

随后实际批准全新测试请求，经显式获准的 data relay 交付测试材料、保存并
重开双方身份，再以当前 member/1 会话收发 PING/PONG。IRC 保持受限联系用途；
普通 STREAM 的资格来自成员 gate 与实际获准数据路径。以上不覆盖 M84 尚未
完成的物理异网全矩阵。

socket 发起关闭、SDK close/free 返回，以及浏览器实际收到 WSS close event
是不同观察。500 ms 未收到事件必须保留；清理以独立有界晚期观察确认，不能
把销毁页面后的 socket 消失称为自然 SDK socket 回收。临时诊断不进入最终
产品构件，原始回执及 pinned SDK 保留在私有证据目录。
