# MIOPUNCH-45 Standards review

Reviewed committed diff `2b3711ab5a9af39a560902f52bcf8f6f46cd65b9...4c4b5dc387a56f9d968d2d49cf33cb098052131e` (one commit: `Implement MIOPUNCH-45 bounded IRC control bootstrap`). Working-tree edits were excluded.

**Hard documented-standard violations: none found.** Sources: root `AGENTS.md`, `CONTEXT.md`, `docs/agents/domain.md`, current design entry points, ADRs 0004/0005. The change preserves the restricted control-bootstrap vocabulary, independent membership authorization and stable node identity. Adapter choices and qualification limits are recorded explicitly; no archived Go/OpenSpec gate or research suggestion becomes a new product requirement. No separate coding-standard file exists. Tooling-enforced formatting/compilation is excluded from this axis.

**Non-blocking judgement calls:**

1. Possible **Duplicated Code**, `src/resources.rs:156–171`: the new `irc_resources` copies `mqtt_resources`' parsing, permission filtering and origin deduplication, differing only in scheme. Quoted hunk: `if url.scheme() == "irc" && self.allows(&resource.url, ResourceUse::Signaling) { origins.insert(resource_origin(&resource.url).unwrap(), url); }`. A private scheme-filtered resource selector could keep both callers on the same configuration-policy implementation.

2. Possible **Duplicated Code**, `tests/irc_bootstrap.rs:213–223,341–351`: both new fixture clients repeat registration/channel-join handling. Quoted shared hunk: `if line.contains(" 001 ") { ... JOIN {CHANNEL} ... }` followed by `if line.contains(" 366 ") { break; }`. Extract the real fixture-client registration helper so later protocol handling fixes reach the observer and injection client together.

3. Possible **Data Clumps**, `src/irc_transport.rs:392–401,472–482`: `drive` and `read_loop` repeatedly pass the binding's route, service, address watcher, status and traffic separately. Quoted parameter group: `route: Route, service: Service, addresses: Watchable<Vec<CustomAddr>>, ... status: watch::Sender<Status>, traffic: Arc<Traffic>`. A small binding-state type could make ownership and lifecycle easier to follow while leaving read/write channels explicit.

Evidence review: authoritative `completed-acceptance` and `continued-short` receipts report `passed=true`; the earlier `final` receipt reports failure and is accurately explained in the evidence index. No tests were rerun and no runtime/test files were edited.

Summary: 0 hard violations; 3 low-severity maintainability observations, all judgement calls.
