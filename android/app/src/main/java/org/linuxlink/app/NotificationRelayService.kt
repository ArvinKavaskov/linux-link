package org.linuxlink.app

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

class NotificationRelayService : NotificationListenerService() {

    override fun onListenerConnected() {
        instance = this
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (!shouldRelay(sbn)) return
        val extras = sbn.notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val body = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
        if (title.isEmpty() && body.isEmpty()) return

        val replyAction = findReplyAction(sbn.notification)
        if (replyAction != null) replyActions[sbn.key] = replyAction

        LinkBus.send(JSONObject().apply {
            put("type", "notification")
            put("key", sbn.key)
            put("app", appLabel(sbn.packageName))
            put("title", title)
            put("body", body)
            put("can_reply", replyAction != null)
        })
        Log.d(TAG, "→ ${sbn.packageName} : $title (reply=${replyAction != null})")
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        replyActions.remove(sbn.key)
        if (!shouldRelay(sbn)) return
        LinkBus.send(JSONObject().apply {
            put("type", "notification_dismissed")
            put("key", sbn.key)
        })
    }

    private fun shouldRelay(sbn: StatusBarNotification): Boolean {
        if (sbn.packageName == packageName) return false
        if (!sbn.isClearable) return false
        if (sbn.notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return false
        return true
    }

    private fun findReplyAction(n: Notification): Notification.Action? {
        val actions = n.actions ?: return null
        return actions.firstOrNull { a ->
            a.remoteInputs?.any { it.resultKey != null } == true
        }
    }

    private fun appLabel(pkg: String): String = try {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    } catch (_: PackageManager.NameNotFoundException) {
        pkg
    }

    companion object {
        private const val TAG = "NotificationRelay"

        @Volatile
        private var instance: NotificationRelayService? = null

        private val replyActions = ConcurrentHashMap<String, Notification.Action>()

        fun sendReply(key: String, text: String): Boolean {
            val service = instance ?: run {
                Log.w(TAG, "service not connected, reply ignored")
                return false
            }
            val action = replyActions[key] ?: run {
                Log.w(TAG, "no reply action for $key")
                return false
            }
            val remoteInputs = action.remoteInputs ?: return false
            return try {
                val intent = Intent()
                val bundle = Bundle()
                remoteInputs.forEach { ri -> bundle.putCharSequence(ri.resultKey, text) }
                RemoteInput.addResultsToIntent(remoteInputs, intent, bundle)
                action.actionIntent.send(service, 0, intent)
                Log.i(TAG, "↩ reply sent through the app for $key")
                true
            } catch (e: PendingIntent.CanceledException) {
                Log.e(TAG, "PendingIntent canceled for $key", e)
                false
            }
        }
    }
}
