package org.linuxlink.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Brings the link back up on its own after a phone reboot.
 *
 * Without this the user had to open the app once after every restart, which is
 * exactly the "you have to reconnect" feeling the whole v3 effort is about.
 * BOOT_COMPLETED is one of the few broadcasts still allowed to start a
 * foreground service from the background, so this is legal on Android 15.
 *
 * MY_PACKAGE_REPLACED is here too: updating the app kills the service, and
 * without this the link would stay down until the next manual launch.
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED -> Unit
            else -> return
        }
        if (PairedPc.load(context) == null) {
            Log.i(TAG, "boot: no PC paired, staying quiet")
            return
        }
        val service = Intent(context, LinkForegroundService::class.java)
            .setAction(LinkForegroundService.ACTION_CONNECT)
        runCatching { context.startForegroundService(service) }
            .onFailure { Log.w(TAG, "could not start on boot: ${it.message}") }
        Log.i(TAG, "boot: reconnecting to the PC")
    }

    private companion object {
        const val TAG = "BootReceiver"
    }
}
