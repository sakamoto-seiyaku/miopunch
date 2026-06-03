# miopunch-poc-localapi-v0 Specification

## Purpose
TBD - created by archiving change poc-05-daemon-up-localapi. Update Purpose after archive.
## Requirements
### Requirement: LocalAPI v0 is IPC-only and must not expose a public TCP listener
LocalAPI v0 SHALL be served only over OS-native IPC transports:
- Linux: unix domain socket
- Windows: named pipe

The system SHALL NOT expose LocalAPI v0 as an externally reachable TCP listener.

#### Scenario: LocalAPI is reachable via IPC without a TCP port
- **WHEN** a daemon is running (`miopunch up`)
- **THEN** a client can call LocalAPI via the platform IPC address
- **AND** LocalAPI does not require binding a TCP port on a network interface

### Requirement: LocalAPI v0 requires a fixed Host header for intent validation
LocalAPI v0 SHALL require `Host: local-miopunch.localapi` on all HTTP requests.
Requests with a different Host value (or missing Host) SHALL be rejected with an actionable error response.

#### Scenario: Request with the wrong Host is rejected
- **WHEN** a client sends a LocalAPI request with `Host` not equal to `local-miopunch.localapi`
- **THEN** the server rejects the request
- **AND** the response includes actionable diagnostics

### Requirement: LocalAPI v0 exposes a frozen minimal route set
LocalAPI v0 SHALL expose the following routes under `/api/v0`:
- `GET /api/v0/status`
- `GET /api/v0/peers`
- `GET /api/v0/topology`
- `GET /api/v0/tasks`
- `GET /api/v0/tasks/<task_id>`
- `POST /api/v0/tasks`
- `GET /api/v0/events` (SSE)
- `GET /api/v0/tasks/<task_id>/events` (SSE)
- `GET /api/v0/tasks/<task_id>/report` (Markdown)
- `GET /api/v0/tasks/<task_id>/ws` (WebSocket, `sh_attach` only)
- `GET /api/v0/desktop/state`
- `GET /api/v0/desktop/events` (SSE)
- `PATCH /api/v0/desktop/config`

The route set remains versioned under `/api/v0`; `desktop/state`,
`desktop/events`, and `desktop/config` are the product-facing desktop runtime
contract for the desktop shell, while task routes remain available for
compatibility, debug, and report flows.

#### Scenario: A client can create and observe a task
- **WHEN** a client calls `POST /api/v0/tasks` to create a task
- **THEN** the server returns a `task_id`
- **AND** the client can observe progress via `GET /api/v0/tasks/<task_id>` and task SSE

#### Scenario: A client can query mainline topology diagnostics
- **WHEN** a client calls `GET /api/v0/topology`
- **THEN** the server returns a machine-readable topology snapshot
- **AND** the snapshot includes bootstrap, reachability, active neighbor, degree, latest attempt, and recovery evidence when available

#### Scenario: A desktop client can bootstrap product runtime state in one call
- **WHEN** a desktop client calls `GET /api/v0/desktop/state`
- **THEN** the server returns one product-facing desktop runtime snapshot
- **AND** the snapshot includes a monotonic `rev`
- **AND** the client does not need to merge separate `status`, `peers`, `topology`, and `tasks` routes to establish initial runtime state

#### Scenario: A desktop client saves runtime config
- **WHEN** a desktop client calls `PATCH /api/v0/desktop/config` with valid supported fields
- **THEN** the daemon persists the desired config
- **AND** the response returns a fresh desktop runtime snapshot
- **AND** desktop events publish updated config and diagnostics state

### Requirement: SSE streams are snapshot-first and use a single JSON event shape
For global, per-task, and desktop SSE streams, the server SHALL:
- Send a `snapshot` event as the first event after the connection is established
- Use a single JSON event body with a `kind` field to distinguish event kinds
- Not require or support `Last-Event-ID` replay; reconnect MUST start from a new `snapshot`

For task SSE streams:
- Treat task update events as coalesced state notifications, not as a lossless replay log
- Include the current task snapshot on task update events when a task is available

