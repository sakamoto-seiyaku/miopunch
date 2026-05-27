## 1. Runtime Broker Override

- [x] 1.1 Keep the runtime broker override wired through `miopunch up` on Unix and Windows.
- [x] 1.2 Make `init-network` use the override instead of starting an embedded broker when the override is present.
- [x] 1.3 Add focused tests for broker override parsing and runtime bootstrap behavior.

## 2. CLI Help and Flags

- [x] 2.1 Document `up --broker <endpoint>` in `miopunch --help`.
- [x] 2.2 Keep `parseUpOptions` accepting `--broker` and `--broker=`.
- [x] 2.3 Add or update unit tests for the `--broker` flag path.

## 3. Guest Smoke Gate

- [x] 3.1 Add a guest runner for the new CLI smoke gate.
- [x] 3.2 Start two node containers with direct daemon startup and explicit broker/localapi/state overrides.
- [x] 3.3 Validate the positive path through `up -> init-network -> invite -> approve -> join -> ls -> ping -> sh ls`.
- [x] 3.4 Fail fast when the broker URL is missing.

## 4. Host Wiring

- [x] 4.1 Add a `labctl` command for the new smoke gate.
- [x] 4.2 Push the required binaries and guest script into the VM.
- [x] 4.3 Pass the host broker URL into the guest smoke runner.

## 5. CI and Docs

- [x] 5.1 Wire the smoke into the intended mainline or dispatch workflow.
- [x] 5.2 Update lab docs to explain the new smoke gate and broker requirement.
- [x] 5.3 Verify the change with focused Go and lab validation.
