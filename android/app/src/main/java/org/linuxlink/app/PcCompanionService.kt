package org.linuxlink.app

import android.companion.AssociationInfo
import android.companion.CompanionDeviceService
import android.content.Intent
import android.os.Build
import android.util.Log
import androidx.annotation.RequiresApi

@RequiresApi(Build.VERSION_CODES.S)
class PcCompanionService : CompanionDeviceService() {

    override fun onDeviceAppeared(associationInfo: AssociationInfo) {
        Log.i(TAG, "PC within BLE range (association ${associationInfo.id}) → connecting")
        startForegroundService(
            Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_CONNECT)
        )
    }

    /**
     * BLE range is about ten metres — one wall. Tearing the link down here was
     * wrong: walk into the next room and the clipboard, notifications and file
     * transfers all died while the Wi-Fi link was perfectly healthy. We now
     * hand the decision to the service, which keeps a working QUIC connection
     * and only shuts down if the PC really has gone away.
     */
    override fun onDeviceDisappeared(associationInfo: AssociationInfo) {
        Log.i(TAG, "PC out of BLE range → letting the service decide")
        startService(
            Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_BLE_GONE)
        )
    }

    companion object {
        private const val TAG = "PcCompanionService"
    }
}
