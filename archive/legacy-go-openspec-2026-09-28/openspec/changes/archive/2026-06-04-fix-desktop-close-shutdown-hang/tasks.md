## 1. OpenSpec

- [x] 1.1 Validate `fix-desktop-close-shutdown-hang` with OpenSpec strict validation.

## 2. Desktop Lifecycle

- [x] 2.1 Track and close the active runtime event stream body in `stopEventsPump`.
- [x] 2.2 Bound event pump shutdown waiting and log if it does not stop in time.
- [x] 2.3 Keep Linux close as direct quit without blocking the close callback.
- [x] 2.4 Preserve existing daemon ownership semantics.

## 3. Tests

- [x] 3.1 Add focused tests proving `stopEventsPump` closes a blocking event stream.
- [x] 3.2 Add shutdown coverage for event pump cleanup before managed daemon stop.
- [x] 3.3 Update Linux close tests for async full quit.
- [x] 3.4 Run focused desktop and desktopbridge tests plus Windows compile coverage.
