# File operations for Native and browser nodes

2026-10-23 — [MIOPUNCH-114](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/918b2f20-b668-4a72-93da-f5dc5419ea29).
The [qualification report](../evidence/file-transfer-2026-10-23/README.md)
identifies the actual platforms, source graphs, immutable artifacts and limits.

The file module uses the existing durable node identity, member admission gate
and configured connection coordinator. Its `miopunch/file/1` application
protocol has the existing member-business purpose. A host explicitly enables
its live receiver; approved ordinary members and administrators then send
without another receipt approval. Hosts control the lifetime of that local
enable choice. A reopened SDK host starts with its receiver closed.

`NativeClient` reads source files and saves received files itself. Its host
passes paths and observes task state. `BrowserNetwork` reads immutable `File`
objects by bounded slices and saves through actual origin-private filesystem
(OPFS) writable streams. `received_file` returns a saved browser `File` for
later export by a host. Browser receipt success refers to the OPFS commit;
exporting that saved file to another destination is a separate host action.

The save choices are module implementation details: Native receives into an
explicitly selected directory; OPFS uses a directory for the browser node's
identity. Files get a transfer-ID prefix, a bounded UTF-8 basename, and an
exclusive destination. Existing files are preserved. Remote names are never
interpreted as paths. These choices do not freeze a future product directory,
filename display or platform save-dialog design.

## Public Native host operations

The existing JSON-lines `miopunch-client` host exposes these operations. Every
request includes `id`; responses contain the same `id`, `ok`, and a result or
the existing secret-free host error.

| Operation | Arguments | Result |
| --- | --- | --- |
| `files_enable` | `directory` | Explicitly enable this live receiver. Membership and the creator's recovery-save gate must be complete. |
| `files_disable` | — | Close the service and cancel incoming tasks. Task state reports the result of incomplete-file cleanup; completed files remain. |
| `file_send` | Peer public `address`, local `path` | Start a bounded background task and return its `task` ID. |
| `files_status` | — | Service state, bounded task history and stream/task bounds. |
| `file_cancel` | `task` | Request cancellation. `cancelling` remains pending until owned I/O and partial cleanup finish. |
| `file_disconnect` | `task` | Technical host control to close only that task's present connection and exercise ordinary continuation. |

The SDK counterparts are `enable_files`, `disable_files`, `send_file`,
`file_tasks`, `cancel_file`, and `disconnect_file`. The existing client `close`
joins its tasks and closes the endpoint and durable store. Await it before
reopening the node directory.

`file_cancel` acknowledges `cancel_requested`, rather than claiming a terminal
result. Repeated requests return false once cancellation is already requested
or the task has ended. The service retains an explicit cleanup owner even when
the network receive has already returned `interrupted`; cancel, disable and
close therefore release its real partial and live task slot without a peer
retry. Native and WASM use the same receive/cleanup state transitions.

Native retains at most 16 established receiver connections and owns their jobs
outside the listener's abortable handshake set. It reserves a task before
creating its temporary file. Disable invalidates pending preparation and closes
receiver connections; host close also joins their storage work. Dropping a host
stops new receiver ownership and requests cleanup from those existing owners.
The running async runtime must remain available for that work; await `close`
when the host can do so.

Preparation rejection and retained-partial expiry await actual sink cleanup.
Both adapters report a cleanup error as `failed` / `save_failed`, preserving it
instead of claiming `cancelled` or replacing it with an interruption. A failed
cleanup can leave an incomplete file; the task has no successful receipt.

## Public browser operations

`BrowserNetwork.enable_files()` explicitly enables OPFS receipt after admission
and any recovery-save confirmation. `send_file(peerId, file)` returns a task ID;
`file_status()` returns the same task-state meanings as Native.
`cancel_file(task)`, `disconnect_file(task)` and `disable_files()` control the
same live module. `received_file(savedName)` obtains the actual saved `File`
without materializing its contents as a response. Await `close()` before
reopening the identity. Saved files and node identity survive a normal page
reload; browser storage clearance and page termination retain their actual
platform limits.

## Streaming and task completion

The shared `data_stream` module frames at most 64 KiB per record, rejects larger
records before allocating their payload, and awaits QUIC backpressure. Its
reader and writer can be split for duplex consumers. `BrowserDataStream`
exposes bounded `read_chunk`, `write_chunk`, `finish` and stream-only `cancel`
through admitted member connections. It admits one pending operation per
direction and rejects another rather than accumulating a host-side queue.

Each node admits four live file tasks and retains at most 64 task summaries.
The Native adapter serializes the contact-setup phase to honor the existing
resource owner's pending-contact contract; connected streams may run together.
Files have no demo-derived size ceiling. Source hashing, network records and
platform writes are incremental, with no file-sized task-state payload.

The sender hashes its source before offering its immutable task metadata.
The receiver verifies size and SHA256 while writing. It flushes and exclusively
publishes the Native file, or closes and commits the actual OPFS writable,
before returning a successful receipt. The sender reports `complete` only
after receiving and validating that receipt. Save failures remain failures.
Completed receipt metadata is retained within the same bounded history so a
lost final acknowledgment can be retried without another save.

The states are `preparing`, `transferring`, `interrupted`, `saving`,
`cancelling`, `complete`, `failed`, and `cancelled`. A receipt includes task ID,
actual size, SHA256, and saved name. Diagnostics identify the actual connection
and selected transport path without exposing file contents or credentials.

Browser storage keeps ownership of its provisional output and fallback writer
through cleanup. If cancellation wins before the SDK observes a JavaScript save
result, discard also retracts that unacknowledged output. Once the SDK retains a
successful receipt, normal close or disable preserves the saved file. Cleanup
attempts the remaining owned files even when one step fails, and reports the
failure instead of a successful cancellation.

## Interruption and access boundaries

A live receiver retains a bounded partial and its prefix hash for a short
continuation window, with expiry checked after about a minute. The same sender,
task metadata and admission must still match. The sender makes at most three
connection attempts and resumes from the receiver's accepted byte offset.
A replacement connection is a new connection; it does not revive an old
stream. Service close, task cancellation, source/save failure, exhausted retries
and discarded runtime state lead to an explicit terminal result. Restarting a
node does not offer a persistent transfer-recovery system.

Existing membership enforcement protects this protocol as it protects other
member traffic. The module's current service policy is explicit local enable
followed by ordinary-member access. The service ACL and scoped guest model are
the separate MIOPUNCH-115 integration; M114 grants no guest file access and does
not claim that flow is qualified.

The Native listener preserves the original absolute budget for its existing
finite admission/member operations. File handshake setup is bounded, while a
large file has a per-I/O idle timeout instead of a total transfer-duration cap.
Automatic browser receivers start that setup clock at the actual incoming
packet. Waiting for a future transfer consumes no handshake budget. Explicit
foreground operations retain their original caller deadline, and background
maintenance ticks retire old partials without shortening a new handshake.
This operation boundary adds no product pages or platform lifecycle promise.
