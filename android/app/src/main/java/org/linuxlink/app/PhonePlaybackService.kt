package org.linuxlink.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class PhonePlaybackService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var projection: MediaProjection? = null
    private var record: AudioRecord? = null
    private var job: Job? = null

    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() {
            teardown()
            stopSelf()
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            teardown()
            stopSelf()
            return START_NOT_STICKY
        }
        val code = intent?.getIntExtra(EXTRA_CODE, 0) ?: 0
        @Suppress("DEPRECATION")
        val data: Intent? = intent?.getParcelableExtra(EXTRA_DATA)
        if (code == 0 || data == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        startAsForeground()
        val mgr = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val proj = try {
            mgr.getMediaProjection(code, data)
        } catch (e: Exception) {
            Log.w(TAG, "projection refused: ${e.message}")
            stopSelf()
            return START_NOT_STICKY
        } ?: run { stopSelf(); return START_NOT_STICKY }
        proj.registerCallback(projectionCallback, Handler(Looper.getMainLooper()))
        projection = proj
        stream(proj)
        return START_NOT_STICKY
    }

    private fun stream(proj: MediaProjection) {
        val client = LinkForegroundService.activeClient
        if (client == null) {
            Log.w(TAG, "no PC connected")
            teardown()
            stopSelf()
            return
        }
        val rec = try {
            val format = AudioFormat.Builder()
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(RATE)
                .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                .build()
            val cfg = AudioPlaybackCaptureConfiguration.Builder(proj)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
            val min = AudioRecord.getMinBufferSize(
                RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT
            )
            AudioRecord.Builder()
                .setAudioFormat(format)
                .setAudioPlaybackCaptureConfig(cfg)
                .setBufferSizeInBytes(min * 4)
                .build()
        } catch (e: Exception) {
            Log.w(TAG, "capture setup failed: ${e.message}")
            teardown()
            stopSelf()
            return
        }
        record = rec
        job = scope.launch {
            try {
                val out = client.openPhoneAudio(RATE, 2)
                rec.startRecording()
                running = true
                Log.i(TAG, "phone audio → PC started")
                val buf = ByteArray(CHUNK)
                while (true) {
                    val n = rec.read(buf, 0, buf.size)
                    if (n <= 0) break
                    out.write(buf, 0, n)
                    out.flush()
                }
            } catch (e: Exception) {
                Log.w(TAG, "phone audio ended: ${e.message}")
            } finally {
                running = false
                teardown()
                stopSelf()
            }
        }
    }

    private fun startAsForeground() {
        val channelId = "link_phone_audio"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Phone audio on PC", NotificationManager.IMPORTANCE_LOW)
        )
        val notif = Notification.Builder(this, channelId)
            .setContentTitle("Phone audio playing on the PC")
            .setContentText("Voice messages and media are heard on the computer")
            .setSmallIcon(android.R.drawable.stat_sys_speakerphone)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    private fun teardown() {
        running = false
        job?.cancel()
        job = null
        runCatching { record?.stop() }
        runCatching { record?.release() }
        record = null
        runCatching { projection?.unregisterCallback(projectionCallback) }
        runCatching { projection?.stop() }
        projection = null
    }

    override fun onDestroy() {
        teardown()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "PhonePlayback"
        private const val NOTIF_ID = 47
        private const val RATE = 48_000
        private const val CHUNK = 3_840
        private const val ACTION_STOP = "org.linuxlink.app.PHONE_AUDIO_STOP"
        private const val EXTRA_CODE = "code"
        private const val EXTRA_DATA = "data"

        var running by mutableStateOf(false)
            private set

        fun start(context: Context, resultCode: Int, resultData: Intent) {
            context.startForegroundService(
                Intent(context, PhonePlaybackService::class.java)
                    .putExtra(EXTRA_CODE, resultCode)
                    .putExtra(EXTRA_DATA, resultData)
            )
        }

        fun stop(context: Context) {
            context.startService(
                Intent(context, PhonePlaybackService::class.java).setAction(ACTION_STOP)
            )
        }
    }
}
