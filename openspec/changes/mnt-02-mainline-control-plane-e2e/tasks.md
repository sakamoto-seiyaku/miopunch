## 1. Seed & Topology

- [x] 1.1 Add `miopunch-lab mnt02-seed` to generate N peer identities and minimal `state.json` (self-hosted broker, stable ports; no membership/decl/peers injection)
- [x] 1.2 Add unit tests for `mnt02-seed` (discloses injected fields; rejects public-broker defaults)
- [x] 1.3 Add a guest topology helper for N peers (single NAT + multi-peer LAN) suitable for scenario-2 control-plane e2e
- [x] 1.4 Wire NAT profile application (default NAT1) and optional STUN endpoint into the MNT-02 topology

## 2. Runner & Gates

- [x] 2.1 Implement `lab/guest/bin/mlab-mnt02-run` (start broker/STUN, start N daemons, drive CLI tasks via LocalAPI, collect artifacts, write per-case summary)
- [x] 2.2 Add `lab/guest/lib/mnt02_aggregate.sh` to aggregate case summaries into gate-level `summary.json`
- [x] 2.3 Add `lab/guest/bin/mlab-mnt02-smoke` (minimal `up -> invite/approve/join -> ping` + `sh` smoke)
- [x] 2.4 Add `lab/guest/bin/mlab-mnt02-selftest` (multi-member consistency, idempotency, restart, broker outage/recovery, revoke boundary, bounded concurrency)

## 3. Evidence & Redaction

- [x] 3.1 Add report evidence checks in runner (stage/reason_code presence; `ping=ok` for required cases; artifacts pointers on failure)
- [x] 3.2 Add report redaction checks (no `secret_key`, `net_secret_b64`, `invite_secret_b64`, or unredacted invite codes in exported reports)
- [x] 3.3 Add broker artifacts collection (broker logs + mqtt pcap) for required cases

## 4. Host Integration

- [x] 4.1 Extend `lab/host/labctl` with `mnt02-smoke` and `mnt02-selftest` commands (run guest gate + pull artifacts)

## 5. Verification

- [x] 5.1 Run `go test ./...` and `go vet ./...` (host)
- [x] 5.2 Run `./lab/host/labctl mnt02-smoke` and ensure gate passes with artifacts summary
- [x] 5.3 Run `./lab/host/labctl mnt02-selftest` and ensure gate passes with artifacts summary
