## Done

- Freeze `dial_offer/dial_answer` with the v1 minimal field set, including `member_credential` handoff to `poc-v1-05-secure-session`.
- Freeze `PathResult` as the output of this change: selected UDP path, resource ownership, and punch evidence only.
- Freeze the 5B attempt matrix: max concurrency 4, total budget 10s, no ICE/trickle/overlay fallback.
