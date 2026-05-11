## ADDED Requirements

### Requirement: First-run desktop exposes network setup entry points
When the desktop GUI is connected to a blank uninitialized node, it SHALL expose
both new-network and existing-network setup paths.

A blank uninitialized node is one whose topology has no net ID, no governance
head, no decls head, no members, and a missing or `unknown` self role.

The GUI MAY treat that blank first-run node as an owner candidate for UI
visibility only. This SHALL NOT require daemon startup to create network,
governance, or declaration state.
This SHALL NOT imply that runtime broker state such as `brokers_effective`
already exists before the user starts `invite/create` or completes `join`.

#### Scenario: Blank node can create or join from Access
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Access shows Join network
- **AND** Access shows Create invite
- **AND** Access shows Approve request

#### Scenario: Blank node can open Admin before network creation
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Admin navigation is available
- **AND** the local self row is displayed as an owner candidate

#### Scenario: Joined member remains restricted
- **WHEN** the desktop GUI loads topology for a node whose self role is `member`
- **THEN** admin-only navigation and Access flows remain hidden
