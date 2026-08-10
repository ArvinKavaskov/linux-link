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
        val enabled = AppPrefs.secondScreenEnabled(this)
        val up = LinkForegroundService.activeClient != null
        tile.state = if (enabled && up) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = when {
                !enabled -> "Off — enable in the app"
                up -> "Ready"
                else -> "No PC connected"
            }
        }
        tile.updateTile()
    }

    override fun onClick() {
        // Feature off on this device (the phone default): the tile opens the
        // app on the switch instead of a screen that would refuse to work.
        val target = if (AppPrefs.secondScreenEnabled(this))
            SecondScreenActivity::class.java else MainActivity::class.java
        val intent = Intent(this, target)
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
