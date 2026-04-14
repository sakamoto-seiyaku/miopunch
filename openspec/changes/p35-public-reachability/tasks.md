## 1. Pre-flight Validation

- [x] 1.1 Run baseline `go test ./...` (clean tree)
- [x] 1.2 Run baseline `go vet ./...`

## 2. CLI & YAML Surface

- [x] 2.1 Add peer short flags `-4/-6` and plumb into runtime config (`P2P/打洞` only)
- [x] 2.2 Add YAML config fields for P2P IP family preference (keep `snake_case`)
- [x] 2.3 Add YAML + CLI knobs for built-in DNS mode and resolver list (scope-limited to STUN/MQTT)
- [ ] 2.4 Ensure “explicit `--stun`/`stun:` disables internal STUN + cn/global arbitration” is enforced

## 3. Built-in DNS (TCP/53)

- [x] 3.1 Add unit tests for resolver mode behavior and TCP/53 query path (use a local test DNS server)
- [x] 3.2 Implement built-in DNS resolver over `TCP/53` with modes `auto|on|off`
- [x] 3.3 Default built-in resolver list to `1.1.1.1, 8.8.8.8, 223.5.5.5, 119.29.29.29` and allow YAML override
- [x] 3.4 Integrate resolver into STUN endpoint hostname resolution only
- [x] 3.5 Integrate resolver into MQTT broker hostname resolution only

## 4. STUN cn/global Sampling & Single-View Arbitration

- [ ] 4.1 Add unit tests for arbitration determinism and tie-breaking (incl. “hard tie defaults to global”)
- [ ] 4.2 Add internal STUN defaults split into `cn` and `global(!cn)` buckets (source: gonc baseline)
- [ ] 4.3 Gather per-bucket observation summaries (availability, ok_count, RTT, NAT difficulty)
- [ ] 4.4 Extend exchange payload to carry enough observation summary for deterministic selection
- [ ] 4.5 Implement deterministic arbitration: availability → NAT difficulty → STUN RTT (30ms tie) → ok_count → default global
- [ ] 4.6 Ensure `exchange` produces exactly one final `selected_view`, and attempt/punching uses only that view’s candidates

## 5. Observability & Experiment Runbook

- [ ] 5.1 Add `debug` evidence chain logs for cn/global observations + step-by-step arbitration
- [ ] 5.2 Add non-debug summary logs for `selected_view` + key reason
- [ ] 5.3 Add a minimal public-network runbook doc (case0..caseN) that records exact commands and required evidence to accept a run (incl. `--log-level debug`)

## 6. Post-change Validation

- [ ] 6.1 Run `go test ./...` and `go vet ./...`
- [ ] 6.2 If touching lab/runtime behavior, run the full lab gate set from `.codex/skills/dev/SKILL.md`
- [ ] 6.3 Manually verify at least one real-network run where system DNS fails but built-in DNS resolves STUN/MQTT
- [ ] 6.4 Manually verify at least one real-network run that exercises cn/global view selection with recorded evidence
