# miopunch-poc-v1-gui-wizard Specification

## Purpose
Defines the POC v1 GUI wizard stages and the user-facing output contract.

## ADDED Requirements

### Requirement: GUI stages are fixed to 6
The system SHALL implement exactly 6 GUI stages: Network, Enroll, Discover, Punch, SecureSession, Shell.

### Requirement: Summary + Evidence output contract
The system SHALL present a short UserSummary (<=3 lines) per stage and SHALL provide an Evidence view for detailed diagnostics.

### Requirement: reason_code cardinality is bounded
The system SHALL bound reason_code to at most 12 values; adding a new reason_code requires merging/replacing an existing one.
