## ADDED Requirements

### Requirement: Host labctl exposes POC e2e commands
The lab host runtime SHALL provide `poc-e2e-selftest` and `poc-e2e-fulltest` commands.

Each command SHALL reuse the existing QEMU VM lifecycle: start the VM, wait for SSH and cloud-init, push guest runtime, push miopunch binaries, execute the corresponding guest runner, and pull artifacts into `lab/_artifacts/`.

#### Scenario: Host selftest command runs through the VM
- **WHEN** a developer runs `./lab/host/labctl poc-e2e-selftest`
- **THEN** labctl prepares the QEMU VM using the existing host flow
- **AND** it executes the guest POC e2e selftest runner
- **AND** it pulls run artifacts back to `lab/_artifacts/`

#### Scenario: Host fulltest command runs through the VM
- **WHEN** a developer runs `./lab/host/labctl poc-e2e-fulltest`
- **THEN** labctl prepares the QEMU VM using the existing host flow
- **AND** it executes the guest POC e2e fulltest runner
- **AND** it pulls run artifacts back to `lab/_artifacts/`

### Requirement: Guest harness creates an isolated Docker topology
The guest POC e2e harness SHALL create a run-scoped Docker network and SHALL start one broker container plus multiple node containers.

The topology SHALL include `broker`, `node-a`, `node-b`, and `node-c` when a case needs a second joiner, outsider, or wrong approver.

#### Scenario: Topology is isolated per run
- **WHEN** a POC e2e run starts
- **THEN** the harness creates containers and networks with a run-scoped identifier
- **AND** cleanup removes those containers and networks at the end of the run
- **AND** cleanup evidence is written to artifacts

### Requirement: Broker is a real MQTT server
The POC e2e harness SHALL use a real MQTT server as the default broker.

The first implementation SHALL use `mosquitto`, reachable from node containers as `broker:1883`.

#### Scenario: Invite uses the lab broker
- **WHEN** `node-a` creates an invite
- **THEN** its local state pins `local.mqtt_broker` to `broker:1883`
- **AND** the generated invite uses the broker reachable by `node-a`, `node-b`, and `node-c`

### Requirement: Node containers are full system daemon instances
Each node container SHALL run Linux with systemd as PID 1 and SHALL install miopunch through `miopunch install-system-daemon`.

Each node SHALL expose the system LocalAPI socket at `/run/miopunch/localapi.sock` and persist state under `/var/lib/miopunch`.

#### Scenario: Node daemon readiness is verified
- **WHEN** a node container is prepared
- **THEN** `systemctl is-active miopunch` succeeds
- **AND** a LocalAPI readiness probe succeeds through the system socket
- **AND** the harness verifies that user-mode daemon state is not used

### Requirement: Selftest covers the mandatory product closure
`poc-e2e-selftest` SHALL verify the fast POC closure path: daemon install/start, LocalAPI readiness, broker state pinning, `invite`, `approve`, `join`, `ping`, `sh ls`, LocalAPI WebSocket `sh_attach`, and revoke-after-deny.

The selftest SHALL use product CLI commands for normal product operations and LocalAPI WebSocket only for automated `sh_attach` bytes.

#### Scenario: Member joins and reaches issuer
- **WHEN** `node-a` creates an invite, `node-a` runs approve, and `node-b` runs join
- **THEN** `node-b` receives a membership bundle
- **AND** identity, net, governance head, decls, and seed peer state are persisted
- **AND** `node-b ping <node-a-peer-id>` succeeds

#### Scenario: Member uses shell capabilities
- **WHEN** `node-a` has a tmux session and `node-b` is an approved member
- **THEN** `node-b sh ls <node-a-peer-id> local` lists the expected session
- **AND** automated LocalAPI WebSocket `sh_attach` sends marker bytes and observes the expected tmux output

#### Scenario: Revoked member is denied
- **WHEN** `node-a` revokes `node-b`
- **THEN** subsequent `node-b ping` or shell access to `node-a` fails
- **AND** the failure report attributes the denial to authorization, revoke, or hello validation rather than an unrelated transport error

### Requirement: Fulltest covers negative and diagnostic scenarios
`poc-e2e-fulltest` SHALL cover slower negative, persistence, diagnostic, and network evidence scenarios.

Fulltest SHALL include missing approve timeout, wrong approver rejection, invite max-uses, invite expiry, daemon restart persistence, broker outage diagnostics, second member unaffected by revoke, single-writer shell lock, non-default data protocol, redaction, cleanup evidence, and broker non-relay proof.

#### Scenario: Negative control-plane cases are explicit
- **WHEN** invite approval is missing, performed by the wrong node, over-used, or expired
- **THEN** join or approve fails deterministically
- **AND** reports identify the control-plane reason instead of a generic network failure

#### Scenario: Restart preserves membership
- **WHEN** approved nodes restart their miopunch system daemons
- **THEN** identity, net, governance, decls, peer state, ping, and shell listing continue to work

#### Scenario: Broker is not used as data-plane relay
- **WHEN** fulltest runs shell attach with recognizable marker payload
- **THEN** packet captures and logs provide evidence that broker traffic is control-plane/signaling only
- **AND** shell payload is observed on the node/data-plane path rather than broker relay

### Requirement: LocalAPI WebSocket shell automation exists in repo tooling
The repo toolchain SHALL provide a helper for POC e2e `sh_attach` automation.

The helper SHALL create a `sh_attach` task over LocalAPI, connect to `/api/v0/tasks/{task_id}/ws` with subprotocol `miopunch.sh.v0`, send marker bytes, and return machine-readable success/failure output.

#### Scenario: Helper drives shell attach without a pseudo-terminal
- **WHEN** the selftest needs to validate interactive shell bytes
- **THEN** it invokes the helper (built from `tools/miopunch-poc-e2e`) inside `node-b`
- **AND** the helper uses `/run/miopunch/localapi.sock`
- **AND** the helper fails if WebSocket setup, marker send, marker observe, or task completion fails

### Requirement: Artifacts are sufficient for post-run diagnosis
Every POC e2e run SHALL collect artifacts for successful and failed cases.

Artifacts SHALL include command stdout/stderr/exit codes, report exports, daemon journals, state snapshots, broker logs, Docker inspect output, network inspect output, run metadata, and cleanup logs. Packet captures SHALL be mandatory only for fulltest.

#### Scenario: Failed run is debuggable
- **WHEN** a POC e2e case fails
- **THEN** the run directory contains enough CLI, daemon, state, broker, Docker, and network evidence to diagnose the failure without immediately rerunning the case

### Requirement: Implementation remains scoped to Linux POC e2e
The POC e2e container lab SHALL be Linux-only and Docker-only for the first implementation.

It SHALL NOT add Windows node coverage, Podman support, host Docker mode, NAT-type matrices, or public broker dependencies.

#### Scenario: Unsupported scope is not introduced
- **WHEN** the change is reviewed
- **THEN** no new Windows e2e path, Podman backend, host Docker default, NAT1/NAT2/NAT3/NAT4 matrix, or public broker requirement is introduced
