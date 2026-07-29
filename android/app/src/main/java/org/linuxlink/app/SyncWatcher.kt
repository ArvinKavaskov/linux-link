package org.linuxlink.app

import android.os.FileObserver
import android.util.Log
import java.io.File

/**
 * Tells the sync loop when the phone's files have actually changed.
 *
 * v2 walked LinuxLink, Download, Documents and Pictures — recursively, every
 * five minutes, forever. On a phone with a few thousand photos that is tens of
 * thousands of `stat` calls an hour, most of them while the screen is off, all
 * of them to conclude that nothing had happened. The kernel already knows when
 * a file appears: inotify, which Android exposes as [FileObserver].
 *
 * Two deliberate limits:
 *
 *  * [MAX_DEPTH] — we watch the sync roots and their first two levels of
 *    subdirectories. A change deeper than that is still caught by the sync
 *    loop's safety pass; watching an entire photo library would cost thousands
 *    of inotify watches for no real benefit.
 *  * [MAX_WATCHES] — a hard ceiling. inotify watches are a finite kernel
 *    resource shared with every other app on the phone, and running the phone
 *    out of them would break far more than Linux Link.
 */
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
                // Anything at all: the sync loop debounces and works out what
                // actually differs. Note this fires on a binder thread, so it
                // must do nothing but signal.
                onChange()
            }
        }
        runCatching { observer.startWatching() }
            .onSuccess { observers += observer }
            .onFailure { Log.w(TAG, "cannot watch ${dir.name}: ${it.message}") }
        if (depth >= MAX_DEPTH) return
        dir.listFiles()?.forEach { child ->
            // Skip the caches and thumbnail dirs that churn constantly and are
            // never worth syncing.
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
