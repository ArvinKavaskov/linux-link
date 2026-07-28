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

    override fun onDeviceDisappeared(associationInfo: AssociationInfo) {
        Log.i(TAG, "PC out of range → disconnecting")
        startService(
            Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_DISCONNECT)
        )
    }

    companion object {
        private const val TAG = "PcCompanionService"
    }
}
