## Context

The backend topology model intentionally reports `unknown` before a node has a
network. This is accurate state, but it produces a poor desktop first-run
experience because admin-gated actions are hidden before the user can create the
first network.

## Goals / Non-Goals

**Goals:**

- Make a blank first-run desktop node able to create a network from the UI.
- Keep joining an existing network equally visible.
- Avoid writing network/governance state at daemon startup.
- Avoid implying that broker runtime state already exists before the user starts
  `invite/create` or `join`.
- Preserve member role gating after a successful join.

**Non-Goals:**

- Do not implement member-to-admin promotion.
- Do not change LocalAPI topology schema or backend persisted role semantics.
- Do not auto-create a network when the daemon or desktop starts.

## Decisions

- Define a UI-only first-run condition: role is missing or `unknown`, no net ID,
  no governance head, no decls head, and no members.
- Use `owner` as the effective UI role only for that first-run condition.
- Use the effective UI role for navigation, Access flow visibility, and the
  self row shown in Admin.
- Keep broker state lazy: this UI-only owner candidate does not imply that
  `brokers_effective` or other network state already exists.
- Keep the raw topology role everywhere else, especially after the node has
  joined a network as `member`.

## Risks / Trade-offs

- [Risk] Users may interpret first-run "owner" as already initialized state.
  -> Mitigation: this is only a UI permission view; the first durable state is
  still created by invite or join.
- [Risk] Admin promotion remains unavailable.
  -> Mitigation: document it as out of scope for this change and handle it in a
  separate governance-update change.
