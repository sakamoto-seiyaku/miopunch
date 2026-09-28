# Cases

Each case is a small bash file named `<case-id>.sh` that sets:
- `CASE_ID`
- `A_PROFILE`, `B_PROFILE` (`nat1|nat2|nat3|nat4-regular|nat4-irregular`)
- optional `A_P2P_PORT`, `B_P2P_PORT`
- optional `ENABLE_IPV6=1`
- optional `BLOCK_FORWARD_UDP6=1` to keep IPv6 candidates but force IPv6 direct to fail
- optional `WAN_NETEM="..."` to apply symmetric `tc netem` on both WAN directions
- optional `P2P_UDP_LOSS_PROBABILITY=0.10` to drop only peer-to-peer UDP traffic without touching STUN/control flows

The guest runtime only allows one active case at a time.

Strict regression expectations are stored next to the cases under `lab/guest/cases/expect/`.
