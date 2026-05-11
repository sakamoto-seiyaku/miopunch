## 1. Runtime Logs

- [x] 1.1 Add a shared helper for resolving a local session bundle `logs/` path.
- [x] 1.2 Initialize `miopunch-desktop` logging to `logs/miopunch-desktop.log`.
- [x] 1.3 Initialize `miopunch up` session logging to `logs/miopunch.log` on Linux and Windows.
- [x] 1.4 Convert Linux GTK startup panics into actionable diagnostics in stderr and `logs/miopunch-desktop.log`.
- [x] 1.5 Include final task snapshots in task events and merge them in the desktop UI so Create Invite surfaces the real `invite_code`.

## 2. Smoke Instructions

- [x] 2.1 Expand generated session bundle `SMOKE.md` with log paths and ordered Windows/Linux manual smoke steps.
- [x] 2.2 Update desktop smoke notes so source docs match the bundled instructions.

## 3. Validation And Artifacts

- [x] 3.1 Run focused Go tests for runtime path, desktop startup diagnostics, and session daemon startup behavior.
- [x] 3.2 Validate OpenSpec strict mode for the change.
- [x] 3.3 Rebuild current Linux and Windows session bundles and extract both bundles for local manual launch/copy.
- [x] 3.4 Add focused backend/frontend regression tests for invite-code task snapshot delivery.
