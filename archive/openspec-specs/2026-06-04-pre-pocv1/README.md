# Archived OpenSpec Specs: pre-POC v1 active set

This directory preserves specs that used to live under `openspec/specs/` before
the active mainline was realigned to current POC v1 on 2026-06-04.

These specs are historical or deferred references. They are not current
mainline gates unless a future OpenSpec change explicitly restores or rewrites
them for POC v1.

## Archived groups

- XTCP and old connectivity kernel:
  - `xtcp-kernel`
  - `xtcp-connectivity`
  - `miopunch-tcp-p2p-v0`
  - `miopunch-wire-tcp-info-v0`
- Mainline network test gates:
  - `miopunch-mainline-connectivity-matrix-v0`
  - `miopunch-mainline-control-plane-e2e-v0`
  - `miopunch-mainline-nat-composite-network-v0`
- POC v0 product/control-plane specs:
  - `miopunch-poc-*`
- Lab and older experimental support specs:
  - `nat-lab-testbed`
  - `miopunch-lab-*`
  - `miopunch-dataplane`
  - `miopunch-mqtt-signaling`
  - `miopunch-stun-probe-v0`
  - `miopunch-coordinator-errors`

## Current replacement

The active current-mainline boundary is now described by:

- `openspec/specs/miopunch-poc-v1-current-mainline/spec.md`
- `openspec/specs/miopunch-poc-v1-*`
- `openspec/specs/miopunch-punching-decision/spec.md`
- `openspec/specs/miopunch-udp-socket-owner-demux/spec.md`
- `openspec/specs/miopunch-public-reachability/spec.md`
- current desktop, Android, release, packaging, code health, layout, and smoke specs
