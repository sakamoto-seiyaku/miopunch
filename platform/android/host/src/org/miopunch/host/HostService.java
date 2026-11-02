package org.miopunch.host;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Binder;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.UUID;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.JSONObject;

/** Foreground ownership belongs to the Service; Activities only attach. */
public final class HostService extends Service {
    public static final String OPEN = "org.miopunch.host.OPEN";
    public static final String QUIT = "org.miopunch.host.QUIT";
    private static final String CHANNEL = "miopunch-node";
    public static volatile HostService current;
    public final String generation = UUID.randomUUID().toString();
    private final AtomicBoolean stopping = new AtomicBoolean();
    private final ThreadPoolExecutor workers = new ThreadPoolExecutor(
        2, 2, 0, TimeUnit.MILLISECONDS, new ArrayBlockingQueue<Runnable>(16));
    private final Binder binder = new Binder();
    private PowerManager.WakeLock wakeLock;
    private volatile boolean running;

    @Override public void onCreate() {
        super.onCreate();
        current = this;
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, "Device connection", NotificationManager.IMPORTANCE_LOW));
        Intent open = new Intent(this, VerificationActivity.class);
        Intent quit = new Intent(this, HostService.class).setAction(QUIT);
        Notification notification = new Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("miopunch device connection")
            .setContentText("The node stays active when its window is closed")
            .setOngoing(true)
            .setContentIntent(PendingIntent.getActivity(this, 1, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT))
            .addAction(new Notification.Action.Builder(null, "Quit", PendingIntent.getService(this, 2, quit, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT)).build())
            .build();
        if (Build.VERSION.SDK_INT >= 29) startForeground(121, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
        else startForeground(121, notification);
        wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "miopunch:node");
        wakeLock.setReferenceCounted(false);
    }

    @Override public int onStartCommand(Intent intent, int flags, int id) {
        if (intent != null && QUIT.equals(intent.getAction())) { quit(); return START_NOT_STICKY; }
        if (stopping.get()) return START_NOT_STICKY;
        workers.execute(() -> {
            try {
                JSONObject initialized = new JSONObject(NativeHost.initialize(getApplicationContext()));
                if (!initialized.optBoolean("ok")) throw new IllegalStateException("initialization failed");
                synchronized (this) {
                    if (!running && new File(getFilesDir(), "node/node.key").isFile()) {
                        startNode(new JSONObject().put("action", "open"), null);
                    }
                }
            } catch (Exception failure) { writeState("startup_failed"); }
        });
        return START_STICKY;
    }

    /** The app's trusted command adapter uses these same methods in any Activity. */
    public synchronized JSONObject startNode(JSONObject mode, JSONObject resources) throws Exception {
        if (stopping.get()) throw new IllegalStateException("host is stopping");
        JSONObject initialized = new JSONObject(NativeHost.initialize(getApplicationContext()));
        if (!initialized.optBoolean("ok")) return initialized;
        JSONObject result = new JSONObject(NativeHost.start(new File(getFilesDir(), "node").getAbsolutePath(), mode.toString(), resources == null ? "null" : resources.toString()));
        if (result.optBoolean("ok")) {
            running = true;
            wakeLock.acquire();
            writeState("running");
        }
        return result;
    }

    public JSONObject request(JSONObject input) throws Exception {
        if (stopping.get()) throw new IllegalStateException("host is stopping");
        if ("stop".equals(input.optString("op"))) throw new IllegalArgumentException("use the Service quit operation");
        return new JSONObject(NativeHost.request(input.toString()));
    }

    public JSONObject snapshot() throws Exception {
        JSONObject value = new JSONObject().put("service_generation", generation).put("running", running)
            .put("stopping", stopping.get()).put("wake_lock_held", wakeLock != null && wakeLock.isHeld())
            .put("restore_after_reboot", getSharedPreferences("host", MODE_PRIVATE).getBoolean("restore_after_reboot", true))
            .put("pid", android.os.Process.myPid()).put("boot_elapsed_ms", android.os.SystemClock.elapsedRealtime());
        if (running) {
            // The boot Service has no Activity or test mailbox. Its private
            // journal records current public node/address observations so a
            // real peer can verify restoration before any window is opened.
            JSONObject status = new JSONObject(NativeHost.request("{\"id\":0,\"op\":\"status\"}"));
            JSONObject address = new JSONObject(NativeHost.request("{\"id\":0,\"op\":\"address\"}"));
            if (status.optBoolean("ok")) value.put("node", status.get("result"));
            if (address.optBoolean("ok")) value.put("address", address.get("result"));
        }
        return value;
    }

    public void quit() {
        if (!stopping.compareAndSet(false, true)) return;
        // Stop is independent of the bounded normal command workers, so a held
        // command cannot prevent the native owner from cancelling its work.
        new Thread(() -> {
          synchronized (this) {
            boolean closed = false;
            try { closed = new JSONObject(NativeHost.stop()).optBoolean("ok"); }
            catch (Exception failure) { /* recorded below without secret input */ }
            if (!closed) { writeState("shutdown_failed"); return; }
            running = false;
            if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
            writeState("stopped");
            workers.shutdown();
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
          }
        }, "miopunch-stop").start();
    }

    private synchronized void writeState(String state) {
        try {
            JSONObject value = snapshot().put("state", state);
            File temporary = new File(getFilesDir(), "host-state.pending");
            Files.write(temporary.toPath(), value.toString().getBytes(StandardCharsets.UTF_8));
            Files.move(temporary.toPath(), new File(getFilesDir(), "host-state.json").toPath(), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
        } catch (Exception ignored) { /* operational status only; never a saved-node claim */ }
    }

    @Override public IBinder onBind(Intent intent) { return binder; }
    @Override public void onDestroy() {
        if (current == this) current = null;
        if (!stopping.get()) quit();
        super.onDestroy();
    }
}
