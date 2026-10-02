# Spec review

Independent GPT-6.1 Sol HIGH; same snapshot/baseline as Standards.

Spec review: **0 findings** against MIOPUNCH-41 and relevant MIOPUNCH-14 requirements.

- **Missing or partial requirements:** None found within the ticket’s declared scope. Independent TCP listener ownership and existing candidate/connection integration appear in `src/resources.rs` and `src/tcp_transport.rs`. Recorded positive evidence exercises real TCP forwarding, node authentication, manual admission, persisted materials and member data exchange.
- **Unasked behavior or scope creep:** None found. Binding/shutdown hooks and the NAT-PMP release correction support the requested mapping lifecycle. PCP/UPnP, other platforms, hardware routers and TCP punching remain explicitly unclaimed.
- **Apparently implemented but wrong behavior:** None found. Candidate updates follow the existing lease watcher; recorded failure, replacement and restart cases separately check established communication and fresh dial outcomes. Completed release retains the allocated protocol in `vendor/portmapper/src/nat_pmp.rs`; priority evidence preserves the independent same-port UDP mapping and foreign TCP/UDP rules. Advanced-builder cases verify saved mapping and global TCP disables.

Reviewed the pinned diff, relevant implementation/test/lab code and accepted evidence indexed in `docs/evidence/mio41-tcp-portmap/README.md`. This was read-only inspection; no tests were rerun. Evidence supports controlled Linux IPv4 NAT-PMP, with the documented broader capability limits.
