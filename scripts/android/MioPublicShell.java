/** Foreground shell for physical-device integration; no product UI/background claim. */
public final class MioPublicShell {
    private static native int session(Object context, String directory, String bindIp);

    public static void main(String[] args) throws Exception {
        if (args.length != 3) {
            throw new IllegalArgumentException("shared-library data-directory bind-ip");
        }
        Class.forName("android.os.Looper").getMethod("prepareMainLooper").invoke(null);
        Class<?> activityThread = Class.forName("android.app.ActivityThread");
        Object thread = activityThread.getMethod("systemMain").invoke(null);
        Object context = activityThread.getMethod("getSystemContext").invoke(thread);
        System.load(args[0]);
        System.exit(session(context, args[1], args[2]));
    }
}
