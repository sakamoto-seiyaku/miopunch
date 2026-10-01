SPEC review: **0 substantive unresolved findings**.

Independently pinned baseline `91cc520cbae87c5d8f02c7c754eab95af0eae375`; baseline..HEAD has no commits. Reviewed the nonempty actual 38-path cached diff and confirmed every staged file matches its worktree bytes. Authority: full MIOPUNCH-34/MIOPUNCH-14 snapshots, native prerequisite relations, current design and accepted ADRs.

- Missing/partial requirements: none found.
- Unasked product scope: none found.
- Implemented-looking but incorrect behavior: none found.

The requirement “收到有效撤销时拒绝新连接并终止已有组网和受控业务通信，不等待秘密轮换” is supported by actual connection lifetime ownership and removal propagation (`src/admission_gate.rs:209`, `tests/revocation.rs:114`). Both causal arrival orders retain StrongRemove/history; correction permits a new connection while original handles remain closed (`tests/revocation.rs:280`).

Fresh restricted admission requests permit separately approved rejoining; old requests/results still use the specific current grant query. Stable identity, durable material and reopen behavior are covered (`src/native_network.rs:468`, `src/manual_admission.rs:300`, `tests/revocation.rs:189`). Offline and self-departure semantics retain the existing session/elevation checks. The genuine recovery-stage race is corrected by final removal validation (`src/admission_gate.rs:72`, `tests/admission_gate.rs:934`).

Independently verified all **16 current source/harness hashes and recorded mtimes**, all **25 gzip compressed/raw hashes and byte-count roundtrips**, six compiled binary hash/mtime/dep-info bindings and the two recorded test-list bindings. No tests rerun or files changed.

Generation2’s actual log contains **34 successful summary blocks, 294 passed, 0 failed, 4 ignored, MQTT98**, and the terminal all-doctests footer. Four resumed checks have recorded exit0. **Full Cargo exit remains unknown/null; no pre-start generation2 SHA manifest survived.** Current hash equality, pre-start mtimes and compiled bindings support available provenance but do not prove continuously observed source stability during that original suite. Generation1 remains pre-fix history.
