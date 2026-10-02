# MIOPUNCH-49 independent Spec review

Reviewed `4185f55caf70bf884a90cbde806d207caf582dba...5a780dbc8dc5bf241a2674583ad36deda1313a31`
against the actual MIOPUNCH-49 and parent MIOPUNCH-14 descriptions saved from
Plane in `/tmp/mio49-plane/{49,14}.txt`, the current design baseline, and
`final-suite/results.json`. Spec axis only; no source edits or additional tests.

1. **P2 — Recovery failures after successful QUIC handshakes do not accumulate
   backoff.** MIOPUNCH-49 says “连续失败后退避” (line 9) and “失败退避”
   (line 14). In `src/resources.rs:595`, every successful handshake resets
   `failures` to zero. The new coordinator then calls `contact_failed` when
   path/history recovery fails or remains incomplete
   (`src/network_maintenance.rs:503`, `:540`). Consequently, repeated attempts
   to the same unchanged reachable endpoint whose recovery continually fails
   each schedule only the initial one-second contact delay. The exploration
   floor still bounds effort, but these protocol failures never progress through
   the recorded 1/2/4/8/16/32-second backoff. Retain recovery failure history until
   actual recovery succeeds, separately from raw transport-connect success.

2. **P2 — Losing the final synchronized neighbor can publish stale Connected
   state.** MIOPUNCH-49 requires “所有入口失败时持续可解释等待/退避” (line 18);
   MIOPUNCH-14 requires “得到暂不可达状态” (line 152). In the new existing-neighbor
   sync-error branch (`src/network_maintenance.rs:571`), the coordinator removes
   the neighbor and schedules contact backoff without changing the state. With
   the failed contact not yet ready, no dial updates it. The final fallback at
   `:705` runs only when `last_sync` is absent, while `publish` at `:444` has no
   empty-neighbor fallback. Thus a node that previously synced successfully can
   expose `Connected` with zero neighbors while waiting after the last session
   fails. Historical source completion must not suppress current unreachability.

Both findings follow deterministic control flow; the eleven passing acceptance
cases do not cover these two transitions. No other confirmed specification gap
or scope creep was found. Linux/native qualification and unchanged NAT-test
omissions match the recorded boundary and standing user instructions.
