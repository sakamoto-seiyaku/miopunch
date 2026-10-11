No documented-standard violations found in the pinned 14-file WIP diff against `05f99e3d078e17c69c53419faf681dec6cadec5a`.

1. **Possible Duplicated Code — judgement call.** The new CID tuning decision is independently encoded in `src/native_network.rs:678` and `browser-probe/src/network.rs:1875`, and the policies already differ:

   ```rust
   // Native
   resources.udp_direct_disabled && resources.tcp_direct_disabled
       && (!resources.irc_resources().is_empty()
           || !resources.signaling_resources("ircs").is_empty())
   // Browser
   irc.is_empty() || !mqtt.is_empty() || !relays.is_empty() || rtc_effective
   ```

   Native selects two CIDs even with MQTT, an enabled relay or RTC, or an advanced builder that does not install the configured IRC carrier. Browser excludes those carrier combinations but does not exclude enabled member-data forwarding. This makes the tuning broader than the “narrow IRC-only control bootstrap configuration” described in `vendor/iroh/UPSTREAM.md:246`. Centralize the common tuning eligibility rule around actual installed/permitted carriers, with explicit platform inputs, or explain and independently qualify the different policies. This is a heuristic maintainability finding, not an accepted-ADR breach or demonstrated security defect.

The request shortcut is paired with `CoordinatorHook` in both native builder branches; raw gates retain the echo. Browser requests still await `coordinate_session`. These preserve the common identity/admission boundary required by ADR 0009. The reduced receive window retains the five-slot ring and separate multipath limit; reservation, retirement and rotation changes show no concrete correctness defect in static inspection.

No compilation or tracker mutation performed. Public repeated measurements remain pending; this review does not establish qualification of the full public cohort.