For desktop SSE streams:
- Carry `base_rev` and `rev` on every non-snapshot event
- Use typed product-state updates rather than refetch hints
- Allow clients to detect revision gaps and fall back to `GET /api/v0/desktop/state`

Clients SHALL NOT rely on receiving every intermediate task `stage`, `fact`, or
`diagnosis` event. Clients that need reliable task output SHALL merge the
`task` snapshot from update events or refetch the task after observing
completion.

Desktop clients SHALL treat `desktop/events` as the primary live runtime feed
for product UI state and SHALL use `desktop/state` for bootstrap and resync.

#### Scenario: SSE connection receives snapshot first
- **WHEN** a client connects to `GET /api/v0/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`

#### Scenario: Coalesced task events still carry final task output
- **WHEN** a task emits several rapid update events and the stream coalesces them
- **THEN** the latest task update event includes the current task snapshot
- **AND** a client can recover final task facts and suggestions from that snapshot

#### Scenario: Desktop event stream carries ordered runtime state updates
- **WHEN** a desktop client connects to `GET /api/v0/desktop/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`
- **AND** later events carry `base_rev` and `rev`
- **AND** each later event updates one typed runtime subtree without requiring a client-side refetch

### Requirement: Desktop runtime state is product-facing and revisioned
`GET /api/v0/desktop/state` SHALL return a product-facing desktop runtime
snapshot that is safe for the desktop shell to treat as authoritative runtime
state.

The snapshot SHALL include:
- `rev`
- `status`
- `topology`
- `tasks`
- `peer_sessions`
- `shell_sessions`
- `config`
- `diagnostics`
- `approval_requests`

The `config` subtree SHALL include desired/effective runtime config and
desktop-only preferences. It MUST NOT include secret material such as peer
secret keys, invite secrets, net secrets, private keys, raw join codes, or raw
membership bundles.

The snapshot SHALL NOT require the desktop client to parse task facts as the
primary way to derive peer sessions, config state, or diagnostics.

#### Scenario: Desktop snapshot is self-contained
- **WHEN** a desktop client requests `GET /api/v0/desktop/state`
- **THEN** the response includes the product-facing runtime subtrees needed by the desktop console
- **AND** each subtree is typed JSON instead of a stringly-typed diagnostic blob

#### Scenario: Desktop snapshot reserves workflow state for follow-up changes
- **WHEN** a desktop client requests `GET /api/v0/desktop/state` before richer approval workflow support exists
- **THEN** the response still includes `approval_requests`
- **AND** the field is present as a typed list even when it is empty

#### Scenario: Desktop config is redacted
- **WHEN** a desktop client requests `GET /api/v0/desktop/state`
- **THEN** config state includes supported non-secret settings
- **AND** it omits secret keys, invite secrets, net secrets, private keys, and raw membership material

### Requirement: LocalAPI supports desktop runtime config updates
`PATCH /api/v0/desktop/config` SHALL accept partial updates for supported
desktop Settings fields and SHALL validate values before persisting them.

At minimum, invalid values for `p2p_network`, `p2p_ip_family`, `data_proto`,
`quic_cc`, broker endpoints, STUN endpoints, and log level SHALL return a
structured LocalAPI bad-request error with actionable facts and suggestions.

Valid updates SHALL persist network runtime settings to the daemon state file
and desktop-only preferences to the desktop settings file. Updates SHALL return
a fresh desktop runtime snapshot.

#### Scenario: Invalid desktop config update is rejected
- **WHEN** a desktop client submits an unsupported `p2p_network`
- **THEN** LocalAPI returns a non-2xx response with `reason_code=bad_request`
- **AND** the existing persisted config is unchanged

#### Scenario: Log level update applies immediately
- **WHEN** a desktop client saves `log_level=debug`
- **THEN** the daemon persists the preference
- **AND** new daemon log output uses the saved level without requiring daemon restart

