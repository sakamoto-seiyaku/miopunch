## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`

## 2. Define v1 state layout

- [ ] 2.1 Add path helpers for `device/` and `networks/<network_id>/`
- [ ] 2.2 Define file formats (bin for keys/secrets; json for UI/debug)

## 3. Implement atomic read/write

- [ ] 3.1 Atomic write helpers (tmp + fsync + rename where supported)
- [ ] 3.2 Permissions: ensure 0600/0700

## 4. Migrate callers

- [ ] 4.1 Add persist API usage for join/enroll (writes to v1 layout)
- [ ] 4.2 Add persist API usage for GUI/CLI (reads from v1 layout)

## 5. Post-change Validation

- [ ] 5.1 Re-run `go test ./...`
