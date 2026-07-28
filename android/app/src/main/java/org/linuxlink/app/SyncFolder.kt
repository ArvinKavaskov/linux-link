package org.linuxlink.app

import android.content.Context
import android.os.Environment
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

object SyncFolder {
    private const val PREFS = "sync_prefs"

    data class Pair(val id: String, val dir: File)

    fun pairs(): List<Pair> {
        val ext = Environment.getExternalStorageDirectory()
        return listOf(
            Pair("LinuxLink", File(ext, "LinuxLink")),
            Pair("Download", File(ext, "Download")),
            Pair("Documents", File(ext, "Documents")),
            Pair("Pictures", File(ext, "Pictures")),
        )
    }

    fun ensureDirs() {
        pairs().forEach { it.dir.mkdirs() }
    }

    fun isEnabled(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean("enabled", false)

    fun setEnabled(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean("enabled", on).apply()
    }

    fun hasAllFilesAccess(): Boolean =
        android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.R ||
            Environment.isExternalStorageManager()

    fun scan(root: File): JSONArray {
        val arr = JSONArray()
        fun walk(dir: File) {
            dir.listFiles()?.forEach { f ->
                if (f.isDirectory) walk(f)
                else if (f.isFile) {
                    val rel = f.relativeTo(root).path.replace('\\', '/')
                    arr.put(JSONObject().apply {
                        put("path", rel); put("size", f.length()); put("mtime", f.lastModified())
                    })
                }
            }
        }
        walk(root)
        return arr
    }
}
