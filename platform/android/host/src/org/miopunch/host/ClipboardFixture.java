package org.miopunch.host;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Build;
import android.os.Looper;
import android.os.PersistableBundle;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;

/** Debug-only independent framework observer. Original data stays in app RAM. */
final class ClipboardFixture {
    private static final int MAXIMUM = 1048576;
    private static final String OWNER = "org.miopunch.verify.clipboard.owner";
    private final Context context;
    private final String lease;
    private ClipData original;
    private boolean active;
    private String ownedText;
    private long ownedTimestamp;

    ClipboardFixture(Context context, String lease) { this.context = context; this.lease = lease; }
    boolean active() { return active; }

    private ClipboardManager manager() {
        if (Looper.myLooper() != Looper.getMainLooper()) throw new IllegalStateException("main_thread_required");
        VerificationActivity activity = VerificationActivity.current;
        if (activity == null || !activity.hasWindowFocus()) throw new IllegalStateException("focused_activity_required");
        return context.getSystemService(ClipboardManager.class);
    }

    private String marker(JSONObject request) throws Exception {
        String label = request.getString("label");
        if (!label.matches("[A-Za-z0-9_-]{1,64}")) throw new IllegalArgumentException("invalid_marker_label");
        int bytes = request.getInt("bytes");
        if (bytes == 0) return "";
        String prefix = "M121 Android clipboard " + lease + " " + label + " Unicode中文🙂";
        int length = prefix.getBytes(StandardCharsets.UTF_8).length;
        if (bytes < length || bytes > MAXIMUM + 1) throw new IllegalArgumentException("invalid_marker_size");
        StringBuilder value = new StringBuilder(prefix);
        for (int index = length; index < bytes; index++) value.append('x');
        return value.toString();
    }

    private boolean matches(ClipboardManager clipboard, String expected) {
        ClipData clip = clipboard.getPrimaryClip();
        return clip != null && clip.getItemCount() == 1 && clip.getItemAt(0).getText() != null
            && expected.contentEquals(clip.getItemAt(0).getText());
    }

    private JSONObject copy(ClipboardManager clipboard, String value) throws Exception {
        if (!active) throw new IllegalStateException("clipboard_fixture_not_begun");
        ClipData clip = ClipData.newPlainText("miopunch verification", value);
        PersistableBundle extras = new PersistableBundle();
        extras.putString(OWNER, lease);
        extras.putBoolean("android.content.extra.IS_SENSITIVE", true);
        clip.getDescription().setExtras(extras);
        clipboard.setPrimaryClip(clip);
        ClipData actual = clipboard.getPrimaryClip();
        if (actual == null || !matches(clipboard, value)) throw new IllegalStateException("actual_clipboard_copy_failed");
        ownedText = value;
        ownedTimestamp = actual.getDescription().getTimestamp();
        return new JSONObject().put("matches", true).put("bytes", value.getBytes(StandardCharsets.UTF_8).length);
    }

    JSONObject command(String kind, JSONObject request) throws Exception {
        ClipboardManager clipboard = manager();
        if ("clipboard_begin".equals(kind)) {
            if (active) throw new IllegalStateException("clipboard_fixture_already_begun");
            ClipData previous = clipboard.getPrimaryClip();
            if (previous == null && Build.VERSION.SDK_INT < 28) throw new IllegalStateException("empty_snapshot_restore_unsupported");
            if (previous != null) {
                if (previous.getItemCount() != 1 || previous.getItemAt(0).getText() == null
                    || previous.getItemAt(0).getUri() != null || previous.getItemAt(0).getIntent() != null
                    || previous.getItemAt(0).getText().toString().getBytes(StandardCharsets.UTF_8).length > MAXIMUM) {
                    throw new IllegalStateException("prior_clipboard_format_unsupported");
                }
            }
            original = previous;
            active = true;
            return copy(clipboard, marker(request));
        }
        if ("clipboard_copy".equals(kind)) return copy(clipboard, marker(request));
        if ("clipboard_expect".equals(kind)) return new JSONObject().put("matches", matches(clipboard, marker(request)));
        if ("clipboard_restore".equals(kind)) {
            boolean restored = false;
            if (active && ownedText != null) {
                ClipData current = clipboard.getPrimaryClip();
                PersistableBundle extras = current == null ? null : current.getDescription().getExtras();
                if (current != null && extras != null && lease.equals(extras.getString(OWNER))
                    && current.getDescription().getTimestamp() == ownedTimestamp && matches(clipboard, ownedText)) {
                    if (original == null) clipboard.clearPrimaryClip();
                    else clipboard.setPrimaryClip(original);
                    restored = true;
                }
            }
            active = false;
            original = null;
            ownedText = null;
            return new JSONObject().put("restored_if_matching_owned_revision", restored)
                .put("later_revision_preserved", !restored).put("framework_atomic_compare_and_set_available", false)
                .put("original_clipboard_exported", false);
        }
        throw new IllegalArgumentException("unknown_clipboard_fixture_operation");
    }
}
