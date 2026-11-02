package org.miopunch.host;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Minimal lifecycle surface for the real APK; not the product's shared UI. */
public final class VerificationActivity extends Activity {
    private static final java.util.concurrent.atomic.AtomicLong nextWindow = new java.util.concurrent.atomic.AtomicLong();
    private final long clipboardWindow = nextWindow.incrementAndGet();
    public static volatile VerificationActivity current;
    public static final java.util.concurrent.atomic.AtomicInteger created = new java.util.concurrent.atomic.AtomicInteger();
    public static final java.util.concurrent.atomic.AtomicInteger destroyed = new java.util.concurrent.atomic.AtomicInteger();
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        current = this;
        created.incrementAndGet();
        startForegroundService(new Intent(this, HostService.class).setAction(HostService.OPEN));
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        TextView status = new TextView(this);
        status.setText("miopunch host verification\nClosing this Activity keeps the Service running.");
        content.addView(status);
        CheckBox restore = new CheckBox(this);
        restore.setText("Restore connection after reboot");
        restore.setChecked(getSharedPreferences("host", MODE_PRIVATE).getBoolean("restore_after_reboot", true));
        restore.setOnCheckedChangeListener((button, enabled) -> getSharedPreferences("host", MODE_PRIVATE).edit().putBoolean("restore_after_reboot", enabled).apply());
        content.addView(restore);
        Button close = new Button(this);
        close.setText("Close window"); close.setOnClickListener(view -> finish()); content.addView(close);
        Button quit = new Button(this);
        quit.setText("Quit"); quit.setOnClickListener(view -> { startService(new Intent(this, HostService.class).setAction(HostService.QUIT)); finish(); }); content.addView(quit);
        setContentView(content);
    }
    @Override public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        NativeHost.clipboardFocus(clipboardWindow, focused && current == this && !isFinishing());
    }
    @Override public void onPause() {
        NativeHost.clipboardFocus(clipboardWindow, false);
        super.onPause();
    }
    @Override public void onDestroy() {
        NativeHost.clipboardFocus(clipboardWindow, false);
        if (current == this) current = null;
        destroyed.incrementAndGet();
        super.onDestroy();
    }
}
