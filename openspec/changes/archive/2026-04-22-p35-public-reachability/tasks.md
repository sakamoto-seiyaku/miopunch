## 1. Pre-flight Validation

- [x] 1.1 Run baseline `go test ./...` (clean tree)
- [x] 1.2 Run baseline `go vet ./...`

## 2. CLI & YAML Surface

- [x] 2.1 Add peer short flags `-4/-6` and plumb into runtime config (`P2P/打洞` only)
- [x] 2.2 Add YAML config fields for P2P IP family preference (keep `snake_case`)
- [x] 2.3 Add YAML + CLI knobs for built-in DNS mode and resolver list (scope-limited to STUN/MQTT)
- [x] 2.4 Ensure “explicit `--stun`/`stun:` disables internal STUN + cn/global arbitration” is enforced

## 3. Built-in DNS (TCP/53)

- [x] 3.1 Add unit tests for resolver mode behavior and TCP/53 query path (use a local test DNS server)
- [x] 3.2 Implement built-in DNS resolver over `TCP/53` with modes `auto|on|off`
- [x] 3.3 Default built-in resolver list to `1.1.1.1, 8.8.8.8, 223.5.5.5, 119.29.29.29` and allow YAML override
- [x] 3.4 Integrate resolver into STUN endpoint hostname resolution only
- [x] 3.5 Integrate resolver into MQTT broker hostname resolution only

## 4. STUN cn/global Sampling & Single-View Arbitration

- [x] 4.1 Add unit tests for arbitration determinism and tie-breaking (incl. “hard tie defaults to global”)
- [x] 4.2 Add internal STUN defaults split into `cn` and `global(!cn)` buckets (source: gonc baseline)
- [x] 4.3 Gather per-bucket observation summaries (availability, ok_count, RTT, NAT difficulty)
- [x] 4.4 Extend exchange payload to carry enough observation summary for deterministic selection
- [x] 4.5 Implement deterministic arbitration: availability → NAT difficulty → STUN RTT (30ms tie) → ok_count → default global
- [x] 4.6 Ensure `exchange` produces exactly one final `selected_view`, and attempt/punching uses only that view’s candidates
- [x] 4.7 Narrow `selected_view` semantics so it only applies to STUN-derived public candidates; direct/local/assisted/portmap exchange must remain unchanged
- [x] 4.8 Add focused tests/regression coverage for “internal STUN enabled but LAN/direct metadata still survives exchange unchanged”
- [x] 4.9 Re-run public case verification and confirm internal-STUN path does not break same-LAN/near-LAN case behavior purely by view selection

## 5. Observability & Experiment Runbook

- [x] 5.1 Add `debug` evidence chain logs for cn/global observations + step-by-step arbitration
- [x] 5.2 Add non-debug summary logs for `selected_view` + key reason
- [x] 5.3 Add a minimal public-network runbook doc (case0..caseN) that records exact commands and required evidence to accept a run (incl. `--log-level debug`)
- [x] 5.4 Clarify in runbook/reporting that `selected_view` is a STUN-public-only decision, not a direct/local candidate filter

## 6. Post-change Validation

- [x] 6.1 Run `go test ./...` and `go vet ./...`
- [x] 6.2 If touching lab/runtime behavior, run the full lab gate set from `.codex/skills/dev/SKILL.md`
- [x] 6.3 Manually verify at least one real-network run where system DNS fails but built-in DNS resolves STUN/MQTT
- [x] 6.4 Manually verify at least one real-network run that exercises cn/global view selection with recorded evidence
- [x] 6.5 Manually verify at least one same-LAN or near-LAN run where internal STUN is enabled and final connectivity still succeeds through preserved local/direct information

## 7. Code Review Fixups

- [x] 7.1 Redact MQTT broker credentials in logs/events (do not emit user/pass in `broker` fields)
- [x] 7.2 Run `gofmt` for touched files and ensure `gofmt -l .` is clean
- [x] 7.3 Add missing doc comments for exported DNS resolver APIs (`internal/netutil`)
- [x] 7.4 Add clarifying comments for `StunExplicit` behavior (avoid naked bool ambiguity)
- [x] 7.5 Reject unsupported STUN address formats (e.g. `tcp://...` or `?...`) instead of silently treating them as UDP
