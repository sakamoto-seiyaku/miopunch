# Fixed vendor distribution and artifact evidence

MIOPUNCH-105, source `865e3ac2` (main restoration `44608104`, distribution
boundaries `845a986a`, mandatory-input checks `865e3ac2`), fixed point `090bde3d`.

[Validation](validation.json), [independent reviews](review.md),
[archive manifest](archive-manifest.json), [public raw evidence](raw-verification.tar.gz)
and [private-material scan](private-material-checks.json) preserve the actual checks.
The archive includes original fixed-upstream sources, a new nine-package diff
inventory, source package member hashes, frozen inputs, depfiles, controllers,
failed attempts and qualified final receipts. Its inventory retains its actual
pre-commit working-tree header; historical audit statistics are unchanged.

- Eleven original files restore nine declared Noq/Proto targets. Ordinary Noq
  all-target compilation passes; the feature-gated post-quantum test was restored
  without claiming an execution. Five examples compile, three example mains
  establish five connections, and the published ignored 50-connection test passes.
  Client/server help and both Criterion smoke entries pass; no HTTP serving or
  benchmark performance result is claimed.
- Both source packages have every member read back. The extracted pair builds
  and runs unchanged consumers with the documented root companion patch and
  fixed-lock adapter. A bare Noq package using registry Proto is outside this
  qualification: dependency patches do not propagate. Other standalone vendor
  package/platform builds are not certified. Root workspace library builds use
  the existing pinned lock and complete patch graph.
- Existing relay CLI builds and exits successfully through its help entry.
  Explicit Linux Iroh and relay cdylibs regenerate under the shared cache lease,
  retain raw pins and load successfully. Deliberate stale unversioned cache bytes
  are replaced by an actual rebuilt leaf matching the earlier good artifact.
  Mixed default lib/cdylib output remains an accepted exception; this does not
  establish a C ABI or another platform's artifact compatibility.
- Ten absent mandatory-input cases and six changed source/lock/artifact cases
  refuse before Cargo under normal and optimized Python. The final helper also
  successfully regenerates and loads the relay artifact.
- A fresh current Native/Browser SDK run passes restricted admission, one approval,
  durable materials, both same-identity reopenings, member PONG and natural
  actor/relay/browser/socket/container cleanup. Existing verified raw SDK pins
  were reused because their actual selected sources did not change. The inherited
  M104 controller scope label is preserved and explained in the public receipt.

Both review axes finish with zero remaining findings. The initial missing-input
finding, fix and negative evidence are retained. Raw executable/SDK/WASM/cdylib
pins remain local; private SDK actor logs, stores, invitations and keys are
excluded. Generated TLS keys and NSS were removed after qualified cleanup.

See [distribution boundaries](../../notes/2026-10-19-vendor-distribution.md)
for usable commands and the exact remaining exceptions.
