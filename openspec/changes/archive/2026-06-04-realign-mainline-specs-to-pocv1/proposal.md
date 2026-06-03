## Why

The repository mainline is now the current POC v1 demo/product path, but active OpenSpec specs still include older P0/P1/P2/MNT/XTCP/TCP Door-2 and POC v0 requirements as if they were current gates.

This creates the same failure mode that caused recent wasted debugging: agents and reviewers can follow valid-but-stale specs instead of the actual POC v1 runtime, UDP punching, GUI, and Android demo constraints.

## What Changes

- Move historical P0/P1/P2/MNT/XTCP/TCP Door-2 and POC v0 specs out of active `openspec/specs` and into repository archive.
- Add a current-mainline capability that states POC v1 is the active product/demo source of truth.
- Sync completed POC v1 demo changes into main specs, including Android control-lite and the POC v1 desktop console.
- Update current validation specs so host checks plus real Android/Linux/GUI evidence are the active gate; VM lab gates remain historical/debug until a future POC v1 lab change redefines them.
- Update OpenSpec project context, roadmap, and lab README so they no longer present old MNT/XTCP gates as the active mainline.

## Capabilities

### New Capabilities

- `miopunch-poc-v1-current-mainline`: Current active POC v1 mainline, validation scope, and historical-spec boundary.

### Modified Capabilities

- `miopunch-public-reachability`: Remove stale cn/global STUN arbitration as a current POC v1 requirement and keep only current P2P policy/DNS/STUN behavior.
- `miopunch-release-automation-v0`: Replace legacy/core VM lab release gates with current POC v1 host checks, artifact builds, and real-demo evidence.

## Impact

- Affected areas: OpenSpec specs, OpenSpec change archives, `openspec/project.md`, `docs/roadmap.md`, `lab/README.md`, and `AGENTS.md` validation guidance.
- No Go code, runtime behavior, protocol wire format, CLI syntax, or release scripts are changed by this change.
- VM lab assets and historical specs remain available for future redesign and debugging, but they are no longer active current-mainline requirements.
