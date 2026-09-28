## Why

Portable Windows/WSL testing exposed three coupled gaps in the current POC
broker story:

- blank first-run nodes need an owner-candidate UI so they can create or join a
  network without pre-created state;
- owner/admin nodes need a clear rule for choosing the network's runtime broker
  set from explicit local configuration or built-in defaults;
- post-join signaling must keep using the same network broker set instead of
  falling back to each peer's pre-join default broker.

The current change documents still describe a minimal single-broker fix based on
`brokers_effective[0]`. That no longer matches the intended POC behavior.

The desktop also labels a peer as `selected`, which currently means "chosen as a target neighbor", not "connected"; this is easy to misread while debugging first real-machine loops.

## What Changes

- Add a documentation-first broker charter that defines:
  - first-run owner-candidate UI boundaries,
  - explicit versus built-in broker candidate sources,
  - runtime `brokers_effective` primary/secondary semantics,
  - separate `invite_brokers` selection for invite/join exchange.
- Promote `brokers_effective` from "save the first broker" to "propagate and use
  the current runtime broker set (up to primary + secondary)".
- Keep `invite_brokers` as the invite/join-only entry set, and prefer brokers
  outside the current runtime pair when alternatives exist.
- Persist and propagate the full effective broker set through membership, peer
  seed state, and later peer signaling rather than reverting to pre-join
  defaults.
- Clarify desktop peer status so `selected` does not look like an online state, and show recent failure evidence when a selected peer is not active.
- Add focused acceptor lifecycle/attempt logs so incoming signaling and stream failures are visible in portable bundle logs.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-poc-invite-join-approve-v0`: invite, join, and approve align on a
  document-defined distinction between `invite_brokers` and the network runtime
  `brokers_effective` set.
- `miopunch-poc-control-plane-mailbox`: after joining a net, MQTT signaling for
  peer seed configs and runtime peer tasks uses the network's effective
  primary/secondary brokers rather than pre-join defaults.
- `miopunch-desktop-gui-v0`: peer status distinguishes selected target candidates from active connected peers and surfaces recent failures.

## Impact

- Affects `docs/decisions`, glossary terms, and the OpenSpec changes that define
  POC broker behavior.
- Affects `internal/task` invite/join/approve/bootstrap state handling and
  focused tests once implementation resumes.
- Affects `internal/pocacceptor` logging only; no protocol or data-plane relay behavior changes.
- Affects desktop frontend status labels and tests.
