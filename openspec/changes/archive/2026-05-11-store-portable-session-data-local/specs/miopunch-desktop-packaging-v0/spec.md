## ADDED Requirements

### Requirement: Portable session bundles keep runtime data beside the bundle
The current Windows and Linux portable/session bundles SHALL store session runtime data under a `data` directory beside the extracted bundle binaries.

The default portable session state path SHALL be `data/state.json` under the extracted bundle directory. State-derived files such as `net.json`, `identity/`, `decls/`, `bootstrap/`, and task `reports/` SHALL also reside under `data/`.

When `miopunch-desktop` starts the sibling daemon from a session bundle, it SHALL start it with the bundle-local session state path.

When a user manually runs `miopunch up --session` from the extracted bundle and does not provide `--state_path`, the daemon SHALL use the same bundle-local session state path.

An explicit `--state_path` override SHALL continue to take precedence over the portable session default.

#### Scenario: Desktop-managed daemon writes data into the extracted bundle
- **WHEN** a user launches `miopunch-desktop` from an extracted session bundle
- **AND** the GUI starts the sibling daemon
- **THEN** the daemon uses `<bundle>/data/state.json` as its state path
- **AND** identity, network, peer, bootstrap, and report files are written under `<bundle>/data/`

#### Scenario: Manual session daemon writes data into the extracted bundle
- **WHEN** a user runs `miopunch up --session` from an extracted session bundle
- **AND** no `--state_path` override is provided
- **THEN** the daemon uses `<bundle>/data/state.json` as its state path

#### Scenario: Explicit state path override is preserved
- **WHEN** a user runs `miopunch up --session --state_path <custom-state>`
- **THEN** the daemon uses `<custom-state>`
- **AND** it does not replace the custom path with `<bundle>/data/state.json`

#### Scenario: Session bundle smoke docs identify local data
- **WHEN** the session bundle is built
- **THEN** its smoke instructions identify `data/state.json`
- **AND** they state that removing `data/` resets the portable node for a clean smoke run
