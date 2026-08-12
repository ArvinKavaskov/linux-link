package org.linuxlink.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.IBinder

class PhoneAudioService : Service() {
    companion object {
        const val ACTION_SPEAKER_START = "org.linuxlink.SPEAKER_START"
        const val ACTION_SPEAKER_STOP = "org.linuxlink.SPEAKER_STOP"
        const val ACTION_MIC_START = "org.linuxlink.MIC_START"
        const val ACTION_MIC_STOP = "org.linuxlink.MIC_STOP"

        @Volatile var speakerOn = false
            private set
        @Volatile var micOn = false
            private set
    }

    @Volatile private var speakerThread: Thread? = null
    @Volatile private var micThread: Thread? = null
    private var micWriter: MicWriter? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_SPEAKER_START -> {
                refreshForeground(wantSpeaker = true)
                startSpeaker()
            }
            ACTION_MIC_START -> {
                if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
                    android.content.pm.PackageManager.PERMISSION_GRANTED
                ) {
                    refreshForeground(wantMic = true)
                    startMic()
                }
            }
            ACTION_SPEAKER_STOP -> stopSpeaker()
            ACTION_MIC_STOP -> stopMic()
        }
        if (!speakerOn && !micOn) {
            stopSelf()
        } else {
            refreshForeground()
        }
        return START_NOT_STICKY
    }

    private fun startSpeaker() {
        if (speakerThread != null) return
        val client = LinkForegroundService.activeClient ?: return
        val rate = 48_000
        val channels = 2

        speakerOn = true
        speakerThread = Thread {
            try {
                val input = client.openSpeaker(rate, channels)
                val minBuf = AudioTrack.getMinBufferSize(
                    rate, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT
                )
                val track = AudioTrack.Builder()
                    .setAudioAttributes(
                        AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                            .build()
                    )
                    .setAudioFormat(
                        AudioFormat.Builder()
                            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                            .setSampleRate(rate)
                            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                            .build()
                    )
                    .setBufferSizeInBytes(maxOf(minBuf * 2, 16_384))
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .build()
                track.play()
                val buf = ByteArray(8192)
                try {
                    while (!Thread.currentThread().isInterrupted) {
                        val n = input.read(buf)
                        if (n < 0) break
                        if (n > 0) track.write(buf, 0, n)
                    }
                } finally {
                    runCatching { track.stop() }
                    track.release()
                    runCatching { input.close() }
                }
            } catch (_: Exception) {
            } finally {
                speakerOn = false
                speakerThread = null
                stopIfIdle()
            }
        }.also { it.start() }
    }

    private fun stopSpeaker() {
        speakerThread?.interrupt()
    }

    private fun startMic() {
        if (micThread != null) return
        val client = LinkForegroundService.activeClient ?: return
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) return

        val rate = 48_000
        val minBuf = android.media.AudioRecord.getMinBufferSize(
            rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT
        )
        val recorder = try {
            @Suppress("MissingPermission")
            android.media.AudioRecord(
                android.media.MediaRecorder.AudioSource.VOICE_COMMUNICATION, rate,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuf, 8192)
            )
        } catch (_: Exception) {
            return
        }

        val sid = recorder.audioSessionId
        runCatching {
            if (android.media.audiofx.NoiseSuppressor.isAvailable())
                android.media.audiofx.NoiseSuppressor.create(sid)?.enabled = true
            if (android.media.audiofx.AutomaticGainControl.isAvailable())
                android.media.audiofx.AutomaticGainControl.create(sid)?.enabled = true
            if (android.media.audiofx.AcousticEchoCanceler.isAvailable())
                android.media.audiofx.AcousticEchoCanceler.create(sid)?.enabled = true
        }

        micWriter = runCatching { client.openMic(rate, 1) }.getOrElse {
            recorder.release()
            return
        }
        recorder.startRecording()
        micOn = true
        micThread = Thread {
            val buf = ByteArray(4096)
            try {
                while (!Thread.currentThread().isInterrupted) {
                    val n = recorder.read(buf, 0, buf.size)
                    if (n > 0) {
                        try { micWriter?.write(buf, n) } catch (_: Exception) { break }
                    } else if (n < 0) break
                }
            } finally {
                runCatching { recorder.stop() }
                recorder.release()
                micWriter?.close()
                micWriter = null
                micOn = false
                micThread = null
                stopIfIdle()
            }
        }.also { it.start() }
    }

    private fun stopMic() {
        micThread?.interrupt()
    }

    private fun stopIfIdle() {
        if (!speakerOn && !micOn) stopSelf()
    }

    private fun refreshForeground(
        wantSpeaker: Boolean = speakerOn,
        wantMic: Boolean = micOn,
    ) {
        val speaker = speakerOn || wantSpeaker
        val mic = micOn || wantMic
        val channelId = "phone_audio"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(channelId, "Speaker & mic", NotificationManager.IMPORTANCE_LOW)
        )
        val text = when {
            speaker && mic -> "Speaker and microphone of the PC"
            speaker -> "Speaker of the PC"
            else -> "Microphone of the PC"
        }
        val tap = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE
        )
        val notification: Notification = Notification.Builder(this, channelId)
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentTitle("Linux Link")
            .setContentText(text)
            .setContentIntent(tap)
            .setOngoing(true)
            .build()
        var types = 0
        if (speaker) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
        if (mic) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        startForeground(2001, notification, types)
    }

    override fun onDestroy() {
        speakerThread?.interrupt()
        micThread?.interrupt()
        speakerOn = false
        micOn = false
        super.onDestroy()
    }
}
