# Daily administration and temporary genesis elevation

MIOPUNCH-32 connects the management behavior in
[the parent specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)
to the existing `NativeNetwork` boundary. The implementation baseline is
`8ee52120b6499b5ff70142aba571e2bc43c7ce2d`.

## Selected integration

`begin_management_session`, `elevate_genesis`, and `manage` expose ordinary
member addition/removal, administrator appointment, and administrator demotion.
The existing manual-admission path still approves applicants and delivers the
current admission secret; a signed Add alone does not report admission success.
`ManagementResult` distinguishes the requested action, its committed record hash,
and the local effective access at return time. Demotion yields Read membership;
removal yields no membership. Later causal correction can change that view.

Each opaque session belongs to one open node instance. It owns an optional
root signing key and a monotonic expiry, is neither cloneable nor serializable,
and has no secret-bearing Debug representation. The caller explicitly supplies
the independent root private key together with the trusted canonical Create
hash, as fixed by ADR 0008. Opening a creator's stored data does not elevate it.
Each management session starts unelevated; another device, another session, and
even a reopened instance of the same device cannot reuse its window. Explicit
exit drops the session's credential; ending the session invalidates it. Expiry
is checked after asynchronous recovery and again immediately before signing,
after causal graph/log preparation. Expired credentials are discarded on that
check or feedback query. There is no claim of a secure-memory deadline timer.
Window duration remains a caller choice; zero or unrepresentable windows reject.

The implementation reuses ADR 0007's authority mapping. Unelevated daily actions
use the node key; an elevated session uses the separate root key. Administrator
appointment/demotion and removal of a still-administrator require elevation.
The root virtual member cannot be managed as a device. A fresh random author
log ID, checked against retained verified history, is allocated for each action,
matching the existing manual-admission authoring pattern. Every action depends
on all verified causal heads. Sequential root actions therefore retain their
causal relationship without sharing a mutable root log tail across devices.

Local authoring preflights the signed record against a clone of the selected
causal verifier before writing. The actual durable ingest/application still
uses `ManagementAuth` and the transactional `ManagementStore` path. Network
receipt, third-party forwarding and restart use the existing MIOPUNCH-31 LogSync
and replay integration. Session handles, booleans, expiry and root private keys
never enter shared records. Previously valid root-signed records remain valid
after the local window ends; recipients authorize the original signature and
causal prestate, independently of the forwarding node's role or session.

## Checks and failures

The first RED was the missing public management API. The first runtime attempt
failed because the sandbox denied socket binding, before any connection test;
that log is preserved. The same test then ran with authorized network access:
a daily administrator added and removed an ordinary member, supplied the root
credential to appoint an administrator, and the remote admitted node verified
and retained the role after reopen. Only then was the priority failure tested.

The priority test uses actual monotonic expiry and running independently
admitted nodes. Expired, explicitly exited, ended, foreign-device, other-session,
and pre-reopen session windows cannot sign privileged records. Repeated valid
actions share one verified local window without propagating its signing ability.

Demotion's API was added after its own missing-variant RED. A root-signed
demotion retained ordinary membership; separate root-authorized removal ended
membership. After root-session exit and original-author shutdown, a restarted
ordinary forwarder supplied the exact records to another node, whose reopened
view retained both distinct results.

Five separately constructed real daily-administrator signatures bypass the
convenience API: Add Manage, Promote, Demote, Remove an administrator, and
administrator self-removal. All fail local application. A fault-build provider
then sends each record through real admitted TLS/QUIC and LogSync events.
Recipients save and quarantine the unauthorized record, never apply or serve
it, and preserve that exclusion after reopen and third-party forwarding.
Transport Done and complete saved bytes are explicitly distinguished from
authorized application. The fixture provider exists only in `replica-test-hooks`
builds and does not replace signing, transport, verification or storage.

The causal test independently authors B's ordinary-member removal and the
root's concurrent demotion of B against their own available histories. One
receiver sees removal first and initially loses the member; the demotion later
corrects it. Another sees demotion first. Both effective views and reopened
views retain B and the target as ordinary members. Duplicates do not increase
applied count, and the valid-but-filtered original removal remains history.

Credential boundary checks reject node keys, admission secrets, a root key with
the wrong canonical Create hash, and invalid durations. The current evidence
and reproducible commands are in
[`docs/evidence/mio32-daily-management/`](../evidence/mio32-daily-management/README.md).

## Limits

The final generation ran the full real-broker workspace once after the source
was fixed: **274 passed, 0 failed, 4 existing ignored**. The actual MQTT capture
observed 92 publishes. Full suite, browser WASM, both default and fault-build
typechecks, and formatting each exited 0. The ten source/harness hashes stayed
unchanged through the checks; owned broker cleanup was independently verified.
The affected short regression had previously passed 19/0. See the evidence
receipt for commands, clocks, sources and losslessly retained raw logs.
Independent final Standards review found 0 required and 1 optional maintenance
finding; Spec review found 0 required or optional findings. The Spec reviewer
also checked continuation after filtered/quarantined records: valid-but-filtered
records remain applied/servable history, while later public authoring uses fresh
log IDs and verified causal heads, avoiding invalid quarantined backlinks. No
runtime or harness changes followed the final checks or reviews.

These checks exercise native Linux loopback, real Ed25519, SQLite, iroh TLS/QUIC,
manual admission and LogSync. Reopen checks establish durable replay; physical
power loss and broader platform runtime are not claimed. Browser validation is
compilation only. Window duration and trusted credential import belong to the
caller; backup media and formal UI remain deferred. This slice does not deliver
rotation, precodes, full root-recovery lifecycle, migration, service ACL or TUN.
It preserves the existing admission and connection gates. Prior NAT probability
batches and frozen evidence do not require reruns for this management change.
