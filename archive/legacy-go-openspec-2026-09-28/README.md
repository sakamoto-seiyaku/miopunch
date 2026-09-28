# Legacy Go and OpenSpec Baseline

This directory preserves the former Go implementation, Android client,
lab, build tooling, OpenSpec material, and project-local agent instructions
as they stood when miopunch restarted its design work in September 2026.

Nothing here is the current implementation plan. Read these files for
historical behavior and evidence only; new product decisions live in the
repository's current design notes and future ADRs.

The future role of the old tests, lab scripts, and other validation tooling is
undecided. Archiving preserves them; it does not decide whether they will be
reused, adapted, or replaced for the new implementation.

The `frp` Git submodule remains pinned at its original commit under this
directory. The lab's ignored local VM images, state, and artifacts were moved
with the lab directory but remain ignored and are not part of Git history.
