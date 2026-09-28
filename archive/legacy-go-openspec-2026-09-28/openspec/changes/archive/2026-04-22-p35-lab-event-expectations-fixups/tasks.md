## 1. Reproduce & Inventory

- [x] 1.1 Re-run `./lab/host/labctl xtcp-connectivity-selftest` and capture failing cases + missing matcher evidence
- [x] 1.2 Confirm why `--disable-stun` does not produce `gather.stun.skip` under current runner behavior

## 2. Fix Lab Runner STUN Disable Semantics

- [x] 2.1 Update `lab/guest/bin/mlab-xtcp-run` so `--disable-stun` passes an explicit empty STUN config to peers (disables internal defaults)
- [x] 2.2 Ensure both client and visitor paths apply the same `--disable-stun` behavior

## 3. Expectation Alignment

- [x] 3.1 Verify `lab/guest/cases/expect/p2-01-v6-direct.events.json` passes after runner fix (no expectation drift)
- [x] 3.2 Verify `lab/guest/cases/expect/p2-02-portmap-direct.events.json` and `p2-04-v6-fallback-direct-ipv4.events.json` pass after runner fix

## 4. Validation

- [x] 4.1 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...` (sanity)
- [x] 4.2 Run `./lab/host/labctl xtcp-connectivity-selftest` and ensure full pass
