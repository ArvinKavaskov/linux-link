package org.linuxlink.app

import android.os.FileObserver
import android.util.Log
import java.io.File

class SyncWatcher(private val onChange: () -> Unit) {
    private val observers = mutableListOf<FileObserver>()

    fun start() {
        if (observers.isNotEmpty()) return
        for (pair in SyncFolder.pairs()) {
            watch(pair.dir, 0)
        }
        Log.i(TAG, "watching ${observers.size} folders for changes")
    }

    fun stop() {
        observers.forEach { runCatching { it.stopWatching() } }
        observers.clear()
    }

    private fun watch(dir: File, depth: Int) {
        if (observers.size >= MAX_WATCHES || !dir.isDirectory) return
        val observer = object : FileObserver(dir, EVENTS) {
            override fun onEvent(event: Int, path: String?) {
                onChange()
            }
        }
        runCatching { observer.startWatching() }
            .onSuccess { observers += observer }
            .onFailure { Log.w(TAG, "cannot watch ${dir.name}: ${it.message}") }
        if (depth >= MAX_DEPTH) return
        dir.listFiles()?.forEach { child ->
            if (child.isDirectory && !child.name.startsWith(".")) {
                watch(child, depth + 1)
            }
        }
    }

    private companion object {
        const val TAG = "SyncWatcher"
        const val MAX_DEPTH = 2
        const val MAX_WATCHES = 200
        const val EVENTS = FileObserver.CREATE or
            FileObserver.DELETE or
            FileObserver.MOVED_TO or
            FileObserver.MOVED_FROM or
            FileObserver.CLOSE_WRITE
    }
}
