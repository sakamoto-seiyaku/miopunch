package org.miopunch.host;

import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import org.json.JSONObject;

/** Debug APK only: private app-UID mailbox, not an exported management endpoint. */
public final class VerifyInstrumentation extends Instrumentation {
    private File directory;
    private final String lease = java.util.UUID.randomUUID().toString();
    private long lastRequest;
    private ClipboardFixture clipboard;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private void save(String name, JSONObject value) throws Exception {
        byte[] data = value.toString().getBytes(StandardCharsets.UTF_8);
        if (data.length > 262144) throw new IllegalStateException("bounded verification result exceeded");
        File pending = new File(directory, name + ".pending");
        try (FileOutputStream file = new FileOutputStream(pending)) { file.write(data); file.getFD().sync(); }
        Files.move(pending.toPath(), new File(directory, name).toPath(), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
    }
    private void show() {
        startActivitySync(new Intent(getTargetContext(), VerificationActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
    }
    private JSONObject view() throws Exception {
        JSONObject result = new JSONObject().put("activities_created", VerificationActivity.created.get())
            .put("activities_destroyed", VerificationActivity.destroyed.get());
        boolean[] visible = new boolean[2];
        runOnMainSync(() -> { VerificationActivity activity = VerificationActivity.current; visible[0] = activity != null; visible[1] = activity != null && activity.hasWindowFocus(); });
        result.put("activity_exists", visible[0]).put("window_focused", visible[1]);
        HostService service = HostService.current;
        return result.put("service", service == null ? JSONObject.NULL : service.snapshot());
    }
    private JSONObject clipboardCommand(String kind, JSONObject input) throws Exception {
        if ("clipboard_restore".equals(kind)) {
            HostService service = HostService.current;
            if (service != null && service.snapshot().optBoolean("running")) {
                JSONObject state = service.request(new JSONObject().put("id",0).put("op","clipboard_status"));
                if (!state.optBoolean("ok") || state.getJSONObject("result").optBoolean("enabled")) {
                    throw new IllegalStateException("disable_sync_before_fixture_restore");
                }
            }
        }
        JSONObject[] result = new JSONObject[1];
        Exception[] error = new Exception[1];
        runOnMainSync(() -> {
            try { result[0] = clipboard.command(kind, input); }
            catch (Exception failure) { error[0] = failure; }
        });
        if (error[0] != null) throw error[0];
        return result[0];
    }
    @Override public void onStart() {
        directory = getTargetContext().getFilesDir();
        clipboard = new ClipboardFixture(getTargetContext(), lease);
        long deadline = SystemClock.elapsedRealtime() + 600000;
        boolean finished = false;
        try {
            show();
            while (HostService.current == null && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(20);
            if (HostService.current == null) throw new IllegalStateException("Service did not start");
            save("verify-ready.json", new JSONObject().put("ready",true).put("files",directory.getAbsolutePath()).put("pid",android.os.Process.myPid()).put("lease",lease));
            File command = new File(directory,"verify-request.json");
            while (!finished && SystemClock.elapsedRealtime() < deadline) {
                if (!command.exists()) { SystemClock.sleep(25); continue; }
                JSONObject result = new JSONObject().put("lease",lease);
                try {
                    if (command.length()>65536) throw new IllegalArgumentException("request exceeds bound");
                    JSONObject input = new JSONObject(new String(Files.readAllBytes(command.toPath()),StandardCharsets.UTF_8));
                    if (!command.delete()) throw new IllegalStateException("request removal failed");
                    result.put("id",input.getLong("id"));
                    if (!lease.equals(input.getString("lease")) || input.getLong("id") <= lastRequest) throw new IllegalArgumentException("stale verification request");
                    lastRequest = input.getLong("id");
                    String kind=input.getString("kind");
                    HostService service=HostService.current;
                    if ("start".equals(kind)) result.put("result",service.startNode(input.getJSONObject("mode"),input.optJSONObject("resources")));
                    else if ("request".equals(kind)) result.put("result",service.request(input.getJSONObject("request")));
                    else if ("snapshot".equals(kind)) result.put("result",view());
                    else if (kind.startsWith("clipboard_")) result.put("result",clipboardCommand(kind,input));
                    else if ("close_window".equals(kind)) {
                        runOnMainSync(() -> { if (VerificationActivity.current!=null) VerificationActivity.current.finish(); });
                        waitForIdleSync(); result.put("result",view());
                    } else if ("show_window".equals(kind)) { show(); result.put("result",view()); }
                    else if ("reboot_restore".equals(kind)) {
                        boolean enabled=input.getBoolean("enabled");
                        boolean saved=getTargetContext().getSharedPreferences("host",Context.MODE_PRIVATE).edit().putBoolean("restore_after_reboot",enabled).commit();
                        result.put("result",new JSONObject().put("saved",saved).put("enabled",enabled));
                    } else if ("quit".equals(kind) || "finish".equals(kind)) {
                        if (clipboard.active()) {
                            if (!view().getBoolean("window_focused")) show();
                            long focusDeadline = SystemClock.elapsedRealtime() + 10000;
                            while (!view().getBoolean("window_focused") && SystemClock.elapsedRealtime() < focusDeadline) SystemClock.sleep(25);
                            service = HostService.current;
                        }
                        if (service!=null) service.quit();
                        long stopDeadline=SystemClock.elapsedRealtime()+20000;
                        while (HostService.current!=null && SystemClock.elapsedRealtime()<stopDeadline) SystemClock.sleep(25);
                        if (HostService.current!=null) throw new IllegalStateException("Service quit not completed");
                        if (clipboard.active()) result.put("clipboard_cleanup",clipboardCommand("clipboard_restore",new JSONObject()));
                        result.put("result",view()); finished="finish".equals(kind);
                    } else throw new IllegalArgumentException("unknown verification operation");
                    result.put("ok",true);
                } catch (Exception failure) {
                    result.put("ok",false).put("error",failure.getClass().getSimpleName());
                }
                save("verify-result.json",result);
            }
            if (!finished) throw new IllegalStateException("verification lease expired");
            runOnMainSync(() -> { if (VerificationActivity.current!=null) VerificationActivity.current.finish(); });
            Bundle result=new Bundle();result.putString("status","owned_host_stopped");finish(0,result);
        } catch (Exception failure) {
            HostService service=HostService.current;if(service!=null)service.quit();
            Bundle result=new Bundle();result.putString("failure",failure.getClass().getSimpleName());finish(1,result);
        }
    }
}
