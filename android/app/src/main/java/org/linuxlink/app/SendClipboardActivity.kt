package org.linuxlink.app

import android.app.Activity
import android.content.ClipboardManager
import android.os.Bundle
import android.widget.Toast
import org.json.JSONObject

class SendClipboardActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) return

        val cm = getSystemService(ClipboardManager::class.java)
        val text = cm.primaryClip?.getItemAt(0)?.coerceToText(this)?.toString()

        when {
            text.isNullOrEmpty() ->
                Toast.makeText(this, "Clipboard empty", Toast.LENGTH_SHORT).show()
            text == ClipboardStore.lastReceivedFromPc ->
                Toast.makeText(this, "Already synced with the PC", Toast.LENGTH_SHORT).show()
            else -> {
                LinkBus.send(JSONObject().apply {
                    put("type", "clipboard")
                    put("text", text)
                })
                Toast.makeText(this, "Clipboard sent to PC 📋", Toast.LENGTH_SHORT).show()
            }
        }
        finish()
    }
}
