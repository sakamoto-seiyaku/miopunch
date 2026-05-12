## 1. Dataplane Activity Accounting

- [x] 1.1 Update logical-stream activity callbacks so successful reads and writes refresh peer-session activity without changing idle-closer policy.
- [x] 1.2 Add Go regression tests for long-lived active logical streams and truly idle peer sessions.

## 2. Desktop Shell Focus

- [x] 2.1 Focus the embedded terminal when preview or live shell opens.
- [x] 2.2 Extend browser shell coverage to assert terminal focus after connect.

## 3. Validation

- [x] 3.1 Run focused Go tests for `dataplane`.
- [x] 3.2 Run focused desktop browser tests for the shell flow.
