# Ordinary portmapper defaults and explicit product gateway choices

MIOPUNCH-98 implements the accepted default-preservation rule in
[ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md).
Baseline: `1ddafa9da91beeefa981771bf2cc53f1239415c4`.

`PortmapperConfig::default()` again returns upstream `Enabled {}`. This does
not enable a Cargo feature that the consumer omitted, or install a native
mapping service in Browser builds. NativeNetwork's saved ResourceConfig still
sets Disabled/NatPmp before binding, including advanced builders and the TCP
factory. Its implementation and all portmapper protocol/ownership Rust sources
are unchanged.

Direct probes, their existing p2panda builder hooks, Browser startup and ordinary
local fixtures now declare Disabled explicitly. The original fixed consumer
sources from previous compatibility issues remain unchanged. The new fixed
ordinary consumer is copied byte-for-byte from its qualified pre-edit fixture;
its serving endpoint leaves the mapping default untouched. The generic p2panda
wrapper retains the underlying library default. Product and fixture hooks make
their own choice without replacing that generic behavior.

The first ordinary default-feature build exposed a separate compile regression:
the restricted endpoint delegated `custom_addrs` without the ordinary method's
`unstable-custom-transports` condition. The delegate now has the same condition.
Both the ordinary default-feature consumer and the portmapper-disabled feature
graph compile without enabling unstable custom transports. No new feature is
forced onto old callers.

| Boundary | Actual result |
| --- | --- |
| Published iroh 1.2.0 ordinary default, before edits | UPnP requests, real kernel forwarding and PING/PONG pass |
| Pre-edit fork ordinary default features | Real E0599 compile failure, retained separately |
| Pre-edit mapping diagnostic graph | Same consumer times out without mapping or gateway protocol packets; this is a diagnostic, not ordinary feature qualification |
| Restored ordinary default features | Same consumer, UPnP mapping and PING/PONG pass; gateway confirms owned deletion |
| Ordinary consumer without portmapper feature | Real build and runtime pass; packet capture has no mapping protocol packets |
| Actual Browser SDK | Compiled iroh features omit portmapper; startup explicitly disables mapping |
| Product saved Disabled / advanced Disabled | No gateway protocol requests |
| Explicit product NatPmp | Actual UDP and TCP mapping, authorized streams and ownership checks pass |
| Direct raw / wrapped / TCP CLI probes | Actual PING/PONG, zero gateway protocol packets and normal process exit |

The original package baseline uses its exact published lock, with existing
source paths and an attached public example. Missing exact pins were fetched
with `--locked`; the published lock and repository lock were not re-resolved.
The consumer uses an isolated nonempty relay map so the upstream net-report
trigger is available; the lab has no public or real LAN route.

The first controlled gateway implemented only NAT-PMP. Diagnostic logging on a
separate throwaway example showed upstream first attempting UPnP. The qualified
lab therefore also implements actual SSDP discovery and SOAP control, installing
and removing real iptables rules. Original and restored consumers use the same
fixture. Failed preparations, diagnostics, the original consumer revision and
Python fixture correction are retained. Neighbor setup is deterministic within
the isolated namespace and does not change production gateway discovery.

Close returning is not proof of a router-confirmed delete. The original baseline
returns normally but its gateway observes no delete request; its owned rule is
removed when the controlled gateway stops. The restored consumer's gateway
observes the delete and the rule is absent before gateway shutdown. These are
separate observations; neither is generalized into a guarantee for arbitrary
routers. The ordinary UPnP check does not claim a new product UPnP/PCP capability.

Twelve product cases pass: UDP positive/Disabled/allocation-timeout/cancellation,
and TCP positive/Disabled/advanced Disabled/advanced TCP disable/TCP disable with
independent UDP/close isolation/allocation-timeout/cancellation. The close
isolation check proves TCP release preserves an independently allocated UDP
mapping using the same port. Foreign-owner rules remain. Explicit NatPmp cases
also capture the absence of PCP and SSDP packets. SDK workers exit zero; copied
lab controllers stop gateway and sentinel through their normal protocol, stop
capture with SIGINT, remove namespaces and owned stores, and report any forced
worker as a failure.

Two existing saved-consent unit checks pass. One had a stale signed-update
fixture: revision 2 equaled the bundled revision and correctly hit rollback
protection. The fixture now signs `config.defaults.revision + 1`; production
update rules are unchanged. The UDP gateway worker preceding that test is
unchanged, and all four UDP cases were rerun with the final test binary. The
eight TCP cases use byte-identical final test bytes. The full affected fixture
typecheck passes; the last two reruns rebuild only the three wrapped fixture
checks and then the one corrected UDP fixture check.

The current Native/Browser SDK story passes restricted request, real approval,
both admission materials saved, both identities reopened, member PING/PONG and
normal socket/actor cleanup. A final fixture audit changed only test sources
outside the SDKs' actual dependency inputs. The SDK and CLI artifacts are reused
after every selected source, lock, tool and artifact hash is checked against
the new runtime guard; their old build receipts remain intact. Generated keys
and private NSS data are removed after verified process/socket/container cleanup.

Builds use the shared target and unchanged profiles and disk limits. Exact
reviewed obsolete intermediate test cache duplicates were pruned with immutable
test bytes retained. Selected intermediate test pins were subsequently compressed
losslessly with hash-verified decompression and original restore paths; every SDK,
actual final test, source receipt and raw log is retained. Capacity aborts occur
before Cargo and do not lower the compile floor.

[Validation, independent reviews and the raw verification archive](../evidence/portmapper-default-compatibility-2026-10-19/validation.json)
record the commands and boundaries. SDK identity/SDP logs and private stores stay
outside committed evidence. The broader public/physical matrix and M91
convergence remain separate.
