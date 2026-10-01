# Startup resources and permission at actual access

MIOPUNCH-38 implements the resource behavior in the
[implementation specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
It uses the existing MIOPUNCH-30 admission exchange, original iroh TLS/QUIC
connections and relay actor. Configuration supplies candidate facilities; it
does not grant membership, genesis authority or application access.

## Bundled instance table and actual capabilities

`ResourceConfig::default()` takes the production table from pinned iroh 1.2.0
([source](../../vendor/iroh/src/defaults.rs)). Table revision 1 contains:

| Origin | Allowed uses | Limitations |
| --- | --- | --- |
| `https://use1-1.relay.n0.iroh.link./` | Observation, DataRelay | number0 public service; best effort; no availability guarantee |
| `https://usw1-1.relay.n0.iroh.link./` | Observation, DataRelay | same |
| `https://euc1-1.relay.n0.iroh.link./` | Observation, DataRelay | same |
| `https://aps1-1.relay.n0.iroh.link./` | Observation, DataRelay | same |

These facilities provide address observation (QAD) and carry encrypted
endpoint packets. They are not membership authorities, trusted directory
responses, MQTT brokers or application plaintext stores. The controlled
default-test table and a self-hosted replacement each completed relay-only
manual admission, durable authorization plus secret delivery, reopen, and
member application communication. Those tests verify the complete mechanism;
they do not claim public-service availability or a production SLA.

Default standalone auxiliary lookup and signaling entries are empty. Existing
trusted invitation contacts, address books and configured lookup services
supply candidates; encrypted relay contact works without an additional
directory or broker. A custom MQTT carrier identifies its broker as Signaling.
Custom auxiliary carriers can declare Lookup separately. Disabling DataRelay
does not disable Observation, Lookup or Signaling. To disable each use, set
`disabled_uses` explicitly. Direct peer UDP/TCP transports do not use an
auxiliary facility. Remote service ACL design remains outside this ticket.

## Persistence, replacement and trusted update

`NativeNetwork::resource_configuration()` loads private `resources.json`; a
missing file uses the bundled table. `save_resource_configuration()` writes a
private file, fsyncs it, atomically renames it and fsyncs the node directory.
Supported node backup/restore includes this file.

The `replacement` array replaces the complete bundled resource list. `null`
uses defaults; `[]` explicitly supplies no auxiliary facilities. `disabled`
contains `[origin, use]` pairs, and `disabled_uses` disables a purpose globally.
Origins are normalized service identities: paths, query strings, fragments,
equivalent slash spelling and a DNS absolute-name suffix cannot bypass a
local disable. Credentials embedded in resource URLs are rejected.

`SignedResourceTable::sign` uses the existing Ed25519 API and signs the exact
domain-separated table bytes. Applying an update requires an explicitly
configured `trusted_update_key`, a valid signature, the correct domain and a
strictly increasing revision. No publisher is trusted automatically. A client
release can update its bundled table; a locally trusted publisher can supply
a signed update. There is no automatic remote update downloader in this scope.
Publisher trust is separate from membership. Signed updates cannot alter
local replacement, disable entries, LAN disclosure mode or the trusted key.
Personally trusted replacement imports also preserve those local settings.

The thin command shell exposes the same implementation:

```
rtk cargo run --locked --bin miopunch-native-probe -- network resources DATA_DIR
rtk cargo run --locked --bin miopunch-native-probe -- network resources-import DATA_DIR RESOURCES_JSON
rtk cargo run --locked --bin miopunch-native-probe -- network resources-update DATA_DIR SIGNED_UPDATE_JSON
rtk cargo run --locked --bin miopunch-native-probe -- network run DATA_DIR
```

`RESOURCES_JSON` is an array of resources, for example:

```json
[
  {
    "url": "https://relay.example.org/",
    "uses": ["Observation", "DataRelay"],
    "limitation": "personally trusted self-hosted relay; operator-defined availability"
  }
]
```

Only import resources that the operator personally trusts. Publisher-key and
disable settings are explicit configuration/API inputs. An update/import is
effective on the next endpoint start. Existing endpoints retain their startup
snapshot; runtime hot modification is not implemented or claimed.

## Candidate sources and final permission

`RelayMap` lists preferred relay candidates and is not an allowlist. Resource
qualification is installed through `NativeNetwork::bind_endpoint_with` as
well as normal `bind_configured_endpoint` startup. Default tables,
invitations, p2panda address books, lookup responses, cached routes and the
actor's known-route substitution cannot grant access to a forbidden origin.

The existing relay actor checks the finally selected URL before opening or
using a handle. Observation maps are qualified independently before periodic
reports and external mapping sequences. Explicit QAD maps remain QAD-only;
they do not silently add HTTPS/captive-portal probes. Custom auxiliary carrier
facilities are checked before binding and at each send. Built-in Pkarr
publication/resolution and DNS lookup declare their auxiliary origins. The
address-lookup collection checks permission before publishing (including the
immediate publish when adding a service) and before resolving; Arc and
address-filter wrappers retain those declarations. DNS lookup origins are
represented as `dns://ORIGIN_DOMAIN/`; queries use the endpoint's configured
or system DNS resolver. Memory lookup and explicitly configured LAN discovery
have local scope. External lookup implementations must declare their facility
through the same trait. Arbitrary transport address tokens do not confer
permission.

Native Pkarr HTTP clients do not follow redirects. Canonical facility URLs
must be imported/listed explicitly; a redirect response becomes a failed
contact rather than substituting another origin. The browser reqwest backend
does not expose a Fetch redirect policy (pinned reqwest 0.13.5
`src/wasm/client.rs`), so external lookup services fail closed under an
explicit resource policy on WASM. In-memory lookup remains usable. Current
BrowserNode uses a configured relay and installs no external lookup service;
constrained browser lookup capability is not claimed. WASM checks verify
compilation, not browser runtime access enforcement.

An allowed data-only home relay no longer depends on observation permission.
The actor tries another allowed entry if its initial home fails, then uses the
stack's bounded retry backoff when all entries fail. `ResourceEndpoint::connect`
keeps the original iroh path engine, filters relay hints and supplies configured
fallbacks. Contact attempts default to 10 seconds and can be bounded to at most
30 seconds. Failures report `TemporarilyUnreachable` with a per-peer retry delay
of 1–32 seconds; an early retry spends no new attempt. A successful connection
clears the delay. Import/update followed by restart restores recovery without
asserting that every network member is definitively offline.

## Explicit LAN disclosure and upstream branch differences

Native startup preserves its existing Disabled default. Disabled installs no
mDNS service. Explicit Active publishes node identity and addresses under
`miopunchv1`; explicit Passive uses the low-level receive-only mDNS service.
Actual wire discovery plus a public restricted admission request verifies
Passive without publishing its identity. Observation permission and public
data-relay permission do not silently enable LAN publication.

The upstream high-level p2panda Network chooses Active by default
([pinned source](https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/src/network.rs)).
The reusable `p2panda_net::MdnsDiscovery` actor defaults to Passive, but its
current Passive branch returns before constructing a discovery service
([actor](../../vendor/p2panda-net/src/iroh_mdns/actor.rs)). Actual explicit
high-level Active publication succeeds; its default Passive branch neither
publishes nor discovers. Native receive-only discovery therefore uses the
low-level implementation directly rather than claiming the high-level
Passive branch works. This preserves the accepted native disclosure choice;
it does not adopt the high-level Active default as a new product decision.

## Validation scope

The source-fixed runner is `scripts/check_resources.py --suite --output NEW_DIR`.
It snapshots source and harness before execution, records actual command exits
at every boundary, preserves raw logs and executable hashes, and creates and
removes only its owned MQTT broker and anonymous volumes. Incremental runs
retain genuine compile/fixture failures, public REDs and GREENs separately.
Read [the evidence](../evidence/mio38-resources/README.md) for exact generations,
results, source reuse and unchanged NAT exclusions.

The runtime checks use Linux, loopback controlled facilities, real Ed25519,
SQLite/private-file persistence, TLS/QUIC, actual relay/mDNS/broker accesses
and real CLI child processes. WASM validation is compilation only. No public
availability guarantee, live configuration update, new NAT probability
denominator, other-platform runtime, product UI, full TUN/proxy, KCP, Brutal,
custom congestion control or service ACL is claimed.
