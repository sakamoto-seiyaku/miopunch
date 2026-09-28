## 1. OpenSpec and Candidate Provider Shape

- [x] 1.1 Create change artifacts for Android local candidate provider and route-source derivation.
- [x] 1.2 Introduce a local candidate provider boundary that preserves existing non-Android behavior.
- [x] 1.3 Add Android build-tagged local address enumeration that avoids Go standard interface enumeration.

## 2. Route-Source Candidate Derivation

- [x] 2.1 Add UDP route-source derivation from known peer direct targets.
- [x] 2.2 Merge derived candidates with enumerated candidates while preserving IP-family policy filtering.
- [x] 2.3 Ensure Android no-candidate paths do not publish loopback fallback as a direct peer candidate.

## 3. Diagnostics and Tests

- [x] 3.1 Add trace/debug diagnostics for Android provider, route-source derivation, and final direct candidates.
- [x] 3.2 Add focused tests for address filtering and route-source derivation.
- [x] 3.3 Run focused Go tests for changed packages.

## 4. Android/Linux Real Validation

- [x] 4.1 Rebuild Linux CLI and Android Control Lite APK from the changed tree.
- [x] 4.2 Install the APK, clear Android app data/logs, and start both runtimes with trace logs.
- [x] 4.3 Run create-network, invite, join, approve, ls, Android-to-Linux ping, and Android-to-Linux sh ls.
- [x] 4.4 Capture Android and Linux logs with candidate-source and selected-path evidence.
