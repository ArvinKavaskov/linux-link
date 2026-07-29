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

/**
 * Tells the connection loop the exact moment it is worth trying again.
 *
 * v2 just slept: `delay(2s × attempt)` up to thirty seconds. Walk from one
 * access point to another and you would stare at "PC unreachable" for half a
 * minute while the phone had perfectly good Wi-Fi the whole time. The fix is
 * not a shorter sleep — it is not sleeping at all, and instead waking up on
 * the events that actually change the answer:
 *
 *  * a network becomes available or validated (Wi-Fi came back, you roamed,
 *    you walked in the front door),
 *  * the screen comes on, which is both a good proxy for "the user is about
 *    to want this to work" and the moment Doze lets us talk again.
 *
 * The channel is CONFLATED: a burst of six callbacks during a Wi-Fi handover
 * results in exactly one reconnection attempt, not six.
 */
class NetworkMonitor(private val context: Context) {

    private val cm = context.getSystemService(ConnectivityManager::class.java)

    /** Receives a tick whenever it is worth retrying. */
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
        // ACTION_SCREEN_ON is a protected system broadcast, but targetSdk 34+
        // wants the export flag spelled out either way.
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

    /** Wake the loop by hand (used right after the user taps "Connect"). */
    fun poke() {
        wakeups.trySend(Unit)
    }

    private companion object {
        const val TAG = "NetworkMonitor"
    }
}
