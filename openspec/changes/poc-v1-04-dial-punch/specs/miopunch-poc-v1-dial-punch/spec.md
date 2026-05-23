# miopunch-poc-v1-dial-punch Specification

## Purpose
Defines POC v1 dial_offer/dial_answer and UDP punching attempt strategy (5B).

## ADDED Requirements

### Requirement: POC v1 uses UDP punching only
The system SHALL establish P2P paths using UDP punching only in POC v1.

#### Scenario: Path establishment does not branch to other carriers
- **WHEN** a v1 peer tries to establish a direct path to another peer
- **THEN** it performs UDP punching only
- **AND** it does not fall back to TCP, relay, or other carrier types within this change

### Requirement: Dial exchange uses exactly dial_offer and dial_answer
The system SHALL exchange candidates and punch parameters using only `dial_offer` and `dial_answer` messages (peer_e2e_v1) over inbox topics.

#### Scenario: Candidate exchange stays within the fixed two-message protocol
- **WHEN** two enrolled peers coordinate a v1 punch attempt
- **THEN** they exchange candidates and punch parameters using only `dial_offer` and `dial_answer`
- **AND** those messages are sent over inbox topics with `peer_e2e_v1`

### Requirement: Punch attempt strategy is bounded and deterministic
The system SHALL attempt candidate pairs with max concurrency 4 and a fixed total time budget, and SHALL produce evidence for attempts.

#### Scenario: Punch attempts stop at the fixed v1 budget
- **WHEN** a v1 punch run starts with candidate pairs to try
- **THEN** the runtime schedules at most 4 concurrent attempts within the fixed total budget
- **AND** it records evidence for the attempted pairs and the selected result
