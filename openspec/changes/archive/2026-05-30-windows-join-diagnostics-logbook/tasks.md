## 1. Runtime Diagnostics

- [x] 1.1 Add helper logic in the v1 runtime enroll actions to append broker/topic/invite facts to `invite`, `approve`, and `join` success/failure surfaces without leaking secrets.
- [x] 1.2 Ensure `join` failure paths distinguish signaling open, publish, and reply wait with consistent `network_id` / `invite_id` / `broker_endpoint` / topic facts.

## 2. Desktop / Diagnostics

- [x] 2.1 Verify the desktop bridge continues to surface runtime failure facts into GUI-visible errors and diagnostics export.
- [x] 2.2 Add focused tests for runtime and desktop diagnostics coverage.

## 3. Investigation Notes

- [x] 3.1 Add a new `docs/notes` Windows join investigation logbook with current symptoms, log evidence, code-reading conclusions, this patch batch, and next checks.
