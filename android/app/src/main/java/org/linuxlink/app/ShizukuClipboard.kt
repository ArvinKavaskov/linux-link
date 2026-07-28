package org.linuxlink.app

import android.content.ClipData
import android.content.Context
import android.content.pm.PackageManager
import android.os.IBinder
import android.util.Log
import rikka.shizuku.Shizuku
import rikka.shizuku.ShizukuBinderWrapper
import rikka.shizuku.SystemServiceHelper

object ShizukuClipboard {

    private const val TAG = "ShizukuClipboard"
    const val PERMISSION_REQUEST_CODE = 4212

    fun isRunning(): Boolean = try {
        Shizuku.pingBinder()
    } catch (_: Throwable) {
        false
    }

    fun hasPermission(): Boolean = try {
        isRunning() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
    } catch (_: Throwable) {
        false
    }

    fun requestPermission() {
        runCatching { Shizuku.requestPermission(PERMISSION_REQUEST_CODE) }
    }

    fun ready(): Boolean = hasPermission()

    @Volatile
    private var iClipboard: Any? = null

    private fun clipboardService(): Any? {
        iClipboard?.let { return it }
        return runCatching {
            val binder = ShizukuBinderWrapper(SystemServiceHelper.getSystemService("clipboard"))
            val stub = Class.forName("android.content.IClipboard\$Stub")
            val svc = stub.getMethod("asInterface", IBinder::class.java).invoke(null, binder)
            iClipboard = svc
            svc
        }.getOrElse {
            Log.e(TAG, "unable to bind to clipboard service", it)
            null
        }
    }

    fun readText(context: Context): String? {
        val svc = clipboardService() ?: return null
        val method = svc.javaClass.methods.firstOrNull { it.name == "getPrimaryClip" } ?: run {
            Log.e(TAG, "getPrimaryClip not found on this Android version")
            return null
        }
        var firstString = true
        val args = method.parameterTypes.map { t ->
            when {
                t == String::class.java && firstString -> { firstString = false; SHELL_PKG }
                t == String::class.java -> null
                t == Int::class.javaPrimitiveType -> 0
                else -> null
            }
        }.toTypedArray()

        return runCatching {
            val clip = method.invoke(svc, *args) as? ClipData ?: return null
            if (clip.itemCount == 0) return null
            clip.getItemAt(0).coerceToText(context)?.toString()
        }.getOrElse {
            Log.e(TAG, "getPrimaryClip failed", it)
            iClipboard = null
            null
        }
    }

    private const val SHELL_PKG = "com.android.shell"
}
