## 1. Runbook & Templates

- [ ] 1.1 Add a temporary runbook doc at `docs/reports/2026-04-13-public-verification-cases.md` for case0-4 (topology, roles, acceptance, troubleshooting)
- [ ] 1.2 Provide YAML config templates (client/visitor) with placeholders (MQTT/STUN/proxy/secret/timeouts)
- [ ] 1.3 Document Android arm64 build + ADB deploy/exec (no Go required on device)
- [ ] 1.4 Define a minimal log-evidence checklist (MUST include `transport.payload_exchanged` on both sides)

## 2. Case0 (LAN smoke): Host ↔ Pixel6a

- [ ] 2.1 Cross-compile `miopunch` for Android arm64 (Docker or host Go) and push to Pixel6a
- [ ] 2.2 Run case0 (recommended: `signaling=coord`, optional: `signaling=mqtt`) and capture both-side logs
- [ ] 2.3 Record results (commands, timestamps, paths, key events) in `docs/reports/2026-04-13-public-verification-cases.md`

## 3. Case1 (Mobile ↔ H0): Android data network ↔ home hub subnet

- [ ] 3.1 Run case1 with `signaling=mqtt` and capture both-side logs
- [ ] 3.2 Record results in `docs/reports/2026-04-13-public-verification-cases.md`

## 4. Case2 (Mobile ↔ H1): Android data network ↔ home isolated subnet

- [ ] 4.1 Run case2 with `signaling=mqtt` and capture both-side logs
- [ ] 4.2 Record results in `docs/reports/2026-04-13-public-verification-cases.md`

## 5. Case3 (H0 ↔ H1): hub subnet ↔ isolated subnet (same home broadband)

- [ ] 5.1 Run case3 and capture both-side logs
- [ ] 5.2 Record results in `docs/reports/2026-04-13-public-verification-cases.md`

## 6. Case4 (Windows coverage)

- [ ] 6.1 Pick a Windows endpoint (H0 or H1) and run case4 with `signaling=mqtt`
- [ ] 6.2 Record results in `docs/reports/2026-04-13-public-verification-cases.md`

## 7. Follow-ups

- [ ] 7.1 Summarize findings and open issues (timeouts, NAT behaviors, MQTT constraints)
- [ ] 7.2 If needed, propose the next change(s) based on findings
