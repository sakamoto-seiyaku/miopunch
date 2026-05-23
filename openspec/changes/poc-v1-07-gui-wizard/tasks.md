## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`

## 2. Stage model

- [ ] 2.1 Define the 6-stage wizard state machine (Network/Enroll/Discover/Punch/SecureSession/Shell)
- [ ] 2.2 Ensure each stage has: summary, reason_code, suggested next action

## 3. Output contract

- [ ] 3.1 Implement `UserSummary` renderer (<=3 lines per stage)
- [ ] 3.2 Implement `Evidence` panel (fold + export)
- [ ] 3.3 Freeze reason_code enum (<=12)

## 4. Post-change Validation

- [ ] 4.1 Re-run `go test ./...`
