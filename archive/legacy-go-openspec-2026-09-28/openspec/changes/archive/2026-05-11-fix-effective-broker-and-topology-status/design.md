## Context

The invite flow already distinguishes `invite_brokers` from long-lived network
state, but the written change artifacts still assume a minimal single-broker
fix. Product intent has now moved further:

- blank first-run desktop nodes can act as owner candidates for UI visibility;
- owner/admin broker selection should come from explicit `local.mqtt_broker`
  configuration when present, and only fall back to built-in broker defaults
  when absent;
- runtime `brokers_effective` should represent the network's primary/secondary
  brokers, while `invite_brokers` stays specific to invite/join exchange;
- post-join signaling must keep using that effective broker set instead of
  falling back to pre-join defaults.

The gap is therefore broader than "save `brokers_effective[0]`". The change now
needs a documentation-first reset so the next implementation pass does not keep
mixing minimal single-broker behavior, first-run UI assumptions, and runtime
signaling semantics.

## Goals / Non-Goals

**Goals:**

- Document a single POC broker charter for first-run, invite generation, join,
  approve, and post-join signaling.
- Make the current net's effective broker set authoritative for post-join peer
  signaling.
- Keep POC broker selection simple: reachability first, stable source order,
  primary/secondary only.
- Make desktop status and logs sufficient to diagnose selected-but-not-active peers.

**Non-Goals:**

- No whole-network broker rotation or adaptive latency-driven reordering in this
  change.
- No MQTT data-plane relay or shell payload relay.
- No broad topology/session redesign in this change.
- No new broker configuration field name; the explicit local entry remains
  `local.mqtt_broker`.

## Decisions

1. Keep first-run owner candidate as a UI-only state.
   - Blank desktop nodes may expose owner/admin entry points before network
     creation.
   - This does not imply that `brokers_effective`, `net.json`, governance, or
     decl state already exists.

2. Treat `local.mqtt_broker` as the explicit broker candidate source.
   - The persisted field name stays `local.mqtt_broker`.
   - Future implementation must read both old string and new array forms, then
     write back arrays.
   - Built-in broker defaults are used only when no explicit local broker list
     exists.

3. Define `brokers_effective` as the runtime network primary/secondary list.
   - Selection order is: normalize -> dedupe -> reachability -> preserve source
     order among reachable candidates -> keep at most two.
   - The first endpoint is primary; the second is secondary.
   - Runtime signaling may use the secondary endpoint when the primary fails.

4. Keep `invite_brokers` separate from `brokers_effective` when possible.
   - Invite generation uses the same candidate source as runtime broker
     selection.
   - It should prefer reachable candidates outside the current
     `brokers_effective` pair.
   - If there are not enough alternatives, it may reuse current effective
     brokers rather than fail invite creation.
   - `join` and `approve` still use only the `invite_brokers` carried in the
     code during invite/join exchange.

5. Normalize seed peer and membership broker values at membership boundaries.
   - `membership_bundle` carries the full current `brokers_effective` list.
   - On `join`, save the full effective broker set into local signaling state
     before future runtime tasks.
   - On `approve`, save the joiner's peer config with the same full effective
     broker set before future `ping` or `sh` signaling.
   - Seed peer, hello, and runtime dial paths must be able to carry that full
     effective set, not only the first endpoint.

6. Keep UI status semantic names separate.
   - `active` remains the connected state.
   - A selected neighbor candidate is displayed as a target candidate, not as an online state.
   - Recent topology failures are surfaced in peer detail when no active edge exists.

7. Add acceptor logs at operator-relevant lifecycle points.
   - Log sanitized broker endpoint, SID, local peer, gather/listen readiness, incoming attempt, attempt result, and stream handling failures.
   - Do not log invite codes, secrets, raw topics, shell payloads, or private keys.

## Risks / Trade-offs

- [Primary/secondary needs wider payload changes] The current code stores only a
  single peer/local broker string in several places. This change therefore
  requires coordinated updates to state, seed peer, hello, and runtime
  signaling payloads.
- [Existing old state remains split] Existing extracted bundles may need `data/` reset or manual state migration for clean testing.
- [Incoming sessions still have limited topology integration] The UI wording and logs make this visible; full incoming active-edge accounting can follow if needed.
