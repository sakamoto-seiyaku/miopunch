## Context

The runtime already resolves concrete shell targets on Windows peers and concrete tmux sessions when a target is selected. The issue is not discovery, but that the CLI success envelope only shows the count, which blocks scripted consumers from choosing a target.

## Decisions

1. **Keep the change inside `sh ls`.**
   The existing shell protocol and attach flow are already correct. This change only makes the already-enumerated values visible in the CLI success path.

2. **Surface the concrete names as facts and report content.**
   The success result should carry one fact per target or session in addition to the existing count fact. The markdown report should mirror the same facts.

3. **Keep human output unchanged.**
   The line-oriented output remains the existing target/session list; no new formatting rules are introduced.

## Risks / Trade-offs

- **Risk:** Additional facts slightly enlarge JSON/report output.
  **Mitigation:** Limit the extra facts to `sh ls` only.

- **Risk:** The output becomes more verbose for large target lists.
  **Mitigation:** That verbosity is intentional because it is the information the operator needs to choose a target.
