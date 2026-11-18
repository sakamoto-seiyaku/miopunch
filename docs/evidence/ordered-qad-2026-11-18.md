# Ordered QAD failure recovery (MIOPUNCH-137)

The ordered observation API now skips an individual failed QAD observer and continues in configured order. Only distinct resolved destinations count as independent samples; fewer than two successful destinations still fail classification and let the SDK use its permitted STUN fallback. Each observer is retried on a subsequent call. No persistent observer blacklist, new dependency, or larger probing allowance was added.

Observation remains within the SDK's existing round permit and absolute caller deadline. Permission withdrawal, endpoint shutdown, and a physical network-generation change cancel pending ordered work. The endpoint emits each observer's outcome and elapsed time through the library's existing structured DEBUG tracing event convention (`iroh::_events::qad::ordered_observation`), subject to the application's subscriber and level selection. The initial mapping Offer/Plan wait now uses the intersection of the existing round deadline, snapshot expiry, and caller deadline: the former five-second wait otherwise rejected a peer still completing two three-second QAD timeouts before STUN. Later winner validation/commit clocks are unchanged by this fix.

## Reproduction and runtime checks

The new real-endpoint regression first failed on the original code after 3.17 seconds with `TimedOut: mapping observation timed out`. After the fix, the unavailable first observer is skipped and four subsequent independent destinations are retained. Restoring the failed observer produces five samples in the next round.

Builds used the shared cache and committed profiles through `scripts/cargo_cache.py`, with `--locked --offline`, limited to the affected test targets and existing observation tool:

- `/tmp/miopunch137-red-build2/receipt.json`: 468 Fresh, one rebuilt artifact; original red regression.
- `/tmp/miopunch137-corrected-tests-build/receipt.json`: 466 Fresh, four rebuilt artifacts; final affected tests.
- `/tmp/miopunch137-probe-build/receipt.json`: 462 Fresh, five rebuilt artifacts; observation probe.
- `/tmp/miopunch137-selector-compat-build/receipt.json`: 466 Fresh, three rebuilt artifacts; restored the existing formal-entry test name so the management-recovery and admission-rotation runners continue selecting it. The unchanged assertions pass in `selector-compat-qualified.log` (8.57 seconds).

All final checks reuse those binaries. The complete affected `legacy_nat_runtime` target was also run once: 31 passed in the isolated namespace, one existing CPU supplement remained ignored by its explicit fixture requirement, and the ordinary periodic net-report refresh case needed normal host routing. That same case then passed on the original host with the unchanged binary and assertions (`ordinary-refresh-qualified.log`, 21.90 seconds). Thus all 32 enabled cases passed under their applicable prerequisites; the initial isolated run and its environment failure remain visible in `legacy_nat_runtime-full.log`.

Logs are under `/tmp/miopunch137-isolated/`:

| Check | Result / log |
| --- | --- |
| Full `udp_observation` target | 9 passed, `udp_observation-qualified.log` |
| First QAD observer unavailable, subsequent samples consumed by enhanced UDP, original connection IDs and bidirectional business bytes | Passed, `legacy_nat_runtime-qualified.log` (`formal_entry_observes_selects_executes_and_confirms_original_business`) |
| Both permitted QAD observers fail, both get an opportunity, STUN fallback, original business queues/bytes, round exhaustion and recovery | Passed, `native_upgrade_reserves_qad_and_both_stun_observers_on_each_eligible_round.log` |
| Public upgrade absolute caller deadline | Passed, `public_udp_upgrade_deadline_bounds_stalled_hello_and_result.log` |
| Cancellation/revocation does not spend peer budget or update learning | Passed, `cancelled_or_revoked_round_has_no_peer_budget_or_learning_feedback.log` |
| Existing two-QAD classification and original business baseline | Passed, `two_real_qad_observers_public_preference_original_business_and_learning.log` |

The endpoint suite uses the normal host routing setup. The SDK checks use the existing `miopunch-nat61-lab:bookworm` image in independent network namespaces with loopback enabled and the repository mounted read-only; the exact same executable and core assertions are used. During this validation the host had 89 interfaces / 83 addresses, exceeding the existing snapshot wire limit of 64 local addresses. The no-blackhole baseline reproduced the same snapshot rejection there. This work does not introduce a new product address-truncation policy. Conversely, the ordinary periodic net-report tests need normal host routing, so a loopback-only namespace is not their valid prerequisite.

## Fixed-network public observation

An independent probe was run in the existing r_proxy container without replacing or restarting the running demo:

```text
docker exec miopunch-network-demo timeout 40s /demo/m137-qad-20261118/udp_observation_probe sequence
SEQUENCE_BOUND [0.0.0.0:47888]
aps1-1: skipped, TimedOut, 3002 ms
euc1-1: 91.99.237.97:7842, succeeded, 337 ms
use1-1: 87.99.151.110:7842, succeeded, 484 ms
usw1-1: 5.78.69.43:7842, succeeded, 694 ms
SEQUENCE_RESULT samples=3 current=true elapsedMs=4519
```

All three samples reflected `111.197.236.230:10007`. The independent probe used the pinned tool at `.artifacts/runs/miopunch137/udp_observation_probe`. Its public-endpoint code matches the tested endpoint fix; the later SDK plan-wait correction is separately qualified by the final SDK tests above. The single-point timeout still reproduces on aps1; the fix lets the remaining permitted sources run. The independent remote deployment copy was removed after the probe exited; the local pin is retained for the integration consumer.

A Windows probe from the same source was built in MIOPUNCH-138's standard Windows container batch (520 Fresh, eight rebuilt artifacts; `/tmp/mio138-windows-build/build.json`, pin `.artifacts/runs/mio138-windows/udp_observation_probe.exe`). Under the user's explicit deployment authorization, it ran as an independent endpoint on u22sz: aps1/euc1/use1/usw1 each timed out (3008/3011/3017/3011 ms), yielding no QAD samples after 12048 ms. This verifies continued attempts at all sources, and records current public QAD unavailability on that host; it is not a successful Windows QAD classification. Raw output is `/tmp/miopunch137-u22sz-qad.log`. The independent Windows deployment copy was removed after exit. The permitted STUN fallback is qualified at the real SDK seam by the controlled tests above; MIOPUNCH-140 must separately report that fallback and business behavior in its final two-host build.

This validates actual QAD observation, not public UDP punching or office/hotspot recovery. MIOPUNCH-139's first-packet comparison intentionally holds the old QAD-to-STUN source behavior constant; MIOPUNCH-140 must report that experiment separately from the new observation source. The user's single manual network-switch run remains MIOPUNCH-141.

## Review correction

The Standards review identified the initial two direct `eprintln!` QAD diagnostics as violating `vendor/iroh/DEVELOPMENT.md`. They were replaced with message-free structured `tracing::event!` calls at DEBUG level under `iroh::_events::qad::ordered_observation`, retaining observer, outcome, elapsed time, and destination/error-kind fields. This changes logging only. The historical probe outputs above were captured before that correction; the control flow and measured observation behavior are unchanged.

The corrected logging macros are covered by the successful final-source `network_maintenance --features nat-lab-test-hooks` compile in `/tmp/miopunch139/active-public-build/builds/001/receipt.json` (exit 0, 466 Fresh / three rebuilt artifacts). The current `vendor/iroh/src/net_report.rs` was compared with that build's existing source input and matched. No behavior matrix was repeated for the logging-only correction. The main review controller confirmed the targeted Standards finding closed and no remaining Spec finding for MIOPUNCH-137.
