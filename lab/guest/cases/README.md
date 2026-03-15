# Cases

Each case is a small bash file named `<case-id>.sh` that sets:
- `CASE_ID`
- `A_PROFILE`, `B_PROFILE` (`nat1|nat2|nat3|nat4-regular|nat4-irregular`)
- optional `A_P2P_PORT`, `B_P2P_PORT`

The guest runtime only allows one active case at a time.

