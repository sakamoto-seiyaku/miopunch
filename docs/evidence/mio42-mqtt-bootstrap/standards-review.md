# Standards review

Independent GPT-6.1 Sol HIGH Fast reviewer, base
`0f618d176e3021bf65c75283580fbd57e2355598`, owned MIOPUNCH-42 WIP only.

No documented-standard breaches found in the owned MIOPUNCH-42 changes. The
implementation preserves the identity/admission/carrier separation in
`CONTEXT.md` and ADR 0005, and records integration choices explicitly.

One low-priority judgement finding:

- **Possible Shotgun Surgery** — `scripts/mqtt_bootstrap_lab.py:13` borrows
  another runner through source-text slicing and replacements:
  `source[:source.index('client_id=$(')]`, followed by changes to literal ports,
  case lists, cleanup loops and broker startup. Changes to the shared NAT
  fixture's layout or setup now require coordinated edits to this implicit
  contract; replacements can silently stop matching. Extract the reusable
  kernel setup into a sourced fixture with explicit parameters or documented
  insertion points, while preserving the validated topology.

Standards: **0 hard violations; 1 optional maintenance smell**. No independent
tests run.
