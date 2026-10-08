# Current formal SDK IRC contact validation

2026-10-08. [MIOPUNCH-86](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/05b83d41-aa03-4003-8676-559a7f649fa9)
fixes one browser contact-registration omission. Current public IRC restricted
admission and business qualification remain incomplete under MIOPUNCH-84.

The earlier [public IRC evidence](../public-irc-restricted-2026-10-05/README.md)
used a 12-second caller and predates the current common connection protocol.
Those original failures remain preserved; they do not establish the behavior
of today's 60-second formal SDK caller. This work uses
`BrowserNetwork.start/request_admission`, rather than the old diagnostic entry
whose accepted budget is capped at 45 seconds.

## Reproduced omission and fix

The first current public Ergo case completed normal HTTPS, TLS/WSS,
registration and channel qualification. The formal request then returned
`no currently permitted data path` in 6.8 ms, with zero accepted QUIC bytes
and zero sent packets. The browser constructed the common coordinator with
MQTT's contact transport ID alone, so its address filter removed the installed
IRC route before any connection I/O.

The one-file fix adds installed IRC to that contact registry, alongside the
existing MQTT entry. Permission, identity, admission, purpose, cancellation,
data-release rules and protocol bytes are unchanged. It does not enable IRC
as an application-data path or change any profile/default budget.

## Current checks

| Case | Result and qualification boundary |
| --- | --- |
| Public Ergo, original 600 ms pacing / 1.5 s queue TTL / 60 s caller | The immediate route rejection disappears. Encrypted QUIC traffic, target TLS identity and the existing stage agreement are observed. The current purpose-2 exchange does **not** complete: the caller fails at 60,002 ms and the fixture carrier retires at its own 60 s lifetime. |
| Public Ergo, diagnostic 5 s queue TTL, all other profile/caller values unchanged | Still fails at 60,001.5 ms, this time during the existing stage agreement. Browser queued-packet rejections fall from 39 to 9, but differing live outcomes do not establish a sole queue cause. No product configuration is changed. |
| Owned local IRC, no server fakelag, 10 ms pacing, same current formal SDK and 60 s caller | `AwaitingApproval` at 1,216.3 ms; current version 1 / purpose 2 / restricted-only coordinator facts and expected TLS identity pass; native records one unapproved request. This is a controlled regression, not the public-service profile. |
| Owned local MQTT with IRC explicitly disabled | `AwaitingApproval` at 111.3 ms, with the same current purpose-2 and native-request assertions. Existing MQTT contact remains functional. |
| Three IRC disable aliases and an empty contact configuration | Real localhost secure-context storage/Web Locks prerequisites are checked. Each configuration reaches the expected depleted-carrier startup rejection, with zero WebSocket attempts and zero subsequent HTTP requests. |

[public-qualification.json](public-qualification.json),
[owned-regressions.json](owned-regressions.json) and
[disabled-boundaries.json](disabled-boundaries.json) contain whitelist exports.
The public first-stage agreement duration is 22,854 ms; this is time inside
that gate, **not** total TLS setup time. Native manual-inbox status was not
sampled after the post-fix public carriers had already retired; the exports
use `null` rather than treating absent status as a negative inbox observation.

No approval, admission-material delivery, member proof, RTC upgrade or ordinary
business exchange is attempted in these cases. Local restricted success does
not qualify any of those later stages. The public IRC branch remains stopped
at its observed failure boundary. This work is not physical-WAN or NAT evidence.

## Build and evidence boundaries

The existing shared Cargo target is used through `scripts/cargo_cache.py`, with
unchanged committed profiles and locked/offline dependencies. The affected
release WASM build takes 39.508 s, with 295 Fresh artifacts and one rebuilt
crate; 3,159 selected inputs, 198 repository source copies and 264 depfiles are
retained. [build.json](build.json) records the immutable loaded artifact hashes.
The already pinned native helper is reused after verifying its selected graph;
the changed browser file is not an input to that graph. The helper uses the
same original incoming deadline throughout and does not change native SDK code.

Every accepted run checks source, lock, selected-input and artifact guards
before actors and after runtime. Original public RED and post-fix failures,
private raw receipts, pinned SDK artifacts and the initial private-helper
dependency build failure remain intact. Early empty-page boundary checks are
invalid: they stopped at storage availability, even where their first harness
mistakenly reported a pass. Those raw results are preserved and are superseded
by the final storage-capable boundary checks. A subsequent harness assertion
also incorrectly required successful startup when all carriers were disabled;
its failed result remains preserved.

All local SDK closes return and free their handles; owned contexts, sockets,
servers and two uniquely named containers close naturally. In both post-fix
public cases, SDK `close()` does **not** return within the local six-second
cleanup observation. WSS closure is separately observed before forced context
retirement; native exits normally and its PID is absent immediately and later.
Do not describe these public SDK closes as graceful. Owned fixture stores are
removed after process retirement.

[receipt-manifest.json](receipt-manifest.json) binds exports to private originals
and controllers. Invitations, ephemeral endpoint IDs, QUIC contents, keys, CA
private material, SDP, candidates and captures are excluded. Existing 234 WIP
files are verified unchanged. The public queue diagnostic initially received
an automatic approval rejection; the user then explicitly authorized that
specific Ergo action, which ran once. There is no pending approval for it.
