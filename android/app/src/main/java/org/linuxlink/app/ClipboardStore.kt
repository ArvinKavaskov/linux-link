package org.linuxlink.app

object ClipboardStore {
    @Volatile
    var lastReceivedFromPc: String? = null
}
