# MIOPUNCH-65 public browser evidence

Reproduce with `rtk proxy python -B scripts/check_browser_public_relay.py --output /tmp/mio65-check --checks`.
Read the [qualification note](../../notes/2026-10-03-browser-official-public-relay.md)
for service provenance, exact scope and concurrency context.

## Actual public result

The public runs use Chrome for Testing 153.0.8010.12, normal TLS trust and
`https://aps1-1.relay.n0.iroh.link./`. Chromium records the real
`wss://aps1-1.relay.n0.iroh.link./relay` HTTP 101 response with
`iroh-relay-v2` and nonzero bidirectional traffic. Native iroh paths independently
show only that relay selected for admission, LogSync and member data. No
loopback relay, generated CA or certificate exception is involved in these runs.

- `development/first-red`: the original fixture completed actual public manual
  admission, both materials and a member echo, then the new check rejected its
  absent path provenance. This is an evidence-gap RED, not a public-service,
  TLS or admission failure. The native fixture exited 0.
- `development/first-green`: after adding read-only fixture path observations,
  the first complete public/LogSync/refresh check passed before the prioritized
  persisted-disable check. That failed startup in 445 ms, made zero external
  attempts and recovered the same member without another approval. Native exit 0.
- `final-suite/results.json`: the public replay, current native/WASM builds,
  workspace typecheck, three affected native suites and formatting exited 0.
  Its browser access was 2026-10-03 02:26:13–02:26:50 UTC at
  `http://127.0.0.1:43408`; the disabled start failed in 353 ms. Three WSS sessions
  have real HTTP 101/protocol/byte receipts in `public-browser.json`.
- `replica-regression`: the existing browser replica scenario passed after
  rebuilding both native browser fixtures. It checks original-author-offline
  transfer after browser reload and another browser's durable save. This is
  explicitly the prior self-hosted HTTPS/WSS regression with its generated CA
  and isolated certificate exception, not the public acceptance evidence.
- `current-entry`: the final runner entry is rerun after adding process-group
  timeout cleanup, driver-owned private state, final-only success reporting and
  the optional affected browser regression. It performs fresh builds and the
  same ordered public slice; it passed at 2026-10-03 02:28:47–02:29:24 UTC,
  origin `http://127.0.0.1:43784`, with a 334 ms disabled start and native exit 0.
  Exact timestamps and receipts are in its JSON.

## Assertions and interpretation

The complete public check requires one manual approval, saved authorization
and secret, a verified three-record native-source LogSync result, and the exact
original nonempty record body after refresh and reopen. The synthetic control
record is the fixture's existing signed Add example; it accounts for the third
record. After relay disable and explicit restore, the count remains three,
the member set and one admission request are unchanged and there is exactly
one approval event. No additional admission or precode redemption occurs.

The disable is checked both explicitly and from saved configuration. Failed
pages present a temporary-unreachable result and disabled business controls;
Chromium observes zero WSS attempts or external requests in that phase. The
restored page uses the same key and existing materials and completes another
real member exchange. This tests client disable policy, not a public-service
outage or uninterrupted survival of an existing application stream.

The affected native command reports 35 passing entries across `manual_admission`,
`offline_replica` and `admission_gate` without replica fault hooks. Two inert
manual worker entries and one no-broker MQTT early return are included in that
count; these are not new network acceptance. Feature-only replica crash/fault
cases and unchanged slow NAT matrices were not rerun. The public browser
LogSync and separate browser regression are real exchanges.

Logs retain original commands, exits and failures. Repository HTTPS header logs
normalize CRLF and terminal blank lines; original OS-temp outputs remain available. Frame payloads, native
stores, browser profiles, private keys and admission secrets are not retained.
The manual invitation contains only the public network anchor and contact.
All successful browser runs have actual exit-0 fixture cleanup; only this
ticket's private resources were removed. Concurrent MIOPUNCH-64 and unrelated
user working-tree changes remain outside this ticket's review and commit.

Independent [Standards](standards-review.md) and [Spec](spec-review.md) reports
and their [separate aggregation](code-review.md) are complete. Standards found
zero documented violations and one optional duplication heuristic; Spec found
zero findings. Both reviewers used GPT-6.1 Sol HIGH with actual runtime priority
readback. The optional suggestion remains parked under the user's completion
instruction; no required fix remains.
