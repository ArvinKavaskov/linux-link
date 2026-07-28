package org.linuxlink.app

import android.content.ClipboardManager
import android.content.Context
import android.graphics.PixelFormat
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import org.json.JSONObject

class ClipboardAutoSync(private val context: Context) {

    private val windowManager =
        context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
    private val clipboard =
        context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private var overlay: View? = null
    private var listener: ClipboardManager.OnPrimaryClipChangedListener? = null

    private val poller = Handler(Looper.getMainLooper())
    private var lastSeen: String? = null
    private val pollTask = object : Runnable {
        override fun run() {
            onClipChanged()
            poller.postDelayed(this, POLL_MS)
        }
    }

    fun isSupported(): Boolean = ShizukuClipboard.ready() || canDrawOverlays()

    fun start() {
        if (listener != null) return

        val shizuku = ShizukuClipboard.ready()
        if (!shizuku && !canDrawOverlays()) {
            Log.w(TAG, "Neither Shizuku nor SYSTEM_ALERT_WINDOW — automatic mode unavailable")
            return
        }
        if (!shizuku) addOverlay()

        val l = ClipboardManager.OnPrimaryClipChangedListener { onClipChanged() }
        clipboard.addPrimaryClipChangedListener(l)
        listener = l
        poller.postDelayed(pollTask, POLL_MS)
        Log.i(TAG, "Automatic clipboard active (${if (shizuku) "Shizuku" else "overlay"} + polling ${POLL_MS}ms)")
    }

    private fun addOverlay() {
        if (overlay != null) return
        val params = WindowManager.LayoutParams(
            1, 1,
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O)
                WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
            else
                @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.TOP or Gravity.START }

        val view = View(context)
        runCatching { windowManager.addView(view, params) }
            .onSuccess { overlay = view }
            .onFailure { Log.e(TAG, "unable to add overlay", it) }
    }

    fun stop() {
        poller.removeCallbacks(pollTask)
        listener?.let { clipboard.removePrimaryClipChangedListener(it) }
        listener = null
        overlay?.let { runCatching { windowManager.removeView(it) } }
        overlay = null
    }

    private fun onClipChanged() {
        val text = if (ShizukuClipboard.ready())
            ShizukuClipboard.readText(context)
        else
            clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()
        if (text.isNullOrEmpty()) return
        if (text == lastSeen) return
        if (text == ClipboardStore.lastReceivedFromPc) {
            lastSeen = text
            return
        }
        lastSeen = text
        LinkBus.send(JSONObject().apply {
            put("type", "clipboard")
            put("text", text)
        })
        Log.d(TAG, "📋 copy detected (${text.length} characters) → PC")
    }

    private fun canDrawOverlays(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    companion object {
        private const val TAG = "ClipboardAutoSync"
        private const val POLL_MS = 1500L

        fun isEnabled(context: Context): Boolean =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getBoolean("auto_clipboard", false)

        fun setEnabled(context: Context, enabled: Boolean) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .edit().putBoolean("auto_clipboard", enabled).apply()
        }

        private const val PREFS = "clipboard_prefs"
    }
}
