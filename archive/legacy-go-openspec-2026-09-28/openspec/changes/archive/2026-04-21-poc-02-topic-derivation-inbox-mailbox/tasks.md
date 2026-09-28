## 1. Topic Derivation (inbox/mailbox)

- [x] 1.1 Add a shared helper to derive `inbox_topic` from `net_secret + peer_id` (HKDF + base32 raw no-pad + lower-case)
- [x] 1.2 Ensure `peer_id` is canonicalized before using it in HKDF `info`
- [x] 1.3 Add unit tests: deterministic derivation for the same inputs
- [x] 1.4 Add unit tests: different `peer_id` values produce different inbox topics
- [x] 1.5 Add unit tests: derived topic is lower-case base32(raw,no-pad) and fixed length (`26`)
- [x] 1.6 Add unit tests: derived topic does not contain `peer_id` as a substring

## 2. Join Code broker pinning (`invite_brokers`)

- [x] 2.1 Define join code schema/struct to carry `invite_brokers` (`1..2`, `host:port`, no credentials)
- [x] 2.2 Implement broker endpoint canonicalization for join code output (prefer `ip:port`; hostname → resolve first A record; unresolved hostname → keep + strong warning)
- [x] 2.3 Implement invite-side selection of `invite_brokers` (prefer active brokers from `up`; else fall back to brokers_effective)
- [x] 2.4 Implement approve/join-side usage: use only `invite_brokers` from code for invite/join MQTT subscribe/publish
- [x] 2.5 Add unit tests for canonicalization behavior and warning conditions
- [x] 2.6 Add helper + tests: derive MQTT broker URLs exclusively from join code `invite_brokers`

## 3. Control-plane Mailbox Smoke (local/CI reproducible)

- [x] 3.1 Add a local broker-based smoke test harness (reuse `miopunch-lab mqtt-broker` or an embedded broker) for subscribe/publish on derived inbox topics
- [x] 3.2 Ensure the smoke test validates “deliverability” without leaking readable plaintext in MQTT payloads
- [x] 3.3 Make the smoke runnable in CI/local without external network dependencies

## 4. Validation

- [x] 4.1 Run `go test ./...`
- [x] 4.2 Run `go vet ./...`
