## Standards

Reviewed only the four frozen paths against fixed point `56140da9887cfecc24081015eeb49518b1ce3279`. Candidate SHA-256 `849da5345323a86ae388d588a2d910115d9db5c79cac3ae9bdf17bc2cc9cec07` and all four frozen source hashes match.

**Required documented violations: none found.** Standards sources: root `AGENTS.md`, `/home/js/.codex/RTK.md`, `docs/agents/build-cache.md`, `docs/agents/domain.md`, `CONTEXT.md`, current design entry points, and ADR 0009. No additional active coding-standards file was found. Archived Go/OpenSpec rules are excluded; repository rules override Fowler heuristics; tooling-enforced formatting/lint items are skipped.

The candidate preserves compiler-input/artifact preconditions, shared ownership helpers, private first-return evidence, and independently checked cleanup. Relay CLOSING callbacks are narrowly distinguished from MQTT retirement, and pending Relay close/tasks/receive-queue uncertainty remains explicit. The ordinary member waiter retains the connection until consumer or endpoint closure. No documented architecture change is implied.

**Optional Fowler judgements, not violations:**

1. **Possible Duplicated Code** — `browser-probe/src/mqtt.rs:410`: the new hunk repeats `self.socket.set_onopen(None); self.socket.set_onmessage(None); self.socket.set_onerror(None); self.socket.set_onclose(None);`, already present in `Drop` at lines 421–424 after `self.close()`. One callback-detachment helper, or using the now-centralized close path, could prevent future cleanup drift. The present duplication is harmless and may be deliberate defensive cleanup.
2. **Possible Data Clumps** — `scripts/check_current_browser_relay.py:82`: `def guard_build(receipt, pin, *, binary=None, binary_pin=None, package=None, package_pin=None):`. Receipt/artifact paths and their pins travel together at startup and final guards (lines 474–478 and 691–694). Small native/browser artifact-binding records could keep corresponding paths and hashes together and reduce argument mismatch risk. Current explicit calls remain reviewable; this is optional.

Also assessed Mysterious Name, Feature Envy, Primitive Obsession, Repeated Switches, Shotgun Surgery, Divergent Change, Speculative Generality, Message Chains, Middle Man, and Refused Bequest; no additional actionable smell identified.

Read-only review; no tests, builds, SDK/device execution, Git/Plane operations, or source changes. Owned jobs: 0. This standards result does not close M76’s known unobserved API-available RTC-network-failure → successful Relay category.

Counts: **0 required, 2 optional**; highest concern is low-severity duplicated callback cleanup.
