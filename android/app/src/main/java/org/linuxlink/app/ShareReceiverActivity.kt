package org.linuxlink.app

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.util.Patterns
import android.widget.Toast
import org.json.JSONObject

class ShareReceiverActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val fileUris: ArrayList<android.net.Uri> = when (intent?.action) {
            Intent.ACTION_SEND ->
                intent.getParcelableExtra<android.net.Uri>(Intent.EXTRA_STREAM)
                    ?.let { arrayListOf(it) } ?: arrayListOf()
            Intent.ACTION_SEND_MULTIPLE ->
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM) ?: arrayListOf()
            else -> arrayListOf()
        }
        if (fileUris.isNotEmpty()) {
            val svc = Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_SEND_FILE)
                .putParcelableArrayListExtra(LinkForegroundService.EXTRA_FILE_URIS, fileUris)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            val clip = android.content.ClipData.newUri(contentResolver, "files", fileUris[0])
            for (i in 1 until fileUris.size) {
                clip.addItem(android.content.ClipData.Item(fileUris[i]))
            }
            svc.clipData = clip
            startForegroundService(svc)

            val msg = if (fileUris.size == 1) "Sending file to PC… 📎"
            else "Sending ${fileUris.size} files to PC… 📎"
            Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
            finish()
            return
        }

        val text = if (intent?.action == Intent.ACTION_SEND)
            intent.getStringExtra(Intent.EXTRA_TEXT) else null
        val title = intent?.getStringExtra(Intent.EXTRA_SUBJECT).orEmpty()

        if (text.isNullOrBlank()) {
            Toast.makeText(this, "Nothing to send", Toast.LENGTH_SHORT).show()
            finish()
            return
        }

        val url = extractUrl(text)
        if (url != null) {
            LinkBus.send(JSONObject().apply {
                put("type", "handoff")
                put("url", url)
                put("title", title.ifBlank { text.substringBefore('\n').take(80) })
            })
            Toast.makeText(this, "Opening on PC… ↗", Toast.LENGTH_SHORT).show()
        } else {
            LinkBus.send(JSONObject().apply {
                put("type", "clipboard")
                put("text", text)
            })
            Toast.makeText(this, "Sent to PC clipboard 📋", Toast.LENGTH_SHORT).show()
        }
        finish()
    }

    private fun extractUrl(text: String): String? {
        val matcher = Patterns.WEB_URL.matcher(text)
        return if (matcher.find()) text.substring(matcher.start(), matcher.end()) else null
    }
}