### Requirement: WebSocket is only available for `sh_attach` tasks and requires the `miopunch.sh.v0` subprotocol
`GET /api/v0/tasks/<task_id>/ws` SHALL only be usable when the referenced task has `kind=sh_attach`.
The WebSocket handshake SHALL require `Sec-WebSocket-Protocol: miopunch.sh.v0`.

#### Scenario: WebSocket is rejected for a non-`sh_attach` task
- **WHEN** a client opens `GET /api/v0/tasks/<task_id>/ws` for a task whose kind is not `sh_attach`
- **THEN** the server rejects the request

#### Scenario: WebSocket handshake fails without the required subprotocol
- **WHEN** a client opens `GET /api/v0/tasks/<task_id>/ws` for a `sh_attach` task
- **AND** the client does not negotiate `miopunch.sh.v0`
- **THEN** the handshake fails

### Requirement: LocalAPI topology diagnostics are stable for MNT-03
`GET /api/v0/topology` SHALL return a JSON object that can be used by product CLI, lab gates, and operators without parsing daemon logs.

The response SHALL include these fields when available:
- local peer identity and `net_id`
- governance and decls head summaries
- `v4_hint` and `v6_hint`
- presence and online state
- bootstrap candidate list, selected bucket, attempted candidates, and failure reasons
- active neighbor list and degree distribution
- latest selected `attempt_path`, `data_proto`, payload evidence, and failure stop condition
- recovery evidence for revoke, offline, rejoin, broker outage, IPv6 block, and portmap block

The response MUST NOT include secret material, invite secrets, net secrets, raw private keys, or unredacted join codes.

#### Scenario: Topology snapshot is safe for artifacts
- **WHEN** an MNT-03 gate saves `GET /api/v0/topology`
- **THEN** the snapshot contains enough evidence to validate bootstrap and active neighbor behavior
- **AND** it does not contain unredacted secret material

### Requirement: CLI exposes the same topology evidence as LocalAPI
The product CLI SHALL expose topology diagnostics in JSON form using the same LocalAPI data source as `GET /api/v0/topology`.

The CLI command SHALL be suitable for lab automation and MUST NOT require direct reads from daemon private state files.

#### Scenario: CLI topology output matches LocalAPI evidence
- **WHEN** a lab gate collects CLI topology JSON and LocalAPI topology JSON for the same node
- **THEN** both outputs describe the same bootstrap, reachability, active neighbor, and recovery evidence

### Requirement: LocalAPI exposes typed approval request runtime state
`GET /api/v0/desktop/state` SHALL include approval requests as typed JSON objects suitable for desktop review workflows.

Each approval request object SHALL include at minimum: `approve_task_id`, `invite_id`, `request_msg_id`, `member_peer_id`, `status`, `created_at`, and `updated_at` when available.

Approval request objects MUST NOT include invite secrets, net secrets, private keys, decrypted membership bundles, or raw encrypted payloads.

Approval request objects MUST NOT include private restart decision material such as invite brokers, reply topics, validated join request bodies, or member public key payloads.

Desktop SSE SHALL publish `approval_requests.replace` when approval request state changes.

#### Scenario: Pending approval appears in desktop state
- **WHEN** an explicit-review approve task records a pending join request
- **THEN** `GET /api/v0/desktop/state` includes that request in `approval_requests`
- **AND** the request can be addressed by `approve_task_id` and `request_msg_id`

#### Scenario: Approval updates are streamed
- **WHEN** an approval request is created, approved, rejected, or expires
- **THEN** the desktop event stream emits an `approval_requests.replace` update
- **AND** the update does not expose secret material

### Requirement: LocalAPI supports approval decisions through task creation
`POST /api/v0/tasks` SHALL support task kind `approve_decision`.

The `approve_decision` task args SHALL include `approve_task_id`, `request_msg_id`, and `decision`.

The task SHALL fail without side effects when the referenced approve task or request does not exist, when the decision value is invalid, or when the request has already reached a conflicting terminal decision.

The task SHALL support pending requests that were persisted before daemon restart and SHALL NOT require the referenced `approve` task to still be active.

