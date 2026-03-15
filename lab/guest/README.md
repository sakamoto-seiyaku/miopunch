# Guest Runtime (inside the VM)

This directory is copied into the single lab VM and provides the guest-side runtime:
- create/cleanup the `netns/veth` topology
- apply NAT profiles on `natA/natB`
- switch between cases (only one active at a time)

## Quick start

From the host (WSL2):

1) Start the VM:

```bash
./lab/host/labctl download
./lab/host/labctl init
./lab/host/labctl up
./lab/host/labctl wait
```

2) Push this guest runtime into the VM:

```bash
./lab/host/labctl push-guest
```

3) SSH into the VM and run the lab tool:

```bash
./lab/host/labctl ssh
sudo /opt/miopunch-lab/guest/bin/mlab --help
```

## Artifacts

`mlab run <case-id>` writes a per-run directory under:
- `/opt/miopunch-lab/artifacts/`

Each run directory includes at least:
- `validate.log`
- `wan.pcap`
- `natA.iptables`, `natB.iptables`
- `natA.conntrack`, `natB.conntrack`
- `natA.tc`, `natB.tc`

