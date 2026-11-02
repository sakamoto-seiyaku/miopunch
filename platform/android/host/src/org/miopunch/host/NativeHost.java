package org.miopunch.host;

import android.content.Context;

/** Trusted application bridge. Call on worker threads, never the UI thread. */
public final class NativeHost {
    static { System.loadLibrary("miopunch_native_probe"); }
    private NativeHost() {}
    public static native String initialize(Context applicationContext);
    public static native String start(String directory, String mode, String resources);
    public static native String request(String json);
    public static native String stop();
    /** Nonblocking focus notification; performs no SDK work or clipboard read. */
    public static native void clipboardFocus(long window, boolean focused);
}
