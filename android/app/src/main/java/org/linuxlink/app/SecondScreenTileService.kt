package org.linuxlink.app

import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * The Quick Settings tile for the second screen: swipe down, tap, and this
 * device is a monitor — without ever opening the app.
 *
 * The tile tells the truth before it is tapped: active when a PC is
 * connected, inactive with a one-word reason when none is.
 */
class SecondScreenTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        val tile = qsTile ?: return
        val up = LinkForegroundService.activeClient != null
        tile.state = if (up) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = if (up) "Ready" else "No PC connected"
        }
        tile.updateTile()
    }

    override fun onClick() {
        val intent = Intent(this, SecondScreenActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                PendingIntent.getActivity(
                    this, 0, intent,
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
                )
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