#### Scenario: Desktop client submits an approve decision
- **GIVEN** LocalAPI has a pending approval request
- **WHEN** a client creates an `approve_decision` task with `decision="approve"`
- **THEN** the task applies the approval decision to the referenced request
- **AND** task state reports the final decision result

#### Scenario: Desktop client submits a decision after daemon restart
- **GIVEN** LocalAPI has a persisted pending approval request from before daemon restart
- **WHEN** a client creates an `approve_decision` task for that request
- **THEN** the task applies the approval decision without an active `approve` runtime
- **AND** desktop state reports the final decision result

#### Scenario: Invalid decision is rejected
- **WHEN** a client creates an `approve_decision` task with an invalid decision value
- **THEN** the task fails without changing any approval request
- **AND** diagnostics identify the invalid decision

### Requirement: Desktop config supports local peer aliases
`PATCH /api/v0/desktop/config` SHALL accept desktop-local peer aliases under desktop preferences.

`GET /api/v0/desktop/state` and desktop config events SHALL return the persisted alias map in the config preferences subtree. Aliases SHALL be keyed by peer ID and SHALL NOT be written to governance declarations, member declarations, invite material, or known-peer network config.

#### Scenario: Desktop client saves a peer alias
- **WHEN** a desktop client saves `preferences.peer_aliases` for a peer
- **THEN** LocalAPI persists the alias in desktop settings
- **AND** the returned desktop runtime snapshot includes the alias
- **AND** the alias is not written to governance or declaration state

#### Scenario: Desktop client clears a peer alias
- **WHEN** a desktop client saves an empty alias for a peer
- **THEN** LocalAPI persists the cleared alias state
- **AND** later desktop snapshots do not return a non-empty alias for that peer

### Requirement: Desktop topology exposes member display hints
Desktop topology member objects SHALL include optional remote display hints derived from approved member declarations when available.

At minimum, topology members SHALL expose `member_name` and `platform` when those fields exist in the approved member declaration. These fields SHALL be non-secret display metadata and SHALL NOT replace `peer_id`.

#### Scenario: Approved member name appears in topology
- **WHEN** an approved member declaration contains `member_name` and `platform`
- **THEN** `GET /api/v0/desktop/state` includes those values on the corresponding topology member
- **AND** the member still includes its peer ID

#### Scenario: Missing member display hints are omitted
- **WHEN** an approved member declaration does not contain a member name or platform
- **THEN** desktop topology omits those optional fields
- **AND** clients can still identify the member by peer ID

### Requirement: Desktop shell sessions report local attachability
Desktop shell-session summaries SHALL report whether a local WebSocket attach can currently resume the represented `sh_attach` task.

`attachable=true` SHALL mean the task can accept or resume the foreground LocalAPI WebSocket attach. `attachable=false` SHALL mean clients must not attempt Resume for that task and should create another shell task if the user wants a new foreground terminal.

Desktop SSE SHALL publish `shell_sessions.replace` when attachability changes.

#### Scenario: Waiting shell task is attachable
- **WHEN** a `sh_attach` task is waiting for a local WebSocket attach
- **THEN** desktop runtime state includes a shell-session summary for that task
- **AND** the summary has `attachable=true`

#### Scenario: Completed shell task is not attachable
- **WHEN** a `sh_attach` task completes or its attach window is gone
- **THEN** desktop runtime state does not report it as attachable
- **AND** a desktop shell-session update is streamed when visible state changes

### Requirement: Desktop runtime exposes optional connection path details
Desktop topology and peer-session runtime state SHALL expose connection path details only when the daemon has reliable evidence for the value.

Supported optional fields include direct IPv4/IPv6 hints, local endpoint, remote endpoint, public tuple, punch status, and port. The daemon SHALL omit unknown fields instead of fabricating preview values.

#### Scenario: Reliable path details are returned
- **WHEN** the daemon has reliable endpoint or punch-status evidence for an active peer path
- **THEN** desktop runtime state includes the corresponding optional path detail fields
- **AND** the fields are available through the desktop snapshot and relevant desktop state updates

