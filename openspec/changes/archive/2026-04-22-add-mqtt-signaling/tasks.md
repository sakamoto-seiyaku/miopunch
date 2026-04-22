## 1. Dependencies & Scaffolding

- [x] 1.1 Add MQTT + YAML dependencies to `go.mod`
- [x] 1.2 Add `internal/signaling/mqtt` package skeleton

## 2. Local MQTT Broker (Lab)

- [x] 2.1 Add `miopunch mqtt-broker` command (minimal embedded broker)
- [x] 2.2 Emit a deterministic “broker ready” log line for lab waiting

## 3. MQTT Signaling Backend

- [x] 3.1 Derive `sid` from `proxy+secret` (scheme A)
- [x] 3.2 Implement MQTT hello presence + session barrier
- [x] 3.3 Implement exchange of `wire.NatHoleVisitor` / `wire.NatHoleClient`
- [x] 3.4 Implement `ready` + `start_at` barrier before `attempt`

## 4. Peer Integration (coord|mqtt)

- [x] 4.1 Extend peer configs with signaling/mqtt fields
- [x] 4.2 Add MQTT signaling path for `peer client`
- [x] 4.3 Add MQTT signaling path for `peer visitor` (visitor acts as leader)
- [x] 4.4 Expose coordinator analysis helper for MQTT leader to reuse

## 5. YAML Config (`--config`)

- [x] 5.1 Add `--config <yaml>` to peer commands and implement “CLI overrides YAML”
- [x] 5.2 Support mqtt-related fields in YAML (broker/topic/user/pass)

## 6. NAT Lab Regression

- [x] 6.1 Update `lab/guest/bin/mlab-xtcp-run` to support `--signaling mqtt`
- [x] 6.2 Add ordered event expectations for a mqtt signaling run (includes `transport.payload_exchanged`)
- [x] 6.3 Add one mqtt regression run to `lab/guest/bin/mlab-xtcp-connectivity-selftest`

## 7. Verification

- [x] 7.1 Run `/usr/local/go/bin/go test ./...`
- [x] 7.2 Run `/usr/local/go/bin/go vet ./...`
- [x] 7.3 Run `lab/guest/bin/mlab-xtcp-connectivity-selftest`
