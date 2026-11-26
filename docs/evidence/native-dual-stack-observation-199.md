# MioCube 双栈地址观测验收

MIOPUNCH-199 基于 `186bb253`。首次实现提交为 `78464711`，后续修复沿同票提交。

## 已交付行为

原生资源配置通过 `observation_addresses` 按观察站 origin 指定可选 IPv4 和 IPv6
拨号 IP。留空的族使用原有系统 DNS。资源许可、QAD SNI 和证书校验仍使用原 URL。
普通周期 QAD、配置端点、独立融合成员端点以及增强观测窗口使用相同配置。

```json
{
  "observation_addresses": {
    "https://observer.example/": {"ipv6": "2001:db8::10"},
    "stun://observer.example:3478/": {"ipv4": "192.0.2.10"},
    "stun+tcp://observer.example:3478/": {"ipv6": "2001:db8::10"}
  }
}
```

STUN UDP 可以从实际 IPv4 或 IPv6 绑定请求并解析本族 XOR-MAPPED-ADDRESS。
两族的 STUN supersession 分开核算。TCP STUN 选取观察站地址族对应的真实 listener，
复用其本地端口并保留观测连接。增强 TCP 互拨和 IPv6 增强打洞的后续实施分别属于
MIOPUNCH-203、202。

资源读取不再截断前四个 STUN 站点。混合观测在同一实际发送绑定上按目的地址发前去重，
QAD 和 STUN 共用本轮物理观察限额，默认最多 64 个目的地。原生成员保留配置中更低的
观察上限。混合窗口最多同时执行四个站点，失败释放位置并递补，两个独立样本即可推进，
一个有效结果保留为基础候选。查询仍服从原有包数和单站上限，后续批次共享原绝对期限。

周期 QAD 每族保留五个在途站点及原单批八秒机会。配置超过五个时，在同一周期机会内
按本族批次分配时间。它使用同一目的地去重和观察预算，后续站点可以参与。

## 真实入口

`tests/address_observation.rs` 从真实 NativeNetwork 配置端点驱动本地 UDP、TCP 和 QAD
观察站，独立核对站点实收的源地址、本地绑定及收到的映射。

- 双族 STUN 和 TCP 的源端口、按族指定 IP 与留空族系统 DNS 均通过。
  初始证据为 `/tmp/miopunch-199-observation-final.log`。
- 多同族绑定用已有非默认 /32 绑定及一个真实默认绑定。四个静默 STUN 站点均收到请求，
  第五站在 1.503 秒返回 `127.0.0.2:47237`，与本轮固定的实际源一致，原期限为三秒。
  同目的地别名只发一次。日志为 `/tmp/miopunch-199-final-multibind-stun.log`，1 passed。
- 第六个 QAD 站点在五个失败目的地后实际完成原域名握手。普通配置端点和独立融合成员
  端点都取得本族 Fresh 结果，反射的端口等于真实绑定。IPv4、IPv6 均通过。
  可信证书的错误域名仍被拒绝。最终日志为 `/tmp/miopunch-199-sixth-qad-final.log`，
  1 passed。运行产物为 `.artifacts/miopunch-199-delivery.address_observation`。

## 回归与构建

一次完整选定集合保留于 `/tmp/miopunch-199-regression-runtime`。
`public_resources` 11 passed，`udp_observation` 14 passed，库测试 66 passed、3 ignored。
唯一失败 `active_native_budget_deferral_preserves_another_members_original_stream`
已在 MIOPUNCH-196 的 `c1a9a3a8` 基线同样重现，沿用该基线定性。

周期窗口修复后，对受影响集合复验得到 13 passed 和一个慢站点期限回归。
根因为把两族窗口合并核算，已恢复每族五个及原单批八秒。
原 `parallel_qad_includes_a_slow_observer_after_three_fast_reports` 的全部断言随后通过，
日志为 `/tmp/miopunch-199-slow-observer-final.log`。其余有效结果复用。
最终周期测试产物为 `.artifacts/miopunch-199-delivery.udp_observation`。

Native check 516 Fresh / 5 rebuilt，WASM check 293 Fresh / 3 rebuilt，均通过。
收据为 `/tmp/miopunch-199-native-check/receipt.json` 和
`/tmp/miopunch-199-wasm-check/receipt.json`。最后一次原生测试图编译
463 Fresh / 7 rebuilt，收据为 `/tmp/miopunch-199-family-window-build/receipt.json`。
后续周期窗口修改位于原生条件编译区域，WASM 结果继续有效。

## 审查与实测边界

一次独立双轴审查发现实际观测预算 fallback、周期第五站截断以及独立融合端点缺少
Observation map。共同修复后两轴聚焦复核通过。周期计量单位修复也经 Spec 聚焦复核。
未保留已知本票审查发现。

实测宿主为 Linux。TCP 同端口观测仍依赖已有平台端口复用能力，当前复用配置覆盖
Linux、Android 和 Windows。其他宿主未实测。此证据不覆盖后续两票的增强打法或 TCP
协调互拨结果。MIOPUNCH-198 已记录的增强 TCP 冷却尾计数差异继续交给 MIOPUNCH-203。
