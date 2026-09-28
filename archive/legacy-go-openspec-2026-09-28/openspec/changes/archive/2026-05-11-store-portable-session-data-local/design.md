## Context

Session bundles already resolve the sibling daemon and write logs under the extracted bundle directory. The daemon state path still defaults to the platform user config directory, so a copied or re-extracted bundle can silently reuse old identity, network, invite, peer, bootstrap, and report data.

## Goals / Non-Goals

**Goals:**

- Make the current portable/session bundle self-contained for runtime data and logs.
- Use `data/state.json` under the extracted bundle as the default state path for session mode.
- Apply the same state path whether the daemon is started by `miopunch-desktop` or manually with `miopunch up --session`.
- Keep explicit `--state_path` overrides working for labs.
- Make smoke docs name both `logs/` and `data/`.

**Non-Goals:**

- Do not migrate existing user config state into bundle-local data.
- Do not change non-session `miopunch up` defaults.
- Do not remove the system service / privileged packaging scaffolding.

## Decisions

- Use `<bundle_dir>/data/state.json` as the canonical portable state path. This keeps all state-derived files under `<bundle_dir>/data/` because `pocstate.StateDir` derives sibling files from the state file directory.
- Resolve the portable data directory from the running executable path in `internal/bundlepath`. Both `miopunch-desktop` and `miopunch` can use the same helper without duplicating path rules.
- Have `miopunch up --session` fill `StatePath` from the bundle helper only when `--state_path` is not provided. This preserves lab overrides and existing explicit debugging workflows.
- Have desktop-managed daemon startup pass an explicit `--state_path <bundle_dir>/data/state.json`. This makes the managed command unambiguous and keeps behavior stable even if future CLI defaults change.
- Include empty `data/` and `logs/` directories in generated session bundles so testers can see where artifacts will appear before first launch.

## Risks / Trade-offs

- [Risk] Existing user-config state will not appear when testers launch the new portable bundle. -> Mitigation: this is intentional for isolated smoke runs; explicit `--state_path` remains available for debugging old state.
- [Risk] A bundle extracted into a read-only directory cannot write state. -> Mitigation: smoke instructions already require a writable extracted directory; startup diagnostics and logs identify path failures.
- [Risk] Two copies launched from the same extracted directory share identity and state. -> Mitigation: this matches the portable directory model; testers should duplicate/extract into separate directories for independent nodes.
