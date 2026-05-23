# miopunch-poc-v1-dial-punch Specification

## Purpose
Defines POC v1 dial_offer/dial_answer and UDP punching attempt strategy (5B).

## ADDED Requirements

### Requirement: POC v1 uses UDP punching only
The system SHALL establish P2P paths using UDP punching only in POC v1.

### Requirement: Dial exchange uses exactly dial_offer and dial_answer
The system SHALL exchange candidates and punch parameters using only `dial_offer` and `dial_answer` messages (peer_e2e_v1) over inbox topics.

### Requirement: Punch attempt strategy is bounded and deterministic
The system SHALL attempt candidate pairs with max concurrency 4 and a fixed total time budget, and SHALL produce evidence for attempts.
