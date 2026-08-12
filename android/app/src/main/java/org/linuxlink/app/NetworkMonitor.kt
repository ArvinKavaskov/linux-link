package org.linuxlink.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import kotlinx.coroutines.channels.Channel

class NetworkMonitor(private val context: Context) {
    private val cm = context.getSystemService(ConnectivityManager::class.java)

    val wakeups = Channel<Unit>(Channel.CONFLATED)

    @Volatile
    var online: Boolean = false
        private set

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            online = true
            Log.i(TAG, "network available → wake")
            wakeups.trySend(Unit)
        }

        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            val usable = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
            if (usable) {
                online = true
                wakeups.trySend(Unit)
            }
        }

        override fun onLost(network: Network) {
            online = cm?.activeNetwork != null
            Log.i(TAG, "network lost (still online: $online)")
        }
    }

    private val screenOn = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, intent: Intent?) {
            Log.i(TAG, "screen on → wake")
            wakeups.trySend(Unit)
        }
    }

    fun start() {
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
            .build()
        runCatching { cm?.registerNetworkCallback(request, callback) }
            .onFailure { Log.w(TAG, "network callback: ${it.message}") }
        runCatching {
            context.registerReceiver(
                screenOn,
                IntentFilter(Intent.ACTION_SCREEN_ON),
                Context.RECEIVER_NOT_EXPORTED,
            )
        }.onFailure { Log.w(TAG, "screen receiver: ${it.message}") }
        online = cm?.activeNetwork != null
    }

    fun stop() {
        runCatching { cm?.unregisterNetworkCallback(callback) }
        runCatching { context.unregisterReceiver(screenOn) }
    }

    fun poke() {
        wakeups.trySend(Unit)
    }

    private companion object {
        const val TAG = "NetworkMonitor"
    }
}
