# MIOPUNCH-22: connection-bound admission proof qualification

2026-09-29. This records a technical profile and reproducible evidence for the
[MIOPUNCH-22 validation ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/90fcb8da-5b37-4f6b-8695-62995b34e4d0).
It qualifies a proof mechanism for later integration. It does not grant membership,
enable ordinary application protocols, or claim a complete cryptographic audit.

## Fixed qualification profile

- Rust 1.98.1 was used; `Cargo.toml` permits 1.96. The locked mechanism is
  `rsasl = 2.3.1` with only `config_builder`, `provider`, and `scram-sha-2`;
  `iroh = 1.2.0`. The application registers only `SCRAM-SHA-256-PLUS`.
- The initiating iroh endpoint is the SCRAM client; the accepting endpoint is
  the SCRAM server. They exchange one framed proof on a bidirectional stream of
  an already completed, mutually identified iroh TLS connection. A production
  handler must not run this through 0-RTT or hand the connection to normal
  protocols before the result is checked.
- Channel binding is RFC 9266 `tls-exporter`: exactly 32 bytes from the
  connection's ordinary TLS exporter, label `EXPORTER-Channel-Binding`, empty
  context. The client explicitly requests `tls-exporter`; the server requires
  the exact `p=tls-exporter,,` GS2 prefix and supplies only that binding type.
  `n`, `y`, `tls-unique`, authorization identities, and mechanism fallback are
  rejected. This strict application check closes rsasl 2.3.1's `n,,` acceptance
  path under a `-PLUS` mechanism.
- The SCRAM authentication identity is
  `miopunch:<32-byte-network-id-in-hex>:<secret-generation-u64>` and is checked
  against the locally expected scope. It enters SCRAM's authenticated
  transcript. It is a credential selector, never the peer identity: the latter
  comes from iroh TLS. A secret is 32 system-generated random bytes, encoded as
  64 ASCII hex bytes only for SCRAM's password input. Restored bytes must have
  originated from that generator; length alone cannot prove entropy. SCRAM is
  not a PAKE and a human password would permit offline guessing.
- The server uses a fresh 32-byte salt and 16,384 PBKDF2 iterations. The client
  accepts only the exact iteration count and a bounded salt field before rsasl
  starts PBKDF2. Each frame is at most 512 bytes; each proof has a five-second
  deadline, and the module admits at most 32 concurrent client/server proof
  calls and 4,096 live proof sessions. The connection-attempt registry holds
  only weak tokens, not connection references; failed or cancelled attempts
  close their connections. The current API derives server verification keys per attempt and has
  no cross-session credential cache. Later continuous service should prepare
  credentials once per network and generation and bound the upstream connection
  queue as well.
- The server returns a `BootstrapProof` only if rsasl's validation callback
  returned `Some(true)` after checking the client proof, and then the client
  acknowledged verification of the server final. The client returns only after
  rsasl verified that final and the server acknowledged completion. A rsasl
  `Finished` state alone is insufficient. The fact records authenticated peer
  id, connection stable id, network and generation; it grants only restricted
  bootstrap eligibility, subject to separate membership authorization. Its
  fields are private and it holds the original connection; `applies_to` checks
  the live connection and currently expected generation before reuse.
- Treat one proof as one sustained protocol session on one TLS connection.
  Path changes within that connection retain its fact. Closing that protocol,
  rotating the secret, moving to another network, or reconnecting requires a
  fresh connection and proof; never copy the fact to a new connection. The
  module rejects another proof attempt on the same connection while its proof
  is live. Failed and cancelled attempts close their connection automatically.
  `BootstrapProof::invalidate` closes a successful session; dropping that fact
  also closes it. Callers must retain the fact while the protocol is active and
  invalidate it on rotation, revocation, or protocol end. The proof API is a
  qualification seam, not yet the full connection stage gate. The later gate
  must own each connection, compare against the current generation, and check
  authorization before wider synchronization or business traffic.

## Reproduction and observed result

Run `rtk cargo test --locked --test admission_proof -- --test-threads=1` on a host
that permits loopback UDP sockets. Tests use separate real iroh endpoints and
TLS connections. They cover both final successes, wrong secret/network/generation,
transparent forwarding to another TLS connection, a deliberately wrong exporter
on one actual TLS connection, invalid server final, old
valid client transcript replay on a fresh connection, disallowed GS2 binding
forms, excessive iteration count, cancelled-session cleanup, and exporter equality at both ends versus
inequality after reconnect. The captured SCRAM client messages were checked
for absence of both raw and hex-encoded admission secret. An independent
[RFC 7677 §3 example](https://www.rfc-editor.org/rfc/rfc7677.html#section-3)
checks the rsasl SHA-256 server verifier against published bytes via
`rtk cargo test --locked --lib scram_sha256_matches_rfc_7677_server_verifier`.

The validation does not measure brute-force resistance, operational rotation,
all ALPN gate coverage, or network-wide authorization. Those belong to the
dependent integration work, with the accepted boundaries in
[`CONTEXT.md`](../../CONTEXT.md) and
[ADR 0004](../adr/0004-network-admission-secret.md).
