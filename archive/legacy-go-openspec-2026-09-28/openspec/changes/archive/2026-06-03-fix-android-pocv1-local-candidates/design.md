## Context

The current Android failure is not STUN or MQTT. Android logs show STUN mapped
addresses are present, while direct candidates are empty because Go's standard
Linux interface enumeration calls netlink paths that Android app sandboxes reject.
The shared runtime therefore needs a platform-specific local-address provider,
not a different control plane or a Java/Kotlin-only runtime split.

## Goals / Non-Goals

**Goals:**

- Keep Android Control Lite on the same Go runtime and CLI payload model.
- Gather usable Android local IPv4/IPv6 direct candidates without calling
  `net.Interfaces()` or `net.InterfaceAddrs()` on Android.
- Derive route-selected local source addresses from known peer direct targets
  when enumeration is incomplete.
- Make trace logs sufficient to distinguish interface enumeration failure,
  route-source derivation, direct attempt failure, punching failure, and secure
  session failure.

**Non-Goals:**

- Do not patch Go's standard library or vendor a modified GOROOT.
- Do not move primary candidate collection into Android Java/Kotlin.
- Do not restore TCP punching, CN-STUN arbitration, relay, or broker-side data
  forwarding.
- Do not make Android Control Lite a shell target.

## Decisions

1. **Use a GOOS-specific provider boundary for local candidates.**

   Non-Android builds keep the existing standard-library path. Android builds use
   a dedicated provider that avoids Go's `net.Interfaces()` and
   `net.InterfaceAddrs()` calls.

2. **Implement Android enumeration as netlink-lite plus filtering.**

   Android provider attempts `RTM_GETADDR` without the Go standard library's
   explicit `NETLINK_ROUTE` bind and without `RTM_GETLINK`. The provider only
   needs IP addresses for host candidates; it must not require MAC addresses.

3. **Add route-source derivation as a second candidate source.**

   When peer direct targets are known, the runtime can open a temporary UDP dial
   to each target and read `LocalAddr()` to learn the kernel-selected local
   source IP. The result is combined with the runtime UDP port and added as a
   host candidate for that attempt.

4. **Do not invent a new wire shape.**

   Existing `direct_addrs`, path-policy, and punch response fields remain the
   transport for candidate exchange. Android-specific behavior is an input
   collection detail inside the runtime.

## Risks / Trade-offs

- Android may still block custom netlink on some devices -> route-source
  derivation remains available and logs the netlink failure explicitly.
- Route-source derivation only learns addresses for known targets -> use it as a
  supplement, not as the only candidate source.
- More trace logging can be noisy -> keep candidate values at trace/debug level
  and avoid logging credentials or message payloads.

## Migration Plan

Implement as a current POC v1 runtime behavior change. Existing Linux/Windows
candidate gathering remains unchanged. Rollback is to remove the Android provider
and route-source injection, returning Android to the previous empty-direct
candidate behavior.

## Open Questions

- None. Real-device validation will determine whether netlink-lite, route-source
  derivation, or both supply the final Android direct candidate set.
