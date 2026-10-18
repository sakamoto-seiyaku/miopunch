# Ordinary pkarr redirects and explicit constrained lookup

MIOPUNCH-99 implements [ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md).
Baseline: `67d4f496030618f3658beafbf0e2eb26e772ca1d`.

Ordinary native `PkarrRelayClient::new` and `PkarrRelayClientBuilder::build`
again use the original reqwest redirect policy. The two constructors now share
one implementation. A new public `PkarrRelayClient::builder` makes the existing
builder accessible; upstream exposed `build` but no public field constructor.
The old signatures and publisher/resolver defaults remain available.

`resource_policy` on the client, publisher and resolver builders explicitly
selects constrained requests. Native requests disable all redirects and check
lookup permission before every publish or resolve, including background
publication retries. Browser constrained clients refuse requests before Fetch,
which cannot provide equivalent redirect control. Ordinary Browser Fetch
behavior remains available outside explicit constraints.

The standard endpoint builders inherit an explicit startup resource policy.
A caller's additional policy is intersected with it, so neither can broaden
the other. Existing ResourceConfig already installs that explicit endpoint
policy; its saved permission implementation is unchanged.

Prebuilt ordinary pkarr services need separate treatment because the blanket
AddressLookupBuilder implementation returns an existing service unchanged.
Explicit-policy registries now require an external service to declare constrained
resource access before checking its facility permission. Ordinary pkarr services
lack that declaration; explicitly constrained services have it. Arc and filtered
wrappers preserve it. DNS lookup declares its configured DNS query behavior.
The trait method has a conservative default and does not change ordinary
endpoints or require edits to original implementations. This is a declaration
by a third-party implementation, not proof that arbitrary third-party code is
safe or a sandbox around a caller's independently started tasks. Browser external
services under an explicit policy retain their existing blanket refusal.

The unchanged public consumer was qualified against the published iroh 1.2.0
source and exact published lock before production edits. An owned 307 service
and separate signed-packet store observe actual PUT and GET requests; the
original succeeds and the old fork fails without reaching the target. The final
fork succeeds with the same consumer bytes. A separately labelled test-only
source overlay constructs the otherwise inaccessible upstream builder using its
private fields and exercises the unchanged original build method. It is not
presented as an old public constructor or a production API. Both historical
manifest versions are retained byte-for-byte.

Nine selected product tests use their original source functions and fixture
bytes, excluding unrelated mDNS and CLI tests through a temporary selector.
They cover permitted signed lookup followed by real restricted admission,
Disabled lookup/publication, post-bind wrappers, denied redirect resolution,
publication retry over separate HTTP connections, live permission revocation,
policy intersection and prebuilt-service rejection before/after bind. The
selector retains every foreign lock row; a first broader selector's omitted
unrelated test dependencies and its locked mDNS feature mismatch remain recorded
as failed preparations. No lockfile is re-resolved.

A separate Browser WASM consumer runs in Chromium against owned redirect and
storage servers. Ordinary Fetch follows the cross-origin redirect and verifies
the signed packet. Constrained raw, publisher/resolver and registered services
send no additional HTTP requests. Browser contexts/processes and both HTTP
servers close normally. The Browser probe's URL type correction is a fixture
change, with its original bytes retained; actual product-test inputs are unchanged.

[Validation and reviews](../evidence/pkarr-redirect-compatibility-2026-10-19/validation.json)
record the final Native/Browser SDK regression and current selected-input hashes.
The SDK story includes actual restricted admission, approval, both materials
saved, both identities reopened, member business and normal actor/socket cleanup.
Private identity/invitation/store logs are excluded from committed evidence.
The broader public/physical matrix remains separate under M91.
