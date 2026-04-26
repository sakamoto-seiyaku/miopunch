# NAT Lab Testbed (P0)

This directory contains the implementation scaffolding for the `nat-lab-testbed` capability.

Key documents:
- Charter: `docs/decisions/p0-nat-lab-charter.md`
- OpenSpec change: `openspec/changes/add-nat-lab-testbed/`

High-level structure:
- `lab/host/`: host-side helpers to manage the single QEMU VM (download/run/ssh/snapshots).
- `lab/guest/`: guest-side lab runtime that lives inside the VM (case definitions + switch/validate tooling).
- `lab/schema/`: case metadata schemas.

Runtime state is intentionally ignored from git:
- `lab/_images/`: downloaded cloud images
- `lab/_state/`: generated seed images, pidfiles, SSH config
- `lab/_artifacts/`: captures, logs, exported state for runs

## Quick start (WSL2 host)

Prereqs on the host:
- `qemu-system-x86_64`, `qemu-img`
- `cloud-localds` (cloud-init seed generator)
- `ssh`, `ssh-keygen`, `rsync`, `curl`
- `go` (only required for `labctl push-bin` / `labctl xtcp-selftest`)

Install (Debian/Ubuntu):

```bash
sudo apt-get update
sudo apt-get install -y qemu-system-x86 qemu-utils cloud-image-utils openssh-client rsync curl
```

Bring up the single VM:

```bash
./lab/host/labctl download
./lab/host/labctl init
./lab/host/labctl up
./lab/host/labctl wait
```

Or run everything end-to-end:

```bash
./lab/host/labctl selftest
```

Example selftest report (captured runs + artifacts pointers):
- `docs/reports/2026-03-17-selftest.md`

Run P1 `xtcp-kernel` integration regression (builds `cmd/miopunch-lab` on host, pushes into VM, runs guest matrix, pulls artifacts):

```bash
./lab/host/labctl xtcp-selftest
```

Run POC e2e closure selftest (builds `cmd/miopunch`, `cmd/miopunch-lab`, and `tools/miopunch-poc-e2e` on host, pushes into VM, runs Docker+systemd multi-node harness inside the VM, pulls artifacts):

```bash
./lab/host/labctl poc-e2e-selftest
```

Run POC e2e full diagnostic suite (slower; includes packet capture artifacts):

```bash
./lab/host/labctl poc-e2e-fulltest
```

Run MNT-01 mainline connectivity gates (real `miopunch up` daemons, self-hosted MQTT signaling, no `coord` fallback):

```bash
./lab/host/labctl mnt01-smoke
./lab/host/labctl mnt01-selftest
./lab/host/labctl mnt01-fulltest
```

MNT-01 gate layers:
- `mnt01-smoke`: representative MQTT-only signaling, direct paths, punching, TCP hard diagnostics, `auto` priority, portmap helper, STUN unavailable, and transport variant coverage.
- `mnt01-selftest`: UDP 15-class unordered matrix plus representative TCP risk, IPv6 fallback, and loss/netem specialty cases.
- `mnt01-fulltest`: UDP 15-class unordered matrix plus TCP 49-class directed matrix.
- Transport specialty coverage keeps Brutal QUIC as `success-required`; KCP is currently `diag-fail-allowed` with full attempt evidence until `docs/notes/mainline-network-test-findings.md` F-003 is fixed.

MNT-01 fixture scope:
- The fixture may seed only identity, peer config, hello/auth bootstrap, self-hosted MQTT/STUN endpoints, test ports, product connectivity options, and network profile labels.
- Hello/auth bootstrap is limited to governance head snapshot and member approval declaration material needed by the existing mainline hello handshake; it is not invite/join/governance behavior coverage.
- The fixture must not seed NAT results, candidate paths, selected paths, neighbor state, success cache, or payload results.
- Product issues found while running MNT-01 should be recorded in `docs/notes/mainline-network-test-findings.md`, not fixed inside the test change.

Run P1 `xtcp-kernel` against all `core-01..core-10` cases (non-NAT4 cases MUST succeed; NAT4-involved cases are allowed to fail but must emit diagnostics):

```bash
./lab/host/labctl xtcp-fulltest
```

Artifacts (from `xtcp-selftest`) are pulled into `lab/_artifacts/` on the host. Each run dir contains:
- `coord.log`, `client.log`, `visitor.log`: JSON event stream + stderr (stage-level timeline; grep `"kind":"fail"`).
- `wan.pcap`: WAN-side capture for the run.
- `natA/natB` snapshots: `iptables-save`, `conntrack`, `tc qdisc`, `netns` listing.
- `run.env`: parameters + basic timing.

Artifacts (from MNT-01) are pulled into `lab/_artifacts/` on the host. Each case run dir contains:
- `fixture.json` / `fixture.env`: injected setup material, `auth_bootstrap` disclosure, and forbidden preloading proof.
- `daemon-a.log`, `daemon-b.log`, `mqtt.log`, optional `stun.log`: product and fixture service logs.
- `attempt-*.json`, `attempt-*.md`, `attempts.tsv`, `summary.json`: product task output, evidence, bounded repeat summary, outcome classification, and `stop_condition` data.
- `mqtt.pcap`, `wan.pcap`, `natA/natB` snapshots: broker and WAN captures plus NAT state.
- `case.env`: matrix identity, direction, classification, profiles, and artifact parameters.

Each MNT-01 aggregate directory contains `cases.txt` and `summary.json`; the summary includes pass/fail counts plus `required_pass`, `preferred_success`, `allowed_diag_fail`, `required_fail`, and `unexpected_fail`.

Artifacts (from `selftest`) are also pulled into `lab/_artifacts/`. Each run dir contains:
- `validate.log`: observed `RFC 4787` mapping/filtering + `NAT1-4` labels.
- `mlab-map-*`, `mlab-mapped-*`: tcpdump lines used by the validator (per-side, per-try).
- `wan.pcap`, `natA/natB` snapshots, `run.env`, `case.env`.

Event stages (P1): `discovery`, `signaling`, `punching`, `confirm`, `transport`, `supervisor`.

Troubleshooting:

- VM boot / SSH issues: check `lab/_state/qemu.log` and `lab/_state/serial.log`.
- SSH port conflict: set `LAB_SSH_PORT` to a free port (default `2222`).
- No `/dev/kvm` (or permission denied): QEMU will fall back to TCG (slow) — still OK for correctness tests.

Push guest runtime and enter the VM:

```bash
./lab/host/labctl push-guest
./lab/host/labctl ssh
```

Inside the VM (guest runtime):

```bash
sudo /opt/miopunch-lab/guest/bin/mlab case list
sudo /opt/miopunch-lab/guest/bin/mlab case activate core-01
sudo /opt/miopunch-lab/guest/bin/mlab validate case
sudo /opt/miopunch-lab/guest/bin/mlab case deactivate
```

Run a full self-test (runs all core cases and stores artifacts):

```bash
sudo /opt/miopunch-lab/guest/bin/mlab-selftest
```

Pull artifacts back to the host:

```bash
./lab/host/labctl pull-artifacts
```

## Snapshots

Snapshots are stored as qcow2 internal snapshots in `lab/_state/disk.qcow2`.
The VM must be stopped before snapshot operations:

```bash
./lab/host/labctl down
./lab/host/labctl snapshot-create base-ready
./lab/host/labctl snapshot-create lab-ready
./lab/host/labctl snapshot-list
./lab/host/labctl snapshot-restore base-ready
```
