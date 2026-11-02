# System clipboard implementation

MIOPUNCH-117 implements the non-UI clipboard path of the current client PRD.
The public operations are `enable_clipboard`, `start_clipboard`,
`stop_clipboard`, `disable_clipboard` and `clipboard_status` on NativeClient
and BrowserNetwork, with equivalent JSON commands on `miopunch-client`.

## Engineering decisions

Capability, explicit service enablement and an active selected pair remain
separate. Enabling publishes the existing signed clipboard service declaration;
starting selects one admitted peer under each provider's current effective
Service::Clipboard ACL. Existing member/guest authorization applies. A second
pair is rejected until the first ends. No additional permission engine or
per-change confirmation is introduced.

The existing Files guest operation keeps its original scope. The additional
`issue_service_guest` operation selects Files or Clipboard in that same signed
invitation and single-identity binding; the CLI's `guest_issue.service` defaults
to Files for existing callers. Transport qualification and both endpoints'
effects check that exact service/provider/deadline. Clipboard guests receive no
ordinary membership, Files access or unrelated member protocols. Current expiry,
exit, revocation and effective ACL stop their current pair. Existing contact
backoff also applies to explicit retries after a denied network contact.
The transport projection takes a deadline only from each exact currently live
invitation of that provider and service. Multiple live grants may combine;
ended invitations and another service's longer grant do not extend the pair.

The shared protocol carries UTF-8 `text/plain` up to 1 MiB per change. Capability
reports this format and bound. Empty text is a change; an absent text format is
not empty text. Other formats stay untouched. Starting takes a local baseline
and sends subsequent changes, so starting alone does not replace either current
clipboard. One unacknowledged value bounds the sender; newer local changes are
sampled after acknowledgment. A received value becomes the local baseline to
suppress echoes. Acknowledgment means that the platform write completed.

Setup reserves the pair before waiting for transport. Stop retires queued
requests and cancels pending setup before joining workers and platform effects.
Dropped setup releases its reservation and connection. A changed effective ACL,
disable, platform/context failure or disconnect ends the pair; recovery requires
an explicit new start. Host shutdown joins owned work. Clipboard values and
digests are absent from signed records, persisted state, status and diagnostics;
status exposes peer, current transport, counters and fixed failure codes.
Native `clipboard_status` is asynchronous so it can probe the live platform
without sampling its text (X11/Wayland format metadata or a Win32 open/close).
Unified `service_status` reports that current capability; Browser status queries
the real current context and persistent permissions. Recorded declarations
retain their signed revision semantics.

Linux uses real `wl-copy`/`wl-paste` or `xclip` processes available on PATH.
It owns foreground selection processes and retains the current owner across
disable/enable. Missing tools/display or failed calls report unavailable; there
is no memory fallback. Windows uses eager Win32 Unicode clipboard data in the
current visible interactive window station and input desktop. Browser uses the
real Async Clipboard API, a secure focused visible document and granted
persistent read/write permissions. Unsupported browser permission models report
unavailable automatic operation. Browser MIME inspection distinguishes absent
text from empty text.

## Qualification boundaries

The local acceptance uses distinct authenticated X11 servers with high display
numbers, private authorities/runtime directories, disabled global session bus
and cleared alternative Wayland/Mir contexts. Independent OS readers establish
the result. Chromium runs headed in the owned X11 session and attaches through
Playwright `noDefaults: true`; actual document focus is observed independently.
This qualifies those Linux X11 and Chromium/WASM contexts, including duplex
writes, selected-pair isolation, enable/disable, current ACL, permission/focus
loss, reopen, formats and payload bounds. It does not establish Wayland or
Windows runtime behavior, WAN routing, or two physical machines.

An earlier abstract X11 display `:0` loss probe could fall back to the host's
named WSLg display. Its failed isolation observation and metadata-only incident
record remain preserved; its outcome is not accepted as desktop-loss proof.
No host clipboard content or digest was inspected to investigate. Hardened
non-business isolation/focus preflights and subsequent business cases have
separate receipts. Original failures and corrected assertion prerequisites are
retained with the source and artifact provenance.

Actual Windows interactive desktop qualification remains pending applicable
user authorization. Cross-compilation is compilation evidence only. Android
real-device host integration remains the explicit MIOPUNCH-121 boundary.

The prepared independent Windows technical host keeps restorable HGLOBAL
formats only in bounded private RAM. Capture and the first marker share one
clipboard transaction. Restoration compares the last directly controlled
sequence and text and writes the snapshot under the same transaction;
matching received SDK data does not establish restoration ownership. Later
or unproven revisions, including identical or empty text, remain untouched.
Windows documents that [OpenClipboard prevents other applications from changing
the contents](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard)
and that [changes or emptying advance the sequence number](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber).
The controller aggregates cleanup errors and still attempts its final receipt.
Local Linux PowerShell parsing and C#5 managed compilation validate source
syntax only; Windows API execution, Windows PowerShell 5.1 and the desktop
qualification are still pending.

## 2026-10-29 Windows entry and explicit deferral

The current Windows and Linux SDKs passed the real interactive Win32↔private
X11 core chain: Unicode duplex transfer, independent system reads, write
acknowledgments, no echo and no writes after stop. All test owners terminated.
The first snapshot entry failed before Windows SDK startup; subsequent read-only
inspection observed OpenClipboard error 5. The user then confirmed an ordinary
copy, after which the same business chain passed. The original access-denied
cause remains unknown. Cleanup preserved a later or unproven clipboard revision;
it did not restore the snapshot or force an overwrite.

The user explicitly deferred further clipboard qualification because it depends
on the test machine remaining unlocked. Third-node and effective-policy Windows
cases remain unverified; M117 is not complete. See the
[current evidence](../evidence/system-clipboard-2026-10-29/windows-entry-qualification.json).