#### Scenario: Unknown path details are omitted
- **WHEN** the daemon has no reliable endpoint, tuple, punch, or metric evidence for a peer
- **THEN** desktop runtime state omits those optional fields
- **AND** it does not synthesize RTT, throughput, loss, endpoint, or tuple values

### Requirement: LocalAPI supports local network initialization task
`POST /api/v0/tasks` SHALL support task kind `init_network`.

The task args SHALL include `mode`, where supported values are `bootstrap` and
`create_new`. `create_new` SHALL require confirmation value
`create-new-network`.

#### Scenario: Desktop client initializes blank network through a task
- **WHEN** a desktop or CLI client creates `init_network` with `mode=bootstrap`
  on a blank node
- **THEN** LocalAPI creates a task
- **AND** the completed task reports the new local `net_id` and `peer_id`

#### Scenario: Missing confirmation rejects create-new
- **WHEN** a client creates `init_network` with `mode=create_new` without the
  required confirmation
- **THEN** the task fails with `BAD_REQUEST`
- **AND** existing local governance state is preserved

### Requirement: Desktop state exposes local governance capabilities
`GET /api/v0/desktop/state` and desktop runtime events SHALL expose non-secret
local governance capability state under the desktop config/state model.

The state SHALL include the governance classification, self role, whether the
node can initialize owner mode, whether it can create a new network, and whether
invite/approve actions are available.

#### Scenario: Blank node exposes initialization capability
- **WHEN** a desktop client loads state for a blank node
- **THEN** the desktop state reports governance state `no_network`
- **AND** `can_init_owner=true`
- **AND** admin invite/approve capabilities are false until initialization

#### Scenario: Admin node exposes invite approval capability
- **WHEN** a desktop client loads state for an admin network
- **THEN** the desktop state reports governance state `admin_network`
- **AND** invite and approve capabilities are true

#### Scenario: Non-admin node exposes create-new capability
- **WHEN** a desktop client loads state for member, foreign, or stale local
  governance state
- **THEN** invite and approve capabilities are false
- **AND** create-new-network capability is true

### Requirement: Desktop Runtime State Exposes Selected Session Path Facts
`GET /api/v0/desktop/state` SHALL include safe selected session path facts in `peer_sessions` when a live or recent peer session has that evidence.

Supported optional facts include `local_endpoint`, `remote_endpoint`, `punch_status`, and `port`. The daemon MUST omit unknown fields instead of fabricating values from reachability hints or logs.

#### Scenario: Active peer session includes endpoint facts
- **WHEN** a peer session is active and the daemon knows its selected local and remote endpoints
- **THEN** `GET /api/v0/desktop/state` includes those endpoint facts on the matching `peer_sessions` entry
- **AND** the entry still includes `remote_peer_id`, `data_proto`, `path_family`, `healthy`, and `last_activity_unix_ms`

#### Scenario: Unknown path facts remain omitted
- **WHEN** a peer session has no validated endpoint or punch-status evidence
- **THEN** `GET /api/v0/desktop/state` omits the corresponding optional fields

### Requirement: Topology Active Neighbors Mirror Session Path Facts
`GET /api/v0/topology` SHALL expose the same safe selected session path facts for active neighbors that are available in desktop peer sessions.

#### Scenario: Active neighbor includes matching session facts
- **WHEN** an active peer session has selected endpoint facts
- **THEN** the matching `topology.neighbors.active` entry includes those facts
- **AND** the facts match the `peer_sessions` entry for the same peer, protocol, and path family

### Requirement: Topology Attempts Include Selected View Evidence
Topology attempt evidence SHALL include selected STUN/TCP view and reason fields when the decision path produced them.

#### Scenario: TCP punching records selected view
- **WHEN** a TCP punching attempt succeeds and decision output includes `tcp_selected_view` and `tcp_selected_reason`
- **THEN** topology attempt evidence includes the selected view and reason for that attempt
