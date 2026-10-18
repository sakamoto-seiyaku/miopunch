# MIOPUNCH-98: preserve ordinary portmapper defaults

Implementation: `49213d3ae50fa493422fdfdfa9afc062a227212a`.
Baseline: `1ddafa9da91beeefa981771bf2cc53f1239415c4`.

`PortmapperConfig::default()` restores upstream `Enabled {}`. Explicit product
consent, probe/fixture Disabled choices, and feature boundaries remain separate.
Ordinary default and feature-off consumers compile without unstable transports;
the restricted delegate now matches the ordinary method feature condition.

[Validation](validation.json) records the actual unchanged pre-edit consumer,
original mapping and PING/PONG, genuine fork compile failure, marked runtime
diagnostic, restored ordinary mapping/business, and feature-off packet silence.
It also contains 12 unique product gateway cases, 2 saved-consent units, 3 real
CLI PING/PONG cases without gateway traffic, and the current Native/Browser SDK
admission, durable materials, identity reopen, member business and natural cleanup.

Close returning is reported separately from gateway deletion. The original
baseline observed no delete; the restored controlled gateway observed its own
rule deleted while a foreign rule remained. TCP close isolation retains an
independent UDP mapping on the same port. No arbitrary-router cleanup guarantee
or wider public/physical matrix completion is claimed.

The SDKs compiled before four later test-only fixture changes. Every actual
selected source, lock, tool and immutable artifact remains identical and was
revalidated against the final source. Original compile/runtime receipts retain
their actual global source digests; the old runtime guard bytes are preserved.
The corrected UDP unit fixture uses current revision + 1; all four UDP gateway
cases were rerun, while eight TCP cases use byte-identical final binary bytes.

[Independent review](review.md): Standards 0 findings; Spec 0 findings. Reviewers
inspected source/controllers/receipts without rerunning builds or runtime checks.

The raw archive contains an explicit allowlist of controllers, manifests, build
logs, receipts, gateway wire captures, source snapshots and cleanup evidence.
Every [archive member](archive-manifest.json) was read back and hash verified.
Private SDK receipts, synthetic identities/invitations, actor logs, keys, stores
and NSS databases are excluded; public SDK results use scalar/hash fields.
Local immutable executables, WASM and selected source copies remain at receipt
paths. Historical intermediate test compression is lossless and verified.

The 249 pre-existing modified/untracked files retain their original hashes and
status. No push. M91 convergence and its remaining children continue separately.
