## 1. Test Infrastructure

- [x] 1.1 Add reusable Playwright helper modules for fake Wails bridge setup, runtime event capture, topology fixtures, and action call assertions.
- [x] 1.2 Add global browser error guards so page errors and unexpected console errors fail desktop UI tests.
- [x] 1.3 Add `findings.md` and document that product UI defects found by expanded tests are logged before fixes.

## 2. Desktop UI Behavior Coverage

- [x] 2.1 Cover primary tab navigation and second-level overview/detail transitions for Network, Access, Admin, and Settings.
- [x] 2.2 Cover owner/member/empty/disconnected fixture behavior, including admin visibility and disabled states.
- [x] 2.3 Cover Access Join, Invite, and Approve actions with bridge argument assertions and visible result/progress states.
- [x] 2.4 Cover Network peer actions: Ping, List sessions, Shell navigation, shell attach, and invalid peer disabled states.
- [x] 2.5 Cover Admin member detail and Revoke action argument/state behavior.
- [x] 2.6 Cover Settings Local daemon apply/clear, Diagnostics, Preview, refresh, and runtime event rendering.

## 3. Validation

- [x] 3.1 Run OpenSpec strict validation for `expand-desktop-ui-test-coverage`.
- [x] 3.2 Run focused desktop frontend validation: JavaScript syntax check and `npm test`.
- [x] 3.3 Run repository hygiene checks relevant to this change and record any intentionally unfixed UI findings.
