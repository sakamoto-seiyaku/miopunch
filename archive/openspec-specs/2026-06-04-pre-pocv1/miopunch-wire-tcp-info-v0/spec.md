# miopunch-wire-tcp-info-v0 Specification

## Purpose
`miopunch-wire-tcp-info-v0` defines NAT-hole control-plane wire extensions that allow peers to carry TCP candidate information and TCP STUN view observations, and defines the derived TCP fields returned in `NatHoleResp`.

## Requirements

### Requirement: NAT-hole requests carry optional tcp_* candidate and observation fields
The system SHALL support including the following optional fields in NAT-hole request messages (`NatHoleVisitor` and `NatHoleClient`):
- `tcp_direct_addrs` (array of strings, each `host:port`)
- `tcp_assisted_addrs` (array of strings, each `host:port`)
- `tcp_mapped_addrs` (array of strings, each `host:port`)
- `tcp_stun_cn` (object, optional)
- `tcp_stun_global` (object, optional)

`tcp_stun_cn` and `tcp_stun_global` SHALL use the same schema as `stun_cn` and `stun_global` (`STUNViewObservation`).

#### Scenario: Request TCP fields are preserved across the wire
- **WHEN** a peer sends a NAT-hole request that includes `tcp_direct_addrs`, `tcp_assisted_addrs`, `tcp_mapped_addrs`, and `tcp_stun_cn/global`
- **THEN** the receiving side can decode the message and observe the same field values

### Requirement: Exchange response returns peer_tcp_direct_addrs, tcp_assisted_addrs, and tcp_candidate_addrs
When producing an exchange result (`NatHoleResp`), the system SHALL support the following optional TCP fields:
- `peer_tcp_direct_addrs` (array of `host:port`)
- `tcp_assisted_addrs` (array of `host:port`)
- `tcp_candidate_addrs` (array of `host:port`)
- `tcp_selected_view` (string)
- `tcp_selected_reason` (string)

`peer_tcp_direct_addrs` SHALL be derived from the opposite peer's `tcp_direct_addrs`.
`tcp_assisted_addrs` SHALL be derived from the opposite peer's `tcp_assisted_addrs`.

When deriving `tcp_candidate_addrs`, the system SHALL:
- drop empty entries and entries that are not valid `host:port`
- de-duplicate entries while preserving order
- prefer the selected TCP view's `mapped_addrs` when TCP view selection is available; otherwise use the opposite peer's `tcp_mapped_addrs`
- emit usable TCP attempt targets according to the current TCP attempt-target policy when such policy is active, including the `P+100` port convention defined by `miopunch-tcp-p2p-v0`

#### Scenario: Response includes derived tcp_candidate_addrs
- **GIVEN** the client request includes `tcp_mapped_addrs` with at least one valid entry
- **AND** the visitor request includes `tcp_mapped_addrs` with at least one valid entry
- **WHEN** the system produces `NatHoleResp` for both sides
- **THEN** each response includes `tcp_candidate_addrs` containing usable TCP attempt targets derived from the opposite peer's TCP mapped/view-source addresses

### Requirement: TCP cn/global view selection mirrors the selected_view algorithm
When both peers provide both `tcp_stun_cn` and `tcp_stun_global`, the system SHALL deterministically select exactly one `tcp_selected_view`.
The arbitration order SHALL be:
`availability` → `NAT feature difficulty` → `STUN RTT` → `ok_count` → `default global`.

When view selection occurs, the system SHALL set `tcp_selected_reason` to the first factor that decided the outcome (e.g., `availability`).

#### Scenario: Availability selects global when cn is unavailable
- **GIVEN** `tcp_stun_cn.available=false`
- **AND** `tcp_stun_global.available=true`
- **WHEN** the system produces an exchange response
- **THEN** `tcp_selected_view` is `global`
- **AND** `tcp_selected_reason` is `availability`
