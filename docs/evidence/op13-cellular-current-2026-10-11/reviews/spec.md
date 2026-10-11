# Spec review: returned OP13 current SDK evidence

Reviewed `29b5a2ce...783e9ac843d71d99890fa4583e6110c42edb50df` against cached M84 Plane body and the user's relay/MQTT, relay-only without punching, IRC-last request. Exact diff/log are retained beside this report. Earlier review through `e361f45a` is carried forward. No actors, builds, tests or repository/Plane changes were performed.

**Finite deliverable: zero required corrections; zero scope creep or wrong-evidence findings.** The dossier accurately separates successful relay/IRC stories, failed MQTT data paths, observer/fixture corrections and unknown startup causes. Product SDK/lock/profile/policy remain unchanged.

Independent checks matched all559 SDK source hashes,57 published executed-input copies and originals,75 raw references, all successful Android selected-input/copy/captured-depfile graphs and deployed artifacts, and3159 WASM inputs/six package artifacts. All11 public case fact/network projections match retained raw evidence.

**Updated whole-M84 audit:**

- The earlier current nonempty-Replica gap is closed for the exercised direction. Spec: “选择已验证方向完成首次准入/重开/非空 replica”. Both relay stories actually verify/save/apply four records and verify them after reopen; RTC-disabled relay creates zero PC/DC. Actual member/Replica callers match6355.765/9870.730ms and6757.530/9910.890ms.
- **Directional/physical qualification remains partial.** Spec: “Native→Native、Browser→Native、Native→Browser、Browser→Browser及其反向…” and “无 VPN NAT 四层资格”. CELLULAR/WIFI defaults and zero VPN agents are observed before/after, but exact SDK UID/mark/socket/interface correlations and the complete direction matrix remain unqualified; every physical-WAN flag stays false.
- **Current mobile MQTT/RTC business remains failed.** Spec: “公共 MQTT 完整 SDK” and “所有未通过项保留 failed/blocked/unavailable”. Admission/save/reopen passes, but Replica fails NoData23295.775ms and ordinary member fails NoData22179.935ms. Neither signaling nor successful relay substitutes for these data-path results.
- **Complete SDK terminal retirement remains partial.** Spec: “清理…记录即时与晚期 socket 回收”. Successful relay Native close retains four sockets at immediate/100ms observation. IRC's500ms socket check fails before later actual closure2306.410ms later; complete callbacks remain unproven despite natural owned cleanup.

Actual IRC request30163.5ms, approval1, durable identity/material reopens, strict member fresh ACK/original M and PONG independently match raw evidence. Explicit60s/8s/30s remains outside full-default-profile certification. Prior finite carrier/no-path/entry-fallback and protocol-contract results retain their original scopes; full audit is in `acceptance-audit.md`.

The finite user request can complete. M84/M69 must remain In Progress.
