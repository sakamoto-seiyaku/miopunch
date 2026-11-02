# Inner-Web prototype evidence

Disposition: this experiment was started by the assistant before the requested
design interview had reached agreement. The user corrected that scope error;
the preview is stopped. These historical receipts are retained for reference,
not as an accepted product design or completion of the client PRD.

MIOPUNCH-108, 2026-10-21. [Runtime result](result.json),
[source/build manifest](manifest.json), [reused artifact inputs](inputs.json),
[connected page](connected-page.png), [cookie-backed result](logged-in-page.png).

The Chromium participant is an independent BrowserNetwork node. It redeems a
one-use invitation, saves admission material and obtains an authorized ordinary
member connection to the NativeNetwork provider. Requests and responses travel
on that connection through an owned local Iroh relay. The provider alone maps
`node-only.invalid` to its owned HTTP fixture; browser requests do not contact
the target directly.

Eight runtime checks passed: refusal before admission; HTML/CSS/JS/SVG/Fetch;
cross-origin management-window refusal; navigation and POST/303/cookie login;
provider target refusal; provider-only target access; explicit unreachable-target
failure; and refusal after the member connection closes. Both owned actors exited
with code zero, and generated node material/cookies were removed.

The final walkthrough reused the verified native pins and the existing M103
Browser SDK, without invoking Cargo. Initial compilation reported 527 Fresh
artifacts and two rebuilt binaries; the corrected provider reported 528 Fresh
and one rebuilt binary. The intermediate missing trait import is retained as a
failed build, not cache-hit evidence. No dependency library was rebuilt.

An initial pin failed with EXDEV because `/tmp` and the repository are different
filesystems. The same outputs were pinned through `scripts/pin_artifact.py` on
the repository filesystem; this correction did not invoke Cargo again. The
shared `.artifacts/` store contains independent immutable objects, with per-run
links under `.artifacts/runs/inner-web-prototype/`.

This is a bounded HTTP and page-container proof. Its curl adapter, provider-side
single-user cookie jar, one selected origin and small body limits are temporary.
No production service ACL, arbitrary website/absolute-URL/CSP compatibility,
WebSocket, HTTPS target, streaming, Android or Tauri acceptance is claimed.
The separate-origin frame check proves the observed window/handle boundary,
not a complete security audit of a future production container.

The source is explicitly disposable; the [runner instructions](../../../browser-probe/prototype-web/README.md)
describe the one-command interactive entry point. The main implementation and
the accepted Vue/Tauri/Rust stack are unchanged.
