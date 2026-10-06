## Spec

Reviewed the exact frozen four-file candidate against current M76 and ADR 0009; patch and all four source hashes match. No tests, builds, SDK/device operations, Git or Plane actions were run.

**1 qualification evidence gap; 0 required code fixes; 0 scope-creep findings.**

1. **Missing qualification: API-available RTC network failure → permitted Relay.** M76 requires: “API 可用但网络 RTC 建路失败是另一种运行结果，交 coordinator 回退，不能将它写成节点离线或修改成员身份。” The new controller's three modes deliberately make RTC unavailable at startup, and its Native actor explicitly disables RTC. Their successful Relay stories therefore cannot prove fallback after an actual RTC network-establishment failure, or preservation of identity/online state through that transition. Current M77 evidence may supply this separately if it actually demonstrates the required transition. M76 remains incomplete; this is a remaining qualification requirement, not an identified defect in these four files.

Current API-missing, PC-denied and DC-denied receipts all pass with actual PING/PONG, original-M matching, current gates, strict version/purpose, fresh ACKs and preserved durable identities/grant. API-missing additionally proves healthy MQTT plus typed DataPathUnavailable, zero MQTT QUIC bytes and successful new attempt after Relay restoration. DC-denied reports the truthful cumulative two PCs created/closed and zero DCs. Committed R83 separately supplies explicitly disabled RTC; no old projection is counted as current evidence.

The cleanup criterion says: “清理本票拥有的连接、PC、pump、队列、listener、临时进程及转发；记录即时与晚期 socket 回收，强制终止不得写成 graceful。” The candidate satisfies the reviewed ownership scope through closed/freed member handles, retired MQTT queues/callbacks, RTC objects/pumps, and mandatory final tab/store/ADB/listener/observer/Native-group retirement. The configured Relay's real CLOSING tail retains four essential callbacks, zero buffer and unchanged traffic through 100 ms; its task/receive queue remain explicitly unproven. It is neither published as CLOSED nor graceful. This is a documented observation limit, not grounds to detach callbacks required by the pinned library's close future.

No wrong implementation found: MQTT public-close detachment fixes retained-carrier ownership; Native ordinary M waits for consumer/endpoint closure so a queued reply survives setup completion. Original caller 12 s and Native Relay 6 s setup limits remain intact. Both fixes and the narrow evidence controller fit M76 scope.

Owned jobs: 0.
