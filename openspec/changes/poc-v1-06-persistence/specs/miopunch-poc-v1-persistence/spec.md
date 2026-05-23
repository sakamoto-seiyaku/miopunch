# miopunch-poc-v1-persistence Specification

## Purpose
Defines the POC v1 on-disk state layout and persistence rules.

## ADDED Requirements

### Requirement: State layout is device/ + networks/<network_id>/
The system SHALL store device-global keys under `device/` and network-scoped state under `networks/<network_id>/`.

### Requirement: State writes are atomic
The system SHALL write state files atomically (tmp + rename) to avoid partial writes.

### Requirement: State file permissions are restrictive
The system SHALL ensure state directories and files are created with restrictive permissions (0700/0600).
