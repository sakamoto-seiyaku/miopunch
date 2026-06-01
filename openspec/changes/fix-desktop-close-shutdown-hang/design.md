## Context

The desktop GUI starts a runtime event pump after connecting to LocalAPI. The
pump reads a newline-delimited event stream until the stream closes or its
context is cancelled. `shutdown()` currently calls `stopEventsPump()`, which
cancels the context and then waits on `eventsDone`.

If the goroutine is blocked in the stream reader, cancellation alone may not
wake it because the active stream body is owned by the reader. That makes
shutdown wait indefinitely before it reaches terminal bridge cleanup and
managed daemon stop.

## Decisions

### Close the active stream body on stop

The app will track the active event stream body while a pump is reading it.
`stopEventsPump()` will cancel the pump context, close the active body, and
then wait for the pump goroutine.

The body handle is scoped to the current pump and protected by `App.mu`. The
pump clears the handle only when it still owns the registered stream, so stale
pumps cannot remove a newer stream handle.

### Bound event pump shutdown wait

`stopEventsPump()` will wait briefly for `eventsDone`. If the goroutine still
does not finish, it logs a warning and allows shutdown to continue. This keeps
window close from becoming unrecoverable while still giving the normal cleanup
path a chance to complete.

### Linux close remains direct quit

Linux close will continue to mark quit requested and exit the process because
there is no reliable tray affordance in this product path. The close callback
should not block the UI close event while cleanup runs.

### Preserve daemon ownership

Shutdown still uses `App.managedDaemon`. If the GUI reused an already-running
daemon, `managedDaemon` remains nil and shutdown does not stop that external
daemon.

## Risks

- Closing the stream body can race with the pump's normal close path. The
  stream close operation must be best-effort and idempotent from the caller's
  perspective.
- A bounded wait can leave a stuck pump goroutine behind in a mocked or broken
  environment, but full process exit should still proceed.
