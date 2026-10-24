// Actual Async Clipboard API only. No synthetic events or memory fallback.
const limit = 1024 * 1024;

export function clipboard_capable() {
  return Boolean(globalThis.isSecureContext && globalThis.navigator?.clipboard?.read
    && globalThis.navigator?.clipboard?.writeText && globalThis.document);
}

export async function clipboard_capability() {
  let reason = null;
  if (!clipboard_capable()) reason = "secure_clipboard_api_unavailable";
  else if (!document.hasFocus() || document.visibilityState !== "visible") reason = "focused_visible_document_required";
  else {
    // Automatic operation requires persistent permission. A browser which only
    // permits a fresh user gesture for each read cannot promise automatic sync.
    try {
      const [read, write] = await Promise.all([
        navigator.permissions.query({name: "clipboard-read"}),
        navigator.permissions.query({name: "clipboard-write"}),
      ]);
      if (read.state !== "granted" || write.state !== "granted") reason = "automatic_clipboard_permission_required";
    } catch (_) {
      reason = "automatic_clipboard_permission_unavailable";
    }
  }
  return JSON.stringify({backend: "browser/async-clipboard", read: reason === null,
    write: reason === null, reason, formats: ["text/plain"], max_text_bytes: limit});
}

function current_context() {
  if (!clipboard_capable() || !document.hasFocus() || document.visibilityState !== "visible") {
    throw new Error("clipboard_context_unavailable");
  }
}
export async function clipboard_read() {
  current_context();
  const items = await navigator.clipboard.read();
  for (const item of items) {
    if (!item.types.includes("text/plain")) continue;
    const blob = await item.getType("text/plain");
    if (blob.size > limit) throw new Error("clipboard_text_too_large");
    const text = await blob.text();
    if (new TextEncoder().encode(text).byteLength > limit) throw new Error("clipboard_text_too_large");
    return text;
  }
  // readText() would also return "" for a clipboard with no text format.
  // Preserve that distinction; an actual empty text item returns "" above.
  return null;
}
export async function clipboard_write(text) {
  // No await precedes the OS invocation. Rust checks the current gate in this
  // same event-loop turn; the browser itself enforces live system permission.
  current_context();
  if (new TextEncoder().encode(text).byteLength > limit) throw new Error("clipboard_text_too_large");
  await navigator.clipboard.writeText(text);
}
