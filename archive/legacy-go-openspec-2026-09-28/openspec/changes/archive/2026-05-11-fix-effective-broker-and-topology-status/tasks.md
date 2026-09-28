## 0. Documentation Alignment

- [x] 0.1 Create a broker selection charter under `docs/decisions` that defines first-run owner-candidate, explicit broker configuration, `brokers_effective`, and `invite_brokers`.
- [x] 0.2 Align glossary terms and relevant OpenSpec changes with the new charter terminology.

## 1. Broker Configuration And Selection

- [x] 1.1 Extend `local.mqtt_broker` persistence to accept old string and new array forms, while writing arrays on new saves.
- [x] 1.2 Distinguish explicit local broker configuration from built-in fallback-only defaults.
- [x] 1.3 Add broker selection helpers that normalize, de-duplicate, probe reachability, preserve source order, and choose at most two runtime effective brokers.
- [x] 1.4 Add invite broker selection helpers that prefer candidates outside the current `brokers_effective` pair, but may fall back to that pair when alternatives are insufficient.

## 2. Invite, Membership, And Runtime Propagation

- [x] 2.1 Update `invite` to derive and persist the current `brokers_effective` pair from explicit or built-in candidates before emitting the invite code.
- [x] 2.2 Keep `join` and `approve` pinned to `invite_brokers` during the invite/join exchange.
- [x] 2.3 Propagate the full `brokers_effective` set through membership bundle, local state, saved peer configs, seed peer payloads, and hello/runtime metadata.
- [x] 2.4 Update post-join runtime signaling consumers (`pocacceptor`, peer dial, `bootstrap_more`, related helpers) to use primary/secondary runtime brokers rather than a single saved broker.

## 3. Observability And Desktop Status

- [x] 3.1 Keep first-run owner-candidate UI semantics aligned with the broker charter while preserving lazy backend state.
- [x] 3.2 Preserve desktop selected-versus-active peer wording and recent failure evidence.
- [x] 3.3 Preserve sanitized `pocacceptor` lifecycle and attempt logging.

## 4. Validation

- [x] 4.1 Add or adjust focused tests for string/array config compatibility, broker selection, invite broker fallback, membership propagation, and runtime primary/secondary use.
- [x] 4.2 Run focused Go and frontend tests for touched code.
- [x] 4.3 Run `openspec validate fix-effective-broker-and-topology-status --strict` and `git diff --check`.
