# Archive Layout

## Purpose

`archive/` stores repository-local snapshots and retired implementation
surfaces that should remain available for reference without staying mixed into
the active design path.

## Current Entries

- `archive/_legacy_poc_v0/`
  - Removed legacy POC implementation that current
    `poc-v1-01..07` changes are extracting away from.
  - The active root module no longer carries those live source paths; this
    archive is now the retained reference copy.
  - Retired lab/control-plane smoke commands live under `live_removed/` so
    root-module `go test ./...` stays focused on the current POC product path.
