package org.linuxlink.app

import android.content.Context
import android.media.MediaMetadata
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.util.Log
import org.json.JSONObject

class LinkMediaSession(private val context: Context) {

    private var session: MediaSession? = null

    var title: String = "PC media"
        private set
    var artist: String = "Linux Link"
        private set
    var playing: Boolean = true
        private set

    fun start() {
        if (session != null) return
        val s = MediaSession(context, "LinuxLink")

        s.setCallback(object : MediaSession.Callback() {
            override fun onPlay() = sendMedia("play_pause")
            override fun onPause() = sendMedia("play_pause")
            override fun onSkipToNext() = sendMedia("next")
            override fun onSkipToPrevious() = sendMedia("previous")
        })

        val volumeProvider = object : VolumeProvider(
            VOLUME_CONTROL_RELATIVE, 100, 50
        ) {
            override fun onAdjustVolume(direction: Int) {
                when {
                    direction > 0 -> sendVolume("up")
                    direction < 0 -> sendVolume("down")
                }
                currentVolume = 50
            }
        }
        s.setPlaybackToRemote(volumeProvider)

        s.isActive = true
        session = s
        applyState()
        Log.i(TAG, "Media session active (bubble + volume buttons → PC)")
    }

    fun updateInfo(title: String, artist: String, playing: Boolean) {
        this.title = title.ifBlank { "PC media" }
        this.artist = artist.ifBlank { "Linux Link" }
        this.playing = playing
        applyState()
    }

    private fun applyState() {
        val s = session ?: return
        s.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .build()
        )
        s.setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY_PAUSE or
                        PlaybackState.ACTION_SKIP_TO_NEXT or
                        PlaybackState.ACTION_SKIP_TO_PREVIOUS
                )
                .setState(
                    if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
                    PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f
                )
                .build()
        )
    }

    fun token(): MediaSession.Token? = session?.sessionToken

    fun stop() {
        session?.let {
            it.isActive = false
            it.release()
        }
        session = null
    }

    private fun sendMedia(action: String) {
        LinkBus.send(JSONObject().apply {
            put("type", "pc_media")
            put("action", action)
        })
    }

    private fun sendVolume(action: String) {
        LinkBus.send(JSONObject().apply {
            put("type", "pc_volume")
            put("action", action)
        })
    }

    companion object {
        private const val TAG = "LinkMediaSession"
    }
}
