# MIOPUNCH-40 TCP direct evidence

Implementation and recorded choices:
[native TCP note](../../notes/2026-10-01-tcp-direct-validation.md).
Ticket40's approved seams are the formal public node and normal iroh
connections, with actual sockets, TLS, signatures, admission proofs and stores.
No simulated TLS/QUIC, storage success or network reachability is used.

## Current acceptance and affected checks

[`36-final-suite-new-record/receipt.json`](36-final-suite-new-record/receipt.json)
records build0 and test0. Its [test log](36-final-suite-new-record/test.log)
contains **11 passed /0 failed /0 ignored** in35.23s. Both IPv4 and IPv6 have
zero UDP sockets, complete manual admission, durable reopen, and real LogSync
plus controlled bidirectional business streams. After admission, the source
publishes a new signed AddMember record; the receiver does not have it before
TCP sync, then saves it and applies it to its public membership view.

| Check | Actual result |
| --- | --- |
| Wrong expected node identity at a reachable TCP port | Rejected; no manual request or completed admission |
| Real but unapproved node identity | Cannot obtain member history |
| Verified member removal | Live stream ends and stale member's new connection is rejected |
| Disabled exact TCP candidate, raw Endpoint bypass | Actual TCP accepts0 |
| Disabled lookup-supplied candidate | Actual TCP accepts0 |
| Global TCP disable, advanced builder adds a carrier | No advertised normal TCP address; actual target TCP accepts0 |
| Cancel during TCP preface | Pending connect fails and actual socket reaches EOF within bounds |
| Physical loss of both directions of the only TCP path | Old connection/stream fail; later legitimate contact has a new stable ID |
| Normal TCP-only CLI startup and SIGINT | Actual TCP READY address and exit0 |

The final physical-loss check observes closure in10498ms; this is one controlled
measurement, not a universal timeout/SLA. The actual selected data path is
Custom, carrying inner QUIC through TCP. Loopback IPv4 and IPv6 are separate
real Linux sockets, not cross-operator/platform reachability evidence.

Affected short regressions add40 passed cases: carrier2, native probe11,
resource policy26, and the admitted-TCP connection after an unavailable UDP
strategy1. See [initial affected receipt](29-affected/receipt.json) and
[resumed receipt](30-affected-resume/receipt.json). Successful checks were not
repeated when resource compilation exhausted disk. Both admission and business
fixtures use real identities, signatures and private temporary storage.
**Total meaningful ordinary checks:51 passed cases.** This is not a full
workspace runtime-suite claim. The final TCP suite strengthens only the two
sync checkers; runtime source is unchanged from the affected regressions.

Workspace/all-targets typechecking, browser WASM compilation and workspace
formatting pass in30's logs. The first recursive fork formatter inspected an
unchanged child module's preexisting style and returned1; that unrelated
formatting was restored. The narrowed modified-file check with
`--config skip_children=true` returns0 in [32](32-fork-format.log).
WASM evidence is compilation only. Unchanged slow NAT matrices and probability
denominators were excluded under the user's standing policy.

## First real check and immediate failure

The original production startup fails in [03](03-first-real-red.log): no TCP
candidate is advertised. The first real GREEN is [08](08-first-green.log), then
the required immediate wrong-identity check passes in
[09](09-priority-wrong-identity.log). Compiler/sandbox failures01/02, the custom
candidate API/fixture corrections04–07 and later environment failures are kept
distinct from these real public RED/GREENs.

The forbidden candidate is actually dialed in [10](10-disabled-red.log).
After qualification at the fork's actual send boundary, [12](12-disabled-green.log)
shows TCP accepts0.11 failed compilation due to disk. The actual IPv6 listener
exists in [13](13-ipv6-red/test.log), but IPv4-only configuration rejects it;
[14](14-ipv6-green/test.log) proves the independent IPv6 GREEN.15–19 retain
individual revocation, cancellation, unapproved identity, lookup and global
disable checks.26/27 retain failed compilation versus real CLI success.

## Physical-cut diagnosis and disk failures

20's cut fixture can be bypassed by the prior admission circuit.21 uses fresh
endpoints but still leaks a reverse return route.24's public QUIC statistics
prove valid packets continued arriving after that cut; this is not evidence of
an immortal disconnected connection. The corrected proxy confines both
untrusted prefaces to its own route and drops both sockets and the listener.
[25](25-physical-loss-green/test.log),28 and36 exercise the actual complete cut.
Debug instrumentation was removed from source before the final checks.

23,26,29's resource compile,31 and33–35 contain real no-space build failures,
not runtime failures or unexecuted GREENs.34 additionally exhausted space
while writing its receipt; its build log and nonzero wrapper result are retained,
and its incomplete receipt is not counted.32 is only the narrowed format check.
The final36 build and runtime exit0 are recorded normally. `--strip-debug`
affects only the final test link. Necessary cleanup removed only this ticket's
generated incremental sessions/failed outputs and completed/rebuilt binaries,
or stripped their debug sections in place. Prior-ticket frozen evidence and
original user work were preserved. No source archive or executable freezing
gate was introduced. Raw tool output retains its original whitespace; source/doc
whitespace checks exclude these machine-generated logs.

## Reproduction

```sh
rtk proxy python3 -B scripts/check_tcp_direct.py --output NEW_DIRECTORY
rtk proxy cargo test --locked --lib tcp_transport::tests -- --nocapture --test-threads=1
rtk proxy cargo test --locked --test native_probe -- --nocapture --test-threads=1
rtk proxy cargo test --locked --test resources -- --nocapture --test-threads=1
rtk proxy cargo test --locked --test admission_gate an_admitted_tcp_data_connection_survives_an_unavailable_nat4_strategy -- --nocapture --test-threads=1
rtk proxy cargo check --locked --workspace --all-targets
rtk proxy cargo check --locked -p miopunch-browser-probe --target wasm32-unknown-unknown
```

Use `--strip-debug` for a constrained Linux test link and `--case NAME` for one
formal behavior. Test fixtures use only their own sockets, child process and
temporary directories. No long-lived mapping, broker, container, namespace or
private credential artifact is delivered. Independent [Standards](standards-review.md) and [Spec](spec-review.md) reviews
both report0 findings; [aggregation](code-review.md) preserves both axes. Runtime
source was unchanged during review. The implementation commit and Plane completion
are reported by the OS-temp handoff.
