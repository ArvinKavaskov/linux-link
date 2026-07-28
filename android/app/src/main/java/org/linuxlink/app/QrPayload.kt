package org.linuxlink.app

import org.json.JSONObject

data class QrPayload(
    val version: Int,
    val name: String,
    val addrs: List<String>,
    val port: Int,
    val fingerprint: String,
    val token: String,
) {
    companion object {
        fun parse(raw: String): QrPayload {
            val o = JSONObject(raw)
            val addrs = buildList {
                val a = o.getJSONArray("addrs")
                for (i in 0 until a.length()) add(a.getString(i))
            }
            return QrPayload(
                version = o.getInt("v"),
                name = o.getString("name"),
                addrs = addrs,
                port = o.getInt("port"),
                fingerprint = o.getString("fp"),
                token = o.getString("token"),
            )
        }
    }
}
