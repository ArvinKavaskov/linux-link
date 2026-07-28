package org.linuxlink.app

import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import org.json.JSONObject

object LinkBus {
    private val _outgoing = MutableSharedFlow<JSONObject>(
        extraBufferCapacity = 128,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    val outgoing = _outgoing.asSharedFlow()

    fun send(msg: JSONObject) {
        _outgoing.tryEmit(msg)
    }
}
