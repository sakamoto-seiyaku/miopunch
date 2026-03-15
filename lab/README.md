# NAT Lab Testbed (P0)

This directory contains the implementation scaffolding for the `nat-lab-testbed` capability.

Key documents:
- Charter: `docs/p0-nat-lab-charter.md`
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
