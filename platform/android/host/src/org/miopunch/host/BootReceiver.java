package org.miopunch.host;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import java.io.File;

/** Default-on restoration uses credential-encrypted state after the user unlocks. */
public final class BootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        if (!Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction())) return;
        if (!context.getSharedPreferences("host", Context.MODE_PRIVATE).getBoolean("restore_after_reboot", true)) return;
        if (!new File(context.getFilesDir(), "node/node.key").isFile()) return;
        context.startForegroundService(new Intent(context, HostService.class).setAction(HostService.OPEN));
    }
}
