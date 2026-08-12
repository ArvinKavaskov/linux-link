package org.linuxlink.app

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.util.Log
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.coroutines.resume
import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.SocketTimeoutException

object PcLocator {
    private const val TAG = "PcLocator"
    const val DISCOVERY_PORT = 47101
    private const val PROBE = "LINUXLINK?v1"
    private const val SERVICE_TYPE = "_linuxlink._udp"

    suspend fun discover(context: Context, pc: PairedPc): List<String> = coroutineScope {
        val mdns = async { mdnsLookup(context, pc.fingerprint) }
        val udp = udpProbe(pc.fingerprint)

        val found = LinkedHashSet<String>()
        found += udp
        if (found.isEmpty()) found += mdns.await() else mdns.cancel()
        found += extraAddresses(context)
        found.remove(pc.lastAddress)
        if (found.isNotEmpty()) Log.i(TAG, "PC candidates: $found")
        found.toList()
    }

    suspend fun discoverAny(context: Context, known: List<PairedPc>): PairedPc? =
        withContext(Dispatchers.IO) {
            if (known.isEmpty()) return@withContext null
            var found: PairedPc? = null
            runCatching {
                DatagramSocket().use { sock ->
                    sock.broadcast = true
                    sock.soTimeout = 200
                    val data = PROBE.toByteArray()
                    val targets = broadcastAddresses()
                    if (targets.isEmpty()) return@use
                    for (addr in targets) {
                        runCatching { sock.send(DatagramPacket(data, data.size, addr, DISCOVERY_PORT)) }
                    }
                    val buf = ByteArray(512)
                    val deadline = System.currentTimeMillis() + 1_000
                    while (System.currentTimeMillis() < deadline) {
                        val p = DatagramPacket(buf, buf.size)
                        try {
                            sock.receive(p)
                        } catch (e: SocketTimeoutException) {
                            continue
                        }
                        val o = runCatching { JSONObject(String(p.data, 0, p.length)) }.getOrNull()
                            ?: continue
                        val fp = o.optString("fp")
                        val match = known.firstOrNull { it.fingerprint.equals(fp, ignoreCase = true) }
                            ?: continue
                        val host = p.address?.hostAddress ?: continue
                        Log.i(TAG, "known PC ${o.optString("name")} answered from $host")
                        found = match.copy(
                            name = o.optString("name", match.name),
                            lastAddress = host,
                            port = o.optInt("port", match.port),
                        )
                        break
                    }
                }
            }.onFailure { Log.d(TAG, "discoverAny failed: ${it.message}") }
            found
        }

    private suspend fun udpProbe(fingerprint: String): List<String> = withContext(Dispatchers.IO) {
        val out = mutableListOf<String>()
        runCatching {
            DatagramSocket().use { sock ->
                sock.broadcast = true
                sock.soTimeout = 200
                val data = PROBE.toByteArray()
                val targets = broadcastAddresses()
                if (targets.isEmpty()) return@use
                for (addr in targets) {
                    runCatching { sock.send(DatagramPacket(data, data.size, addr, DISCOVERY_PORT)) }
                }
                val buf = ByteArray(512)
                val deadline = System.currentTimeMillis() + 800
                while (System.currentTimeMillis() < deadline) {
                    val p = DatagramPacket(buf, buf.size)
                    try {
                        sock.receive(p)
                    } catch (e: SocketTimeoutException) {
                        continue
                    }
                    val o = runCatching { JSONObject(String(p.data, 0, p.length)) }.getOrNull()
                        ?: continue
                    if (!o.optString("fp").equals(fingerprint, ignoreCase = true)) continue
                    val host = p.address?.hostAddress ?: continue
                    Log.i(TAG, "beacon reply from $host (${o.optString("name")})")
                    out += host
                    break
                }
            }
        }.onFailure { Log.d(TAG, "udp probe failed: ${it.message}") }
        out
    }

    private fun broadcastAddresses(): List<InetAddress> {
        val out = mutableListOf<InetAddress>()
        runCatching {
            for (nif in NetworkInterface.getNetworkInterfaces()) {
                if (!nif.isUp || nif.isLoopback) continue
                for (ia in nif.interfaceAddresses) {
                    ia.broadcast?.let { out += it }
                }
            }
        }
        runCatching { out += InetAddress.getByName("255.255.255.255") }
        return out
    }

    private suspend fun mdnsLookup(context: Context, fingerprint: String): List<String> =
        withTimeoutOrNull(2_500) { mdnsLookupInner(context, fingerprint) } ?: emptyList()

    private suspend fun mdnsLookupInner(context: Context, fingerprint: String): List<String> =
        suspendCancellableCoroutine { cont ->
            val nsd = context.getSystemService(NsdManager::class.java)
            if (nsd == null) {
                cont.resume(emptyList())
                return@suspendCancellableCoroutine
            }
            val results = mutableListOf<String>()

            val discovery = object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(type: String) {}
                override fun onDiscoveryStopped(type: String) {}
                override fun onStartDiscoveryFailed(type: String, code: Int) {
                    finish(cont, results)
                }

                override fun onStopDiscoveryFailed(type: String, code: Int) {}
                override fun onServiceLost(info: NsdServiceInfo) {}

                override fun onServiceFound(info: NsdServiceInfo) {
                    @Suppress("DEPRECATION")
                    nsd.resolveService(info, object : NsdManager.ResolveListener {
                        override fun onResolveFailed(i: NsdServiceInfo, code: Int) {}
                        override fun onServiceResolved(i: NsdServiceInfo) {
                            val fp = i.attributes["fp"]?.let { String(it) }
                            val host = i.host?.hostAddress
                            if (host != null && (fp == null || fp.equals(fingerprint, true))) {
                                Log.i(TAG, "mDNS resolved $host")
                                synchronized(results) { if (host !in results) results += host }
                                finish(cont, results)
                            }
                        }
                    })
                }
            }

            cont.invokeOnCancellation { runCatching { nsd.stopServiceDiscovery(discovery) } }
            runCatching {
                nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, discovery)
            }.onFailure { finish(cont, results) }
        }

    private fun finish(cont: CancellableContinuation<List<String>>, results: List<String>) {
        if (cont.isActive) {
            cont.resume(synchronized(results) { results.toList() })
        }
    }

    private fun extraAddresses(context: Context): List<String> =
        context.getSharedPreferences("paired_pc", Context.MODE_PRIVATE)
            .getString("alt_addresses", "")
            ?.split(',')
            ?.map { it.trim() }
            ?.filter { it.isNotEmpty() }
            ?: emptyList()

    fun rememberAlternates(context: Context, addresses: List<String>) {
        context.getSharedPreferences("paired_pc", Context.MODE_PRIVATE).edit()
            .putString("alt_addresses", addresses.joinToString(","))
            .apply()
    }
}
