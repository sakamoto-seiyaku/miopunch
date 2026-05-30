## Context

`sh ls` currently has two modes:

- `miopunch sh ls <peer>` returns all discoverable targets.
- `miopunch sh ls <peer> <target>` returns tmux sessions for that target.

On Windows-controlled peers, target discovery is intentionally cheap: WSL
targets come from `wsl.exe -l -q`, and SSH targets come from local SSH config
aliases. That keeps the default list fast, but it also means the operator
cannot tell which targets can actually support tmux-backed attach and recovery.

The requested UX is to separate these two concerns:

- raw discovery stays cheap and complete;
- readiness becomes an explicit probe.

This change touches CLI parsing, runtime shell control requests, and the
platform-specific shell target layer, so it benefits from a design document
before implementation.

## Goals / Non-Goals

**Goals:**
- Keep default `sh ls <peer>` as raw discoverable target enumeration.
- Add an explicit `--ready` mode that returns only targets confirmed ready for
  tmux-backed attach.
- Make ready probing bounded and non-interactive so the command cannot hang on
  password prompts or host-key confirmation.
- Preserve partial success: one bad SSH alias must not make the whole ready
  listing fail.
- Surface enough structured status for operators and automation to distinguish
  `ready`, `unsupported`, and `unknown`.

**Non-Goals:**
- Do not change `sh_attach` semantics or tmux recovery semantics.
- Do not redesign target naming or replace tmux with another shell backend.
- Do not make default `sh ls <peer>` perform readiness probing.
- Do not add `-a/--all`; default mode already means "all discoverable targets".

## Decisions

### 1. Readiness is an explicit `sh ls --ready` mode

Decision:
- Extend `miopunch sh ls <peer>` with `--ready`.
- Keep `miopunch sh ls <peer> <target>` unchanged as the session-listing path.
- Reject `--ready` when a concrete target is also supplied.

Rationale:
- This preserves today's fast default behavior.
- It avoids turning a previously cheap command into one that actively contacts
  every SSH alias.

Alternative considered:
- Make readiness the default and add `-a/--all` for raw enumeration.
- Rejected because it would make the common command slower, more failure-prone,
  and potentially interactive on Windows SSH aliases.

### 2. Ready means "tmux attach preflight passes", not "a tmux session exists"

Decision:
- A target is `ready` when a bounded, non-interactive preflight confirms that
  tmux is callable on that target.
- Existing tmux sessions are not required for readiness.

Rationale:
- `sh_attach` uses `tmux new -A -s <session>`, which can create a session when
  none exists.
- Using `tmux list-sessions` as the readiness signal would incorrectly mark a
  usable target as not ready when tmux is installed but no server is running.

Alternative considered:
- Define ready as "target already has at least one tmux session".
- Rejected because it does not match actual attach semantics.

### 3. SSH ready probing uses a dedicated non-interactive probe path

Decision:
- Introduce a readiness-probe path in `internal/shelltarget` instead of
  reusing the current attach/session commands directly.
- Windows `ssh:<host>` ready probing uses SSH with explicit non-interactive
  options and a short timeout, including:
  - `BatchMode=yes`
  - `StrictHostKeyChecking=yes`
  - `ConnectTimeout=<small bounded value>`
  - `NumberOfPasswordPrompts=0`
- The probe command checks tmux availability without side effects.

Rationale:
- `sh ls --ready` must not wait on password prompts.
- `StrictHostKeyChecking=yes` avoids mutating `known_hosts` during a read-only
  readiness check.
- A dedicated probe path lets attach behavior remain unchanged.

Alternative considered:
- Reuse the existing SSH preflight command for readiness.
- Rejected because the current preflight is attach-oriented and may block on
  interactive SSH behavior.

### 4. Ready probing is partial-success tolerant and status-bearing

Decision:
- `sh ls --ready` succeeds if the controlled peer can enumerate targets, even
  if some targets cannot be confirmed.
- Each discovered target gets one status entry:
  - `ready`
  - `unsupported`
  - `unknown`
- `unsupported` is reserved for confirmed tmux absence.
- `unknown` covers timeout, auth failure, host-key refusal, or other bounded
  probe failures.

Rationale:
- Operators need useful results from mixed environments instead of an all-or-
  nothing command.
- `tmux missing` is materially different from "we could not safely prove it".

Alternative considered:
- Fail the whole command on the first unprobeable target.
- Rejected because one stale SSH alias would make the feature unusable.

### 5. Wire/runtime output grows additively

Decision:
- Extend `ShellArgs` and shell control messages with a boolean readiness flag.
- Extend shell-list replies with structured per-target readiness results.
- In `--ready` mode:
  - `targets`/line output contains only the ready subset.
  - `target_statuses` contains one structured record per discovered target.
- Existing non-ready `sh ls` output remains unchanged.

Rationale:
- This preserves existing consumers for the default path.
- The ready subset remains easy for humans, while structured statuses remain
  available for automation and diagnostics.

Alternative considered:
- Replace the existing `targets` list with the full discovered list plus inline
  status text.
- Rejected because it would blur the command's ready-only semantics and make
  automation parsing more fragile.

## Risks / Trade-offs

- [Risk] `sh ls --ready` is slower than raw enumeration on peers with many
  targets. -> Mitigation: keep probing explicit, bounded, and separate from the
  default path.
- [Risk] Strict host-key checking will mark first-seen SSH aliases as
  `unknown`. -> Mitigation: report them as structured unknowns instead of
  silently accepting host keys or mutating local SSH state.
- [Risk] Additional structured result fields may increase `sh ls` JSON/report
  verbosity. -> Mitigation: only emit readiness status details in `--ready`
  mode.

## Migration Plan

- No data migration is required.
- Rollout is additive to the CLI and shell control contract.
- Existing `sh ls <peer>` and `sh ls <peer> <target>` invocations continue to
  behave as before.

## Open Questions

- None. The proposal already fixes the intended UX: default raw enumeration,
  explicit `--ready` probing, partial success, and no `-a/--all`.
