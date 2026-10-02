# MIOPUNCH-45 incremental Standards review

Reviewed cleanup diff `4c4b5dc387a56f9d968d2d49cf33cb098052131e...b4ec3c29fbf178fcb8b2564f22eededdb3820578` only. The original [Standards review](standards-review.md) is retained unchanged. Previously established repository standards and smell-baseline rules remain applicable.

All three original non-blocking observations are resolved:

- `src/resources.rs`: `mqtt_resources` and `irc_resources` now call the same private `signaling_resources(scheme)`. Parsing, permission filtering and origin deduplication have one implementation; carrier-specific entry points remain clear.
- `tests/irc_bootstrap.rs`: both fixture clients now use `registered_irc_client`. Registration and channel-join handling have one implementation, with bounded waiting and explicit failure if the real IRCd disconnects before registration completes.
- `src/irc_transport.rs`: `DriverBinding` gathers route, service, address watcher, status and traffic. `drive`, `read_loop` and `write_loop` borrow that owned binding; channels remain explicit. Driver completion still withdraws addresses, closes status and clears pending observations. Endpoint cancellation retains its existing owned-driver behavior.

No new documented-standard violation or actionable baseline smell was found in this cleanup. The private selector and binding type address demonstrated duplication and parameter grouping; they do not introduce unsupported runtime extension mechanisms or new product decisions.

Reviewed existing post-cleanup receipts: `review-refactor/results.json` reports `passed=true` for ten real IRC cases and the isolated UDP data-cut; `review-mqtt-filter/results.json` reports `passed=true` for configured MQTT startup; `review-compile/results.json` reports successful current-hook/default-native compilation and formatting. No checks were rerun, no slow NAT matrix was repeated, and no runtime/test files were edited by this reviewer.

Summary: 3 original observations resolved; 0 remaining Standards findings; 0 new Standards findings.
