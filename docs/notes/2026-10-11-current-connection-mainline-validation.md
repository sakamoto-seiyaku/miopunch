# 当前连接主线有限验证与 M84 验收收拢

本轮完成用户选定的“先做 1、2”：少量当前 SDK 代表故事，以及基于
`29b5a2ce` 的独立 Standards/Spec 审查和 M84 完整验收审计。没有新增 UI、
Rust 功能、选路默认值或设备网络配置。

公共 relay 在 RTC Answer API 缺失时通过真实入网、审批、材料保存、重开、
无获准数据路时报 DataPathUnavailable、恢复 APS1 后成员业务。Host / Pixel
浏览器双向 MQTT/RTC 也通过，实际 caller 约 8.3–11.1 秒；双方显式预算为
60 秒，原探针 CLI 12 秒默认保留。首轮 12 秒失败原因仍未确定，后续成功
不足以证明只是预算问题。

自有 relay 转 RTC 保留原 M/binding/stream；关闭唯一真实 RTC carrier 后
同 M 修复约 577 ms，超过修复 cap 的后续业务仍正常。RTC 无路用例实际等待
原 60 秒 caller，新增 PC/DC 退休后原 relay 与业务 stream 继续。三个实际
Native SDK 成员的联系入口回退在原 12 秒 deadline 内用 MQTT 控制交换并
转为 UDP 业务，caller 523 ms。IRC 复用 M89 已资格化的当前构件和完整故事，
重核验源码与 pins，未重复运行未变场景。

这证明当前有限流程仍可运行。M84/M69 的全方向当前实机/无 VPN 异网矩阵、
当前非空 replica、完整默认 IRC profile 和完整 socket/task 终止关闭仍有
缺口。部分 relay/MQTT 尾部在观察窗口后仍 CLOSING，明确保留
`allSdkTerminalTasksRetiredProven=false`，不能称所有 socket 已自然关闭。
旧完整 contract 与 OP13 记录仍属于当时构件，不能换标为当前全矩阵。

本轮修正探针实际 caller 预算证据，并保持 SDK/锁图/profile 不变。前后源码、
实际编译输入、构件及 234 份既有 WIP hash 均核验。所有首失败、独立清理
计时与计数、公共资格边界见
[完整 dossier](../evidence/current-connection-mainline-2026-10-11/README.md)。
任务状态和剩余验收归 Plane M84/M69，本文仅保存本轮结果。
独立 Standards 未发现违规或可操作 smell；Spec 未发现需要修正的源码/证据
问题，完整验收仍保留上述三类缺口。有限执行与审查范围完成，总验收保持进行中。
