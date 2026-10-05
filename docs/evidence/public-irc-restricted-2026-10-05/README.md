# Public IRC restricted-session qualification

2026-10-05. [MIOPUNCH-72](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/33eed8c2-e556-476b-8879-ca1f6b6a5eba) concludes **available=false for the explicitly tested public slow profile**. The final runner exits 0 because it measured the negative outcome and cleanup; it does not qualify IRC restricted control, ordinary data, or the future unified SDK protocol.

The [official testnet page](https://testnet.ergo.chat/) permits non-abusive client testing and publishes TLS 6697 and WSS `/webirc`. This investigation uses those public facilities with two clients and a fresh random channel. Chromium actually navigates to that HTTPS page (HTTP 200), then loads the local SDK JS/WASM in its execution context. The page response and WebSocket Origin header are not replaced. CDP records the browser-generated `Origin: https://testnet.ergo.chat` and WSS 101 with `text.ircv3.net`. Native TLS uses the existing webpki public roots, hostname verification, TLS 1.3, and a saved leaf certificate hash. This is an explicit fixture configuration, not a default service selection or deployment on the service's website.

The existing formal `BrowserNetwork.irc_restricted_probe` runs the actual admission request through the original QUIC/TLS bridge, invitation contact checks and admission gate. Native uses a durable `NativeNetwork`, its original gate and `serve_manual_admission`; private stores are removed after the process exits. The old admission-request/1 protocol is the investigation scope. New `miopunch/session/member/1` or later unified SDK public bootstrap must be qualified separately.

| Qualification layer | Final observation |
| --- | --- |
| Public facility TLS/WSS | Normal verified native TLS, genuine HTTPS origin, WSS 101, IRC registration/channel ready |
| Full restricted QUIC/TLS plus gate within caller budget | Failed at `connecting`; caller elapsed 12,008 ms; result null; end-to-end identity and admission gate not reached |
| Restricted control request | None accepted; native request table empty; no restricted A returned |
| Ordinary data / validated data path | No handle returned; no business permission, ordinary IP transport, relay or RTC path in this fixture |
| Future unified protocol / public NAT data | Not tested or qualified by this investigation |

The profile remains 600 ms fragment pacing, 128-byte chunks, 8,000 ms assembly TTL, 1,500 ms queue admission TTL and bounded 2,048-byte datagrams. Caller budget is the existing startup-inclusive 12 seconds; the existing admission agreement would have at most its original five seconds and the caller's remaining budget. No deadline, pacing rate, QUIC Initial size, service policy or product configuration is raised to pass the test. The carrier's 60-second maximum lifetime is inherited from the existing fixture profile, not the caller deadline.

## Complete packet and stop-branch evidence

The final browser accepts nine 1,200-byte Initial emissions (10,800 bytes); these include subsequently dropped queue entries and are not nine completed transmissions or an exact retransmission counter. One complete 1,200-byte Initial is actually sent and reassembled natively. Its first and tenth browser fragments are emitted at approximately 1,742 and 7,171 ms after carrier start. Native receives 1,200 bytes and completes one full 1,200-byte response transmission. The browser receives only seven response fragments, approximately 7,841–11,451 ms, before caller retirement; its complete received datagram count is zero. It holds one incomplete assembly at the failure boundary, then clears it.

Browser queue peak/drop count is 5/4. Native accepts 3,738 bytes in four queued datagrams (1,200, 138, 1,200, 1,200); complete sent/received counts are 1/1 and queue peak/drop count is 4/3. The native complete-send count is collected after the browser caller ended, not proof of full response delivery within its deadline. Accepted bytes must not be confused with successfully sent bytes. IRC line maximum is 435 bytes. Fragment timestamps, ciphertext hashes, traffic counters and CDP frame byte totals are in [final/qualification.json](final/qualification.json). The first-byte/version observations prove that the Initial remains at least 1,200 bytes.

The public service permits and delivers normal traffic; the failure is full QUIC response completion under this profile's unchanged caller budget, before end-to-end authentication. Identity match fields are null because that stage was not reached, rather than false assertions of a tested identity rejection. No gate or allowed application control request is treated as successful. The caller records the original temporarily-unreachable error and stops the IRC branch with `available=false`. MQTT remains independent: this IRC-only runner installs no MQTT facility, modifies no MQTT code or budget, and does not repeat the already-qualified [mixed retry](../../notes/2026-10-04-browser-irc-restricted-handshake.md) story. The prior owned Ergo slow-profile failure is retained separately and is not superseded.

A second independent negative call configures a contact route for a different identity, leaving no route for the invitation contact. It fails in 1,611 ms with `no allowed contact route`, accepts/sends zero QUIC bytes, returns no data handle, and saves no authorization or secret. Startup still performs ordinary WSS registration before checking the target route. This checks wrong-contact **route matching**, not authenticated TLS wrong-peer rejection; the latter is not advanced after restricted-session qualification failed.

## Cleanup, retained first failure, and reproduction

The expected-contact browser's separate graceful QUIC drain times out after 2,001 ms and is recorded as `graceful_close=false`; it is never relabeled graceful. Carrier retirement leaves `socket_state=3`, zero queue, assembly and RX entries, zero waiters and no timer. The runner waits 301 ms for asynchronous CDP WSS close delivery before destroying its owned browser context, terminating its remaining browser jobs. Both WSS sockets are closed at immediate and 500 ms late observation. Native carrier cleanup finishes in 13 ms with driver/queue/assembly zero, graceful endpoint close true and process exit 0. Final runner cleanup takes 706 ms; no process is force-killed. No PC, container, local listener or forwarding process is created.

The retained [first/qualification.json](first/qualification.json) and [first/results.json](first/results.json) record the first actual public request: caller 12,001 ms, no session/gate, normal TLS/WSS, runner exit 1. Its second WSS close CDP event was not observed before browser context destruction although the SDK recorded CLOSED and empty resources. That observation race was fixed by awaiting delivery before context shutdown. It remains a failed measurement receipt; the final run uses the same caller/stage/profile and does not erase it. [manifest.json](manifest.json) records original and exported hashes. Export removes public endpoint identities from the native member array, retaining its count. Receipts contain no invitation, raw SDP, exporter, secret, private key or node store.

Native/WASM builds, bindgen and the final protocol invocation all exit 0 in [final/results.json](final/results.json). Source hashes before and after the final build are equal; executable, generated JS and WASM hashes are saved. Their source tree includes the concurrently developed changes named in the hash manifest; HEAD alone is not claimed as the artifact's exact source. The final runner duration is 23.720 seconds inside its 50-second work guard plus ten seconds cleanup reserve. Build budgets are separately recorded. The first and final private fixtures are removed; the final removal boolean is retained. Affected IRC datagram regressions are recorded in [validation.json](validation.json).

The exact owned runner, wrapper and native fixture sources used by that final invocation are preserved under [sources/](sources/), with hashes matched to its receipt. Subsequent concurrent Cargo builds can replace `target/debug/browser_irc_restricted_native`; its current pathname must not be mistaken for the historical hashed executable. The recorded historical source/artifact identities and raw exits are retained unchanged.

## Review fixes and local validation

The concentrated review reports Standards: two P2 findings; Spec: zero required findings, with two matching robustness observations. The historical exported repository evidence already omitted member identities and the measured native exit was actually 0 with graceful cleanup. Review identified gaps in future runner executions: nested `members` could be written before manual export sanitization, and a native exit 1 or nongraceful receipt could leave `passed=true` merely because the process exited.

The current runner uses the concrete `public-irc-receipt.mjs` serializer and cleanup predicate. Serialization automatically replaces nested member arrays with counts. Every native process exit saves its actual code and signal before resolving or rejecting waiters, including exits before cleanup starts. Passing now requires native exit 0 without a signal/forced kill, a graceful `closed` receipt, zero native driver/queue/assembly, no browser interval, and complete immediate and late socket observations. Five local Node tests exercise the real historical final receipt plus privacy, exit 1, signal, nongraceful, missing receipt, queue/assembly/driver residue, forced kill and socket/interval failures. They pass and keep historical `available=false`; their command, exit and log are in [validation.json](validation.json) and [receipt-tests.log](receipt-tests.log). The current helper source is included in future wrapper hash capture.

These post-review runner changes were verified locally, with **no third public run**. They do not rewrite or re-qualify the historical final result. [manifest.json](manifest.json) distinguishes the saved final-run source hashes from the current post-review runner hashes.

```sh
rtk proxy python3 scripts/check_public_irc.py \
  --facility docs/evidence/public-irc-restricted-2026-10-05/facility.json \
  --output /tmp/mio72-public-irc-new
```

The facility must be supplied explicitly. The runner generates a fresh channel, never installs the service as a product default, and cannot return a normal data handle. This document records a negative qualification result, not a task backlog or a recommendation to increase public load.
