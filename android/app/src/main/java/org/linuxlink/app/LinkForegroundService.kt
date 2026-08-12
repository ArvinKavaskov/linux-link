package org.linuxlink.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelChildren
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

class LinkForegroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var connectionJob: Job? = null
    private var client: LinkClient? = null
    private var autoClipboard: ClipboardAutoSync? = null
    private var mediaSession: LinkMediaSession? = null
    private var batteryReceiver: android.content.BroadcastReceiver? = null
    private var dndReceiver: android.content.BroadcastReceiver? = null
    private var netMonitor: NetworkMonitor? = null
    private var syncWatcher: SyncWatcher? = null
    private val syncWakeups = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var dndFromPc = false
    @Volatile private var connected = false
    @Volatile private var bleAway = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DISCONNECT -> {
                disconnect()
                stopSelf()
            }
            ACTION_BLE_GONE -> {
                bleAway = true
                if (connected) {
                    Log.i(TAG, "out of BLE range but the Wi-Fi link is up — staying connected")
                } else {
                    Log.i(TAG, "out of BLE range and no link — standing down")
                    disconnect()
                    stopSelf()
                }
            }
            ACTION_RECONNECT -> {
                disconnect()
                startAsForeground("Switching PC…")
                connect()
                netMonitor?.poke()
                startAutoClipboardIfEnabled()
            }
            ACTION_MEDIA_PREV -> sendPcMedia("previous")
            ACTION_MEDIA_PLAYPAUSE -> sendPcMedia("play_pause")
            ACTION_MEDIA_NEXT -> sendPcMedia("next")
            ACTION_SEND_FILE -> {
                val uris = intent.getParcelableArrayListExtra<android.net.Uri>(EXTRA_FILE_URIS)
                if (!uris.isNullOrEmpty()) sendFilesToPc(uris)
            }
            else -> {
                bleAway = false
                startAsForeground("Connecting to the PC…")
                connect()
                netMonitor?.poke()
                startAutoClipboardIfEnabled()
            }
        }
        return START_STICKY
    }

    private fun connect() {
        if (connectionJob?.isActive == true) return
        var pc = PairedPc.load(this) ?: run {
            Log.w(TAG, "No PC paired — nothing to do")
            stopSelf()
            return
        }
        val monitor = netMonitor ?: NetworkMonitor(this).also { it.start(); netMonitor = it }
        connectionJob = scope.launch {
            val identity = Identity.loadOrCreate(this@LinkForegroundService)
            val c = LinkClient(identity).also { it.expectedFingerprint = pc.fingerprint }
            client = c
            activeClient = c
            var attempt = 0
            while (isActive) {
                try {
                    val (address, pcName) = openConnection(c, pc)
                    if (address != pc.lastAddress) {
                        pc = pc.copy(lastAddress = address)
                        PairedPc.save(this@LinkForegroundService, pc)
                        Log.i(TAG, "PC address updated → $address")
                    }
                    connected = true
                    linkUp = true
                    KnownPcs.remember(this@LinkForegroundService, pc.copy(name = pcName))
                    updateNotification("Connected to $pcName")
                    startMediaSession()
                    startBatteryReporting()
                    startDndSync()
                    attempt = 0
                    coroutineScope {
                        launch {
                            var seq = 0L
                            while (isActive) {
                                monitor.wakeups.receive()
                                val rtt = withTimeoutOrNull(4_000) { c.ping(seq++) }
                                    ?: throw java.io.IOException("health check timed out")
                                Log.d(TAG, "health check rtt=${rtt}ms")
                            }
                        }
                        launch { syncLoop(c) }
                        launch {
                            c.subscribe { msg -> handlePush(msg) }
                        }
                        LinkBus.outgoing.collect { msg -> c.sendRaw(msg) }
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "connection lost: ${e.message}")
                    connected = false
                    linkUp = false
                    c.close()
                    attempt++
                    if (attempt == 2) {
                        val others = KnownPcs.list(this@LinkForegroundService)
                            .filterNot { it.fingerprint.equals(pc.fingerprint, ignoreCase = true) }
                        PcLocator.discoverAny(this@LinkForegroundService, others)?.let { found ->
                            Log.i(TAG, "${pc.name} absent but ${found.name} is here — switching")
                            pc = found
                            PairedPc.save(this@LinkForegroundService, found)
                            c.expectedFingerprint = found.fingerprint
                            updateNotification("Switching to ${found.name}…")
                            attempt = 0
                        }
                    }
                    if (bleAway && attempt >= 4) {
                        Log.i(TAG, "PC out of reach and out of range — standing down")
                        stopSelf()
                        return@launch
                    }
                    val wait = reconnectDelay(attempt)
                    if (attempt > 2) updateNotification("PC unreachable — retrying…")
                    if (wait > 0) waitBeforeRetry(monitor, wait)
                }
            }
        }
    }

    private suspend fun openConnection(c: LinkClient, pc: PairedPc): Pair<String, String> {
        if (pc.lastAddress.isNotEmpty()) {
            try {
                c.connect(pc.lastAddress, pc.port, timeoutMillis = 1_200)
                return pc.lastAddress to c.hello(Build.MODEL)
            } catch (e: Exception) {
                Log.d(TAG, "${pc.lastAddress} did not answer (${e.message}) — looking for the PC")
                c.close()
            }
        }
        val candidates = PcLocator.discover(this, pc)
        for (host in candidates) {
            try {
                c.connect(host, pc.port, timeoutMillis = 2_500)
                return host to c.hello(Build.MODEL)
            } catch (e: Exception) {
                Log.d(TAG, "$host refused: ${e.message}")
                c.close()
            }
        }
        error("PC not found on this network")
    }

    private fun reconnectDelay(attempt: Int): Long = when (attempt) {
        1, 2 -> 0L
        3 -> 300L
        4 -> 1_000L
        5 -> 3_000L
        6 -> 8_000L
        else -> 20_000L
    }

    private suspend fun waitBeforeRetry(monitor: NetworkMonitor, ms: Long) {
        withTimeoutOrNull(ms) { monitor.wakeups.receive() }
    }

    private suspend fun syncLoop(c: LinkClient) {
        if (!SyncFolder.isEnabled(this) || !SyncFolder.hasAllFilesAccess()) return
        startSyncWatcher()
        while (true) {
            for (pair in SyncFolder.pairs()) {
                try { c.runSync(pair.id, pair.dir) }
                catch (e: Exception) { Log.w(TAG, "sync ${pair.id} : ${e.message}") }
            }
            withTimeoutOrNull(2 * 60 * 60_000) { syncWakeups.receive() }
            delay(10_000)
            while (syncWakeups.tryReceive().isSuccess) { /* drain the burst */ }
        }
    }

    private fun startSyncWatcher() {
        if (syncWatcher != null) return
        syncWatcher = SyncWatcher { syncWakeups.trySend(Unit) }.also { it.start() }
    }

    private fun sendFilesToPc(uris: List<android.net.Uri>) {
        val c = client ?: run {
            Log.w(TAG, "not connected, file send ignored")
            notify("File not sent: PC not connected")
            return
        }
        scope.launch {
            var ok = 0
            for ((i, uri) in uris.withIndex()) {
                try {
                    val (name, size) = queryNameSize(uri)
                    notify(
                        if (uris.size == 1) "Sending \"$name\" to the PC…"
                        else "Sending ${i + 1}/${uris.size} : $name…"
                    )
                    val input = contentResolver.openInputStream(uri) ?: error("cannot read")
                    c.sendFile(name, size, input)
                    ok++
                } catch (e: Exception) {
                    Log.e(TAG, "file send", e)
                }
            }
            notify(
                if (uris.size == 1 && ok == 1) "File sent to the PC ✔"
                else "$ok/${uris.size} file(s) sent to the PC ✔"
            )
        }
    }

    private fun receiveFileFromPc(id: String, name: String, size: Long) {
        val c = client ?: return
        scope.launch {
            try {
                notify("Receiving \"$name\"…")
                val values = android.content.ContentValues().apply {
                    put(android.provider.MediaStore.Downloads.DISPLAY_NAME, name)
                    put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
                }
                val resolver = contentResolver
                val collection = android.provider.MediaStore.Downloads
                    .getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
                val uri = resolver.insert(collection, values) ?: error("cannot create")
                val out = resolver.openOutputStream(uri) ?: error("cannot write")
                c.pullFile(id, size, out)
                values.clear()
                values.put(android.provider.MediaStore.Downloads.IS_PENDING, 0)
                resolver.update(uri, values, null, null)
                notifyOpenFile("\"$name\" received ✔", uri, name)
            } catch (e: Exception) {
                Log.e(TAG, "file receive", e)
                notify("Receive failed: ${e.message}")
            }
        }
    }

    private fun receiveClipboardImageFromPc(id: String, size: Long) {
        val c = client ?: return
        scope.launch {
            try {
                val dir = java.io.File(cacheDir, "clipboard").apply { mkdirs() }
                val file = java.io.File(dir, "clip.png")
                c.pullFile(id, size, java.io.FileOutputStream(file))
                val uri = androidx.core.content.FileProvider.getUriForFile(
                    this@LinkForegroundService, "$packageName.fileprovider", file
                )
                val cm = getSystemService(android.content.ClipboardManager::class.java)
                val clip = android.content.ClipData.newUri(contentResolver, "Image", uri)
                cm.setPrimaryClip(clip)
                Log.d(TAG, "🖼 clipboard image from the PC placed (${size} bytes)")
            } catch (e: Exception) {
                Log.e(TAG, "clipboard image receive", e)
            }
        }
    }

    private fun queryNameSize(uri: android.net.Uri): Pair<String, Long> {
        var name = "file"
        var size = 0L
        contentResolver.query(uri, null, null, null, null)?.use { cur ->
            val nameIdx = cur.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
            val sizeIdx = cur.getColumnIndex(android.provider.OpenableColumns.SIZE)
            if (cur.moveToFirst()) {
                if (nameIdx >= 0) name = cur.getString(nameIdx) ?: name
                if (sizeIdx >= 0) size = cur.getLong(sizeIdx)
            }
        }
        return name to size
    }

    private fun notify(text: String) {
        val channelId = "link_transfer"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Transfers", NotificationManager.IMPORTANCE_LOW)
        )
        nm.notify(
            NOTIF_TRANSFER_ID,
            Notification.Builder(this, channelId)
                .setContentTitle("Linux Link").setContentText(text)
                .setSmallIcon(android.R.drawable.stat_sys_upload_done).build()
        )
    }

    private fun notifyOpenFile(text: String, uri: android.net.Uri, name: String) {
        val channelId = "link_transfer"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Transfers", NotificationManager.IMPORTANCE_DEFAULT)
        )
        val view = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, contentResolver.getType(uri) ?: "*/*")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        val pending = android.app.PendingIntent.getActivity(
            this, name.hashCode(), view,
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )
        nm.notify(
            NOTIF_TRANSFER_ID,
            Notification.Builder(this, channelId)
                .setContentTitle("Linux Link").setContentText(text)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentIntent(pending).setAutoCancel(true).build()
        )
    }

    private fun startBatteryReporting() {
        if (batteryReceiver != null) return
        val receiver = object : android.content.BroadcastReceiver() {
            private var lastLevel = -1
            override fun onReceive(ctx: android.content.Context?, intent: Intent?) {
                intent ?: return
                val level = intent.getIntExtra(android.os.BatteryManager.EXTRA_LEVEL, -1)
                val scale = intent.getIntExtra(android.os.BatteryManager.EXTRA_SCALE, 100)
                val status = intent.getIntExtra(android.os.BatteryManager.EXTRA_STATUS, -1)
                if (level < 0) return
                val pct = level * 100 / scale
                val charging = status == android.os.BatteryManager.BATTERY_STATUS_CHARGING ||
                    status == android.os.BatteryManager.BATTERY_STATUS_FULL
                if (pct == lastLevel) return
                lastLevel = pct
                LinkBus.send(org.json.JSONObject().apply {
                    put("type", "battery")
                    put("level", pct)
                    put("charging", charging)
                })
            }
        }
        registerReceiver(receiver, android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        batteryReceiver = receiver
    }

    private fun startDndSync() {
        if (dndReceiver != null) return
        val nm = getSystemService(NotificationManager::class.java)
        if (!nm.isNotificationPolicyAccessGranted) return

        LinkBus.send(org.json.JSONObject().apply {
            put("type", "dnd"); put("on", isDndOn())
        })

        val receiver = object : android.content.BroadcastReceiver() {
            override fun onReceive(c: android.content.Context?, i: Intent?) {
                if (dndFromPc) { dndFromPc = false; return }
                LinkBus.send(org.json.JSONObject().apply {
                    put("type", "dnd"); put("on", isDndOn())
                })
            }
        }
        registerReceiver(
            receiver,
            android.content.IntentFilter(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)
        )
        dndReceiver = receiver
    }

    private fun isDndOn(): Boolean {
        val nm = getSystemService(NotificationManager::class.java)
        return nm.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
    }

    private fun applyDndFromPc(on: Boolean) {
        val nm = getSystemService(NotificationManager::class.java)
        if (!nm.isNotificationPolicyAccessGranted) return
        if (isDndOn() == on) return
        dndFromPc = true
        nm.setInterruptionFilter(
            if (on) NotificationManager.INTERRUPTION_FILTER_PRIORITY
            else NotificationManager.INTERRUPTION_FILTER_ALL
        )
        Log.d(TAG, "🌙 DND applied from the PC: $on")
    }

    private fun sendPcMedia(action: String) {
        LinkBus.send(org.json.JSONObject().apply {
            put("type", "pc_media")
            put("action", action)
        })
    }

    private fun startMediaSession() {
        if (mediaSession != null) return
        android.os.Handler(mainLooper).post {
            val ms = LinkMediaSession(this).also { it.start() }
            mediaSession = ms
            showMediaNotification(ms)
        }
    }

    private fun showMediaNotification(ms: LinkMediaSession) {
        val token = ms.token() ?: return
        val channelId = "link_media"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "PC media", NotificationManager.IMPORTANCE_LOW)
        )
        fun act(action: String, code: Int) = android.app.PendingIntent.getService(
            this, code,
            Intent(this, LinkForegroundService::class.java).setAction(action),
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )
        val playPauseIcon = if (ms.playing)
            android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play
        val notif = Notification.Builder(this, channelId)
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(token)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            .setContentTitle(ms.title)
            .setContentText(ms.artist)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .addAction(android.R.drawable.ic_media_previous, "Previous", act(ACTION_MEDIA_PREV, 10))
            .addAction(playPauseIcon, "Play/Pause", act(ACTION_MEDIA_PLAYPAUSE, 11))
            .addAction(android.R.drawable.ic_media_next, "Next", act(ACTION_MEDIA_NEXT, 12))
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOngoing(false)
            .build()
        nm.notify(NOTIF_MEDIA_ID, notif)
    }

    private fun startAutoClipboardIfEnabled() {
        if (!ClipboardAutoSync.isEnabled(this)) return
        if (autoClipboard != null) return
        autoClipboard = ClipboardAutoSync(this).also {
            android.os.Handler(mainLooper).post { it.start() }
        }
    }

    private fun handlePush(msg: org.json.JSONObject) {
        when (msg.optString("type")) {
            "clipboard" -> {
                val text = msg.optString("text")
                if (text.isEmpty()) return
                if (text == ClipboardStore.lastReceivedFromPc) return
                ClipboardStore.lastReceivedFromPc = text
                val cm = getSystemService(android.content.ClipboardManager::class.java)
                cm.setPrimaryClip(android.content.ClipData.newPlainText("Linux Link", text))
                Log.d(TAG, "📋 clipboard received from the PC (${text.length} characters)")
            }
            "notification_reply" -> {
                val key = msg.optString("key")
                val text = msg.optString("text")
                if (key.isEmpty() || text.isEmpty()) return
                NotificationRelayService.sendReply(key, text)
            }
            "open_url" -> {
                val url = msg.optString("url")
                if (url.isEmpty()) return
                showOpenUrlNotification(url, msg.optString("title"))
            }
            "media_info" -> {
                val ms = mediaSession ?: return
                android.os.Handler(mainLooper).post {
                    ms.updateInfo(
                        msg.optString("title"),
                        msg.optString("artist"),
                        msg.optBoolean("playing")
                    )
                    showMediaNotification(ms)
                }
            }
            "file_offer" -> {
                val id = msg.optString("id")
                val name = msg.optString("name")
                val size = msg.optLong("size")
                if (id.isEmpty() || name.isEmpty()) return
                if (msg.optBoolean("clipboard")) receiveClipboardImageFromPc(id, size)
                else receiveFileFromPc(id, name, size)
            }
            "display_invite" -> openSecondScreen()
            "dnd" -> applyDndFromPc(msg.optBoolean("on"))
            "phone_media" -> controlPhoneMedia(msg.optString("action"))
            "phone_volume" -> {
                val am = getSystemService(android.media.AudioManager::class.java)
                val stream = android.media.AudioManager.STREAM_MUSIC
                val showUi = android.media.AudioManager.FLAG_SHOW_UI
                when (msg.optString("action")) {
                    "up" -> am.adjustStreamVolume(
                        stream, android.media.AudioManager.ADJUST_RAISE, showUi)
                    "down" -> am.adjustStreamVolume(
                        stream, android.media.AudioManager.ADJUST_LOWER, showUi)
                    "mute" -> am.adjustStreamVolume(
                        stream, android.media.AudioManager.ADJUST_TOGGLE_MUTE, showUi)
                    "set" -> {
                        val pct = msg.optInt("value").coerceIn(0, 100)
                        val max = am.getStreamMaxVolume(stream)
                        am.setStreamVolume(stream, (pct * max) / 100, showUi)
                    }
                }
            }
            else -> Log.d(TAG, "unknown push: ${msg.optString("type")}")
        }
    }

    private fun controlPhoneMedia(action: String) {
        try {
            val msm = getSystemService(android.media.session.MediaSessionManager::class.java)
            val component = android.content.ComponentName(this, NotificationRelayService::class.java)
            val sessions = msm.getActiveSessions(component)
            val controller = sessions.firstOrNull() ?: run {
                Log.w(TAG, "no active media session on the phone")
                return
            }
            val tc = controller.transportControls
            when (action) {
                "play_pause" -> {
                    val playing = controller.playbackState?.state ==
                        android.media.session.PlaybackState.STATE_PLAYING
                    if (playing) tc.pause() else tc.play()
                }
                "next" -> tc.skipToNext()
                "previous" -> tc.skipToPrevious()
            }
        } catch (e: SecurityException) {
            Log.e(TAG, "media session access denied (grant notification access)", e)
        }
    }

    private fun openSecondScreen() {
        if (!AppPrefs.secondScreenEnabled(this)) {
            val channelId = "link_screen"
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(
                NotificationChannel(channelId, "Second screen", NotificationManager.IMPORTANCE_DEFAULT)
            )
            val openApp = android.app.PendingIntent.getActivity(
                this, SECOND_SCREEN_NOTIF,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
            )
            val notif = Notification.Builder(this, channelId)
                .setContentTitle("Second screen is off on this device")
                .setContentText("Your PC offered its display — enable Second screen in Linux Link")
                .setSmallIcon(android.R.drawable.stat_sys_upload_done)
                .setContentIntent(openApp)
                .setAutoCancel(true)
                .build()
            nm.notify(SECOND_SCREEN_NOTIF, notif)
            return
        }
        val open = Intent(this, SecondScreenActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        if (android.provider.Settings.canDrawOverlays(this) &&
            runCatching { startActivity(open) }.isSuccess
        ) {
            Log.d(TAG, "🖥 second screen opened on the PC's request")
            return
        }

        val channelId = "link_screen"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Second screen", NotificationManager.IMPORTANCE_HIGH)
        )
        val pending = android.app.PendingIntent.getActivity(
            this, SECOND_SCREEN_NOTIF, open,
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )
        val notif = Notification.Builder(this, channelId)
            .setContentTitle("Second screen")
            .setContentText("Your PC is offering its display — tap to open it")
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentIntent(pending)
            .setFullScreenIntent(pending, true)
            .setAutoCancel(true)
            .build()
        nm.notify(SECOND_SCREEN_NOTIF, notif)
    }

    private fun showOpenUrlNotification(url: String, title: String) {
        val channelId = "link_handoff"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Handoff", NotificationManager.IMPORTANCE_HIGH)
        )
        val view = Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val pending = android.app.PendingIntent.getActivity(
            this, url.hashCode(), view,
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )
        val notif = Notification.Builder(this, channelId)
            .setContentTitle(if (title.isBlank()) "Open from the PC" else "Open: $title")
            .setContentText(url)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build()
        nm.notify(url.hashCode(), notif)
    }

    private fun disconnect() {
        connected = false
        linkUp = false
        connectionJob?.cancel()
        connectionJob = null
        client?.close()
        client = null
        activeClient = null
        scope.coroutineContext.cancelChildren()
    }

    override fun onDestroy() {
        disconnect()
        netMonitor?.stop()
        netMonitor = null
        syncWatcher?.stop()
        syncWatcher = null
        autoClipboard?.let { ac -> android.os.Handler(mainLooper).post { ac.stop() } }
        autoClipboard = null
        mediaSession?.let { ms -> android.os.Handler(mainLooper).post { ms.stop() } }
        mediaSession = null
        batteryReceiver?.let { runCatching { unregisterReceiver(it) } }
        batteryReceiver = null
        dndReceiver?.let { runCatching { unregisterReceiver(it) } }
        dndReceiver = null
        getSystemService(NotificationManager::class.java).cancel(NOTIF_MEDIA_ID)
        super.onDestroy()
    }

    private fun startAsForeground(text: String) {
        val channel = NotificationChannel(
            CHANNEL_ID, "PC connection", NotificationManager.IMPORTANCE_MIN
        )
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        val notification = buildNotification(text)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIF_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } else {
            startForeground(NOTIF_ID, notification)
        }
    }

    private fun updateNotification(text: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIF_ID, buildNotification(text))
    }

    private fun buildNotification(text: String): Notification =
        Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("Linux Link")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setOngoing(true)
            .build()

    companion object {
        const val ACTION_CONNECT = "org.linuxlink.app.CONNECT"
        const val ACTION_DISCONNECT = "org.linuxlink.app.DISCONNECT"
        const val ACTION_RECONNECT = "org.linuxlink.app.RECONNECT"
        const val ACTION_BLE_GONE = "org.linuxlink.app.BLE_GONE"
        const val ACTION_MEDIA_PREV = "org.linuxlink.app.MEDIA_PREV"
        const val ACTION_MEDIA_PLAYPAUSE = "org.linuxlink.app.MEDIA_PLAYPAUSE"
        const val ACTION_MEDIA_NEXT = "org.linuxlink.app.MEDIA_NEXT"
        const val ACTION_SEND_FILE = "org.linuxlink.app.SEND_FILE"
        const val EXTRA_FILE_URIS = "file_uris"

        @Volatile
        var activeClient: LinkClient? = null
            private set

        var linkUp by mutableStateOf(false)
            private set
        private const val CHANNEL_ID = "link_connection"
        private const val NOTIF_ID = 1
        private const val NOTIF_MEDIA_ID = 2
        private const val NOTIF_TRANSFER_ID = 3
        private const val SECOND_SCREEN_NOTIF = 4
        private const val TAG = "LinkForegroundService"
    }
}
