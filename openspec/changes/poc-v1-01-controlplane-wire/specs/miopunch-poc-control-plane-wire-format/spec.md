# miopunch-poc-control-plane-wire-format Specification

## Purpose
为历史 POC v0 JSON/AES-GCM control-plane wire 增加 source-of-truth 边界说明，防止其继续被误用为当前 POC v1 peer-targeted 消息的合同。

## ADDED Requirements

### Requirement: POC v0 control-plane wire is legacy-only after v1 extraction
The system SHALL treat the JSON/AES-GCM control-plane wire defined by `miopunch-poc-control-plane-wire-format` as a legacy POC v0 contract only.

Current POC v1 peer-targeted control-plane messages SHALL use `miopunch-poc-v1-controlplane-wire` as their source of truth and SHALL NOT reuse this legacy capability as the runtime contract for current v1 extraction work.

#### Scenario: Current v1 implementation chooses the v1 wire contract
- **WHEN** a developer implements or reviews the current POC v1 peer-targeted control-plane path
- **THEN** they use `miopunch-poc-v1-controlplane-wire` as the governing contract
- **AND** they treat the JSON/AES-GCM capability as historical reference for archived POC v0 only
