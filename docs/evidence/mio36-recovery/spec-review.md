Spec rereview — MIOPUNCH-36 latest179 staged paths against f691d2b4c2c0f8188d9e9418ef691c21978a806c.

Required: 0. Optional: 0. Initial P2 closed.

Parent14 requires: “有效移除或退出后重入需要新批准或新有效邀请，可以保留节点身份；旧授权和旧申请重放不能重新取得成员资格。” The repair satisfies this: src/native_network.rs:543 permits fresh ordinary rejoin only after effective membership loss; src/management_recovery.rs:247 verifies historical root-proof/grant evidence and saved material/completion without granting current authority. Completion sealing precedes atomic/fsynced ordinary-binding publication; reopen at src/native_network.rs:427 validates retained recovery evidence and activates only the ordinary flow. Pending root recovery remains isolated. The old root result stays invalid, and fresh approval restores only Read membership with the same identity, canonical anchor and history.

The public case at tests/management_recovery.rs:1457 demonstrates actual recovery, root-authorized administrator appointment/removal, LogSync, reopen, a distinct ordinary request/grant and durable Read membership. The two switch-boundary tests at :1760 and :1769 actually exit86, reopen safely, preserve the fresh request and require new approval. Original P2 RED101, corrected GREEN0 and the separate moved-value compile failure remain preserved.

No additional missing/incorrect requirement or scope expansion found. Exporter-bound proof before bounded signed history/secret, canonical Create, independent identity, causal merge, NEW versus CURRENT grant validation, durable stages/retries and separate explicit elevation remain intact.

Latest final2 evidence independently checks116/0/0, all36 recovery tests, eighteen real exit86 boundaries and MQTT85. Ten affected targets, Cargo441.456s, WASM/default/fault/fmt and wrapper79579 each exited0. All54 prestart inputs match current source and the preserved runner. Owned broker/two volumes have cleanup0 and independent absence receipts; all54 compressed logs roundtrip exactly. Exact179 staged/current hashes, protected59 and original36 dirty entries are unchanged; the initial report and final1 receipts remain immutable historical evidence.

Frozen60–63/prior real passes support omitting unchanged NAT; actual hunks do not affect its packet/transport paths. Forty-five distinct skips have no collisions with observed or new P2 names; include-nat preserves original argv. The latest affected command matches its actual receipt. A filtered whole-workspace suite remains unexecuted and is not claimed.
