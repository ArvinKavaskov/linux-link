package org.linuxlink.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

object KnownPcs {
    private const val PREFS = "known_pcs"
    private const val KEY = "pcs"

    fun list(context: Context): List<PairedPc> {
        val raw = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(KEY, null) ?: return emptyList()
        return runCatching {
            val arr = JSONArray(raw)
            (0 until arr.length()).mapNotNull { i ->
                val o = arr.optJSONObject(i) ?: return@mapNotNull null
                val name = o.optString("name")
                val fp = o.optString("fp")
                if (name.isEmpty() || fp.isEmpty()) return@mapNotNull null
                PairedPc(
                    name = name,
                    lastAddress = o.optString("addr"),
                    port = o.optInt("port", 47100),
                    fingerprint = fp,
                )
            }
        }.getOrElse { emptyList() }
    }

    fun remember(context: Context, pc: PairedPc) {
        val merged = list(context)
            .filterNot { it.fingerprint.equals(pc.fingerprint, ignoreCase = true) } + pc
        save(context, merged)
    }

    fun forget(context: Context, fingerprint: String) {
        save(context, list(context).filterNot {
            it.fingerprint.equals(fingerprint, ignoreCase = true)
        })
    }

    private fun save(context: Context, pcs: List<PairedPc>) {
        val arr = JSONArray()
        for (pc in pcs) {
            arr.put(
                JSONObject()
                    .put("name", pc.name)
                    .put("addr", pc.lastAddress)
                    .put("port", pc.port)
                    .put("fp", pc.fingerprint)
            )
        }
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(KEY, arr.toString())
            .apply()
    }
}
