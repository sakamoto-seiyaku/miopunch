# MIOPUNCH-38 resource validation

The current source generation is **final3**: the actual wrapper and all nine
commands exited 0; **29 tests passed, zero failed and zero were ignored**.
All 303 prestart source/harness inputs remained unchanged. These are all 26
public resource cases plus the original NativeNetwork MQTT admission,
MemoryLookup resolution and gated p2panda protocol actor cases. WASM/default/
fault typechecks and workspace/modified-iroh formatting passed. The owned MQTT
broker and both anonymous volumes were removed and inspected absent.

Reproduce the exact current affected selection in a new directory:

```
rtk proxy python -B scripts/check_resources.py --lookup-fixes --output /tmp/mio38-new-verification
```

The broader `--suite` selection also covers native/gate/direct/signaling/
observation and two permission-NAT cases. Its historical **final1** wrapper and
all eleven commands exited 0 with **67 passed / 0 failed / 0 ignored** and unchanged 303
inputs; owned broker/two-volume cleanup and absence passed. Those checks
preceded the independent review which found actual lookup access gaps.
Final1 retains the original runtime/harness bytes and does not claim current
source reran 67 tests. **Final2-driver** changed only one Python abort-return
line and passed its actual workspace test typecheck; its precise earlier reuse
is recorded in [reuse-basis.json](reuse-basis.json).

The final lookup fix changes seven of 303 inputs: four iroh runtime files,
two test files and the runner. Its shared socket delta only installs the
policy on the address-lookup collection; observation, relay actor, fallback,
custom-carrier sends and NAT engine code remain identical to final1. Current
all 26 resources recheck the prior 21 behaviors plus lookup facilities,
publication, dynamic additions/wrappers, allowed signed lookup and HTTP
redirect denial. Current real NativeMQTT, memory and gated lookup cases cover
the changed collection integration. [Final3 reuse basis](final3-reuse-basis.json)
retains unchanged prior checks precisely, without a new NAT trial or sample
denominator.

Read [generation summaries](generation-summary.json), the full
[final1 receipt](final1/receipt.json), [actual wrapper exit](final1/wrapper-exit.json),
[prestart hashes](final1/prestart-source-sha256.json), each named command exit,
[source bytes](final1/source-snapshot.tar.gz), current
[final3 receipt](final3/receipt.json), [final3 wrapper exit](final3/wrapper-exit.json),
[final3 prestart hashes](final3/prestart-source-sha256.json) and
[lossless log manifest](lossless-manifest.json). `.log.gz` files preserve all
raw bytes; original/compressed byte counts/SHA and roundtripSHA are checked.
Frozen executable bytes remain in `/tmp/mio38-frozen-executables`, outside
Git. To recover owned disk capacity, 39 earlier executables were losslessly
compressed after verifying original/archive/roundtrip SHA and byte counts.
[Archive mapping](frozen-executable-archives.json) retains original paths and
exact generation hashes; original uncompressed copies were removed only after
verified archival. Later executables remain uncompressed. Only newly created
resource-test duplicate build products were cleaned; [receipt](owned-build-cleanup.json)
proves that test namespace did not exist at the fixed baseline. Prior-ticket
executables and all protected user paths remain untouched.

The 53 incremental generations retain real compile/fixture/environment
failures, public REDs and GREENs separately. A directory name alone does not
define its result: the first `priority-addressbook-red` run matched zero tests
because `--exact priority_disabled` was an unmatched selector; the following runtime RED exposed the real
forbidden access, and the final actual-server metrics prove zero forbidden
connections/bytes. Early first-real/default-table/priority runs have source-byte
archives matching their original prestart hashes, as well as raw logs. They
predate the runner's executable-freezing addition; those early executable
references are unavailable. No historical success is relabeled as current-source evidence.

Actual observations cover relay-only admission plus durable reopen/business,
default/self-hosted configuration, invitation/address-book/lookup/cache inputs,
known-actor route substitution, independent data/observation permissions,
external observation maps, trusted signed updates and restart, URL spelling,
untrusted/tampered/rolled-back updates, bounded temporary unreachability and
import recovery, allowed fallback, backup/restore, forbidden signaling broker
access, mDNS wire publication/receive-only discovery and CLI process lifecycle.
The high-level p2panda Passive early return is retained as an explicit observed
limitation, separate from native low-level Passive discovery.

[NAT selection](nat-selection.json) records eight affected cases run and 37
unchanged cases excluded. Twelve NAT source/test inputs match baseline
`8a93e340c4e01885bd9ed1e3da1811220dbf683b`. Shared relay/observation/custom
resource paths changed, so their actual resource checks run; unchanged physical
socket algorithms, selector, scheduler, ranges, probability and profiling use
the frozen MIOPUNCH-60–63 evidence. No probability trial or sample denominator
was added. This follows the user's explicit unchanged-NAT test policy.

Runtime scope is Linux with controlled local facilities, real signatures,
SQLite/private file persistence, TLS/QUIC and actual access/process records.
WASM is compilation only. Read [the validation note](../../notes/2026-10-01-resource-policy-validation.md)
for resource capabilities, trusted update semantics, startup boundaries and
limits. The [initial Standards](initial-standards-review.md) and [initial Spec](initial-spec-review.md)
reports preserve their findings. The documentation corrections, actual
lookup access RED/GREEN, native broker eligibility RED/GREEN and forbidden
HTTP redirect RED/GREEN remain separate generations. Native Pkarr clients
reject redirects; constrained external lookup fails closed on WASM because
its reqwest Fetch backend cannot control redirects. Current BrowserNode
installs no external lookup service; browser lookup runtime capability is
not claimed. Final independent [Standards](standards-review.md) and [Spec](spec-review.md)
rereviews each report zero required findings. Standards retains one optional
fixture-duplication suggestion.
