## Why

Windows peers already enumerate usable shell targets, but the CLI JSON envelope only exposes a count, which makes automation and operator workflows blind to the actual selectable targets.

## What Changes

- Expose `sh ls` target/session details in the success facts and report output.
- Preserve existing human output and the `miopunch.json.v0` envelope shape.
- Keep the scope limited to `sh ls`; do not change shell protocol, session reuse, or attach behavior.

## Capabilities

### New Capabilities
- `miopunch-poc-v1-shell-ls-targets-output`: `sh ls` returns the concrete target/session names needed for automation and operator selection.

### Modified Capabilities
- `miopunch-poc-v1-headless-runtime`: `sh ls` now surfaces the enumerated target/session names in operator-visible output.
- `miopunch-poc-output-contract-v0`: the existing envelope remains stable while carrying richer success facts for `sh ls`.

## Impact

- Affected code: runtime `sh ls` result construction and focused runtime tests.
- Behavior: `miopunch --format json sh ls <peer>` and `--report` now include the concrete shell targets or sessions.
