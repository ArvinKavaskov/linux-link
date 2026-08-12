package org.linuxlink.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

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
