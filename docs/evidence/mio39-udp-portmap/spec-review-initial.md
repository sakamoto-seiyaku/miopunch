# Initial Spec review

Independent GPT-6.1 Sol HIGH agent, code-review Spec axis. Reviewed
`a56bb9a6ae2ddbdabb34a39a5e3dbe2bb65d47a6...26c53281825ca1a230a8174d88e42137c5b74430`
in the isolated review worktree. Read-only; no tests executed.

One required finding (P2): owned mapping can survive graceful shutdown.

Spec: “保存映射拥有者和租约生命周期，不删除非本节点创建的映射。”
Also: “停止时只清理自身映射”.

NAT-PMP creation receives successful allocation before separately querying
the external address. Ownership reaches CurrentMapping only after that
second request succeeds. If its reply times out or shutdown cancels the
query, the gateway already installed the node's mapping but cleanup loses
ownership. The pending task is dropped and only CurrentMapping is released;
the requested lease is two hours.

Retain cleanup ownership once allocation succeeds and release it on later
failure/cancellation. Existing close evidence covered published mappings only.

No other missing/partial requirements, scope creep or optional suggestions.

## Resolution

The finding was reproduced through the real gateway in
`allocation-timeout-red`, fixed with acknowledged-allocation cleanup and
awaited cancellation, and verified in `allocation-timeout-green` and
`allocation-cancel-green`. A related renewal-candidate RED/GREEN also
ensures cleanup withdraws the older advertised lease. Final reassessment
uses the same independent review agents and is recorded separately.
