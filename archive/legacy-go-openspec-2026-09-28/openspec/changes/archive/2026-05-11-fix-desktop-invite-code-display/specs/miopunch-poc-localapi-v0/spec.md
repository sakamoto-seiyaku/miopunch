## MODIFIED Requirements

### Requirement: SSE streams are snapshot-first and use a single JSON event shape
For both global and per-task SSE streams, the server SHALL:
- Send a `snapshot` event as the first event after the connection is established
- Use a single JSON event body with a `kind` field to distinguish event kinds
- Not require or support `Last-Event-ID` replay; reconnect MUST start from a new `snapshot`
- Treat task update events as coalesced state notifications, not as a lossless replay log
- Include the current task snapshot on task update events when a task is available

Clients SHALL NOT rely on receiving every intermediate task `stage`, `fact`, or
`diagnosis` event. Clients that need reliable task output SHALL merge the
`task` snapshot from update events or refetch the task after observing
completion.

#### Scenario: SSE connection receives snapshot first
- **WHEN** a client connects to `GET /api/v0/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`

#### Scenario: Coalesced task events still carry final task output
- **WHEN** a task emits several rapid update events and the stream coalesces them
- **THEN** the latest task update event includes the current task snapshot
- **AND** a client can recover final task facts and suggestions from that snapshot
