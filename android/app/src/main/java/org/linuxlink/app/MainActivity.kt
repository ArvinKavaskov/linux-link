package org.linuxlink.app

import android.bluetooth.le.ScanFilter
import android.companion.AssociationRequest
import android.companion.BluetoothLeDeviceFilter
import android.companion.CompanionDeviceManager
import android.content.Intent
import android.content.IntentSender
import android.os.Build
import android.os.Bundle
import android.os.ParcelUuid
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Shapes
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.UUID

private val LinkDarkFallback = darkColorScheme(
    primary = Color(0xFF9E7BFF),
    onPrimary = Color(0xFF1A1030),
    primaryContainer = Color(0xFF2C2350),
    onPrimaryContainer = Color(0xFFE7DEFF),
    background = Color(0xFF14121C),
    surface = Color(0xFF1C1926),
    onSurface = Color(0xFFE8E6F0),
    surfaceVariant = Color(0xFF2A2636),
)

private val LinkLightFallback = lightColorScheme(
    primary = Color(0xFF6C4BC7),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFE9DDFF),
    onPrimaryContainer = Color(0xFF23085C),
    background = Color(0xFFFDF8FF),
    surface = Color(0xFFFDF8FF),
    onSurface = Color(0xFF1C1B1F),
    surfaceVariant = Color(0xFFE7E0EB),
)

private val LinkShapes = Shapes(
    extraSmall = RoundedCornerShape(10.dp),
    small = RoundedCornerShape(14.dp),
    medium = RoundedCornerShape(18.dp),
    large = RoundedCornerShape(24.dp),
    extraLarge = RoundedCornerShape(30.dp),
)

@Composable
private fun LinkTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
    } else {
        if (dark) LinkDarkFallback else LinkLightFallback
    }
    MaterialTheme(colorScheme = colors, shapes = LinkShapes, content = content)
}

class MainActivity : ComponentActivity() {

    private var status by mutableStateOf("No PC paired")
    private var pendingPairing: QrPayload? = null
    private val uiScope = CoroutineScope(Dispatchers.Main)

    private val qrScanner = registerForActivityResult(ScanContract()) { result ->
        result.contents?.let { raw ->
            runCatching { QrPayload.parse(raw) }
                .onSuccess { payload -> pairOverWifi(payload) }
                .onFailure { status = "Invalid QR code: ${it.message}" }
        }
    }

    private val cdmChooser = registerForActivityResult(
        ActivityResultContracts.StartIntentSenderForResult()
    ) { result ->
        if (result.resultCode == RESULT_OK) {
            onCompanionAssociated(result.data)
        } else {
            status = "Companion association refused (automatic wake-up will not work)"
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        PairedPc.load(this)?.let { status = "Paired with ${it.name}" }
        ensurePresenceObservation()

        setContent {
            LinkTheme {
                val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
                Scaffold(
                    modifier = Modifier
                        .fillMaxSize()
                        .nestedScroll(scrollBehavior.nestedScrollConnection),
                    topBar = {
                        LargeTopAppBar(
                            title = { Text("Linux Link", fontWeight = FontWeight.Bold) },
                            scrollBehavior = scrollBehavior,
                            colors = TopAppBarDefaults.largeTopAppBarColors(
                                titleContentColor = MaterialTheme.colorScheme.primary,
                            ),
                        )
                    },
                ) { innerPadding ->
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(innerPadding)
                            .padding(horizontal = 16.dp),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        StatusCard(status)

                        Section("Device") {
                            WideButton("Pair a PC (scan the QR)") { startPairing() }
                            WideButton("Connect now") { connectNow() }
                            WideButton("Test the connection (ping)") { testConnection() }
                        }

                        Section("PC control") {
                            Text("Volume", style = MaterialTheme.typography.labelLarge)
                            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                Button(onClick = { pcVolume("down") }, modifier = Modifier.weight(1f)) { Text("🔉 −") }
                                Button(onClick = { pcVolume("mute") }, modifier = Modifier.weight(1f)) { Text("🔇") }
                                Button(onClick = { pcVolume("up") }, modifier = Modifier.weight(1f)) { Text("🔊 +") }
                            }
                            Text("Media", style = MaterialTheme.typography.labelLarge)
                            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                Button(onClick = { pcMedia("previous") }, modifier = Modifier.weight(1f)) { Text("⏮") }
                                Button(onClick = { pcMedia("play_pause") }, modifier = Modifier.weight(1f)) { Text("⏯") }
                                Button(onClick = { pcMedia("next") }, modifier = Modifier.weight(1f)) { Text("⏭") }
                            }
                        }

                        Section("Clipboard") {
                            WideButton("Send the clipboard to the PC") {
                                startActivity(Intent(this@MainActivity, SendClipboardActivity::class.java))
                            }
                            WideButton(
                                if (ClipboardAutoSync.isEnabled(this@MainActivity)) "Auto clipboard: on ✔"
                                else "Enable automatic clipboard"
                            ) { toggleAutoClipboard() }
                            WideButton(
                                if (ShizukuClipboard.ready()) "Shizuku: connected ✔"
                                else "Connect Shizuku (reliable auto)"
                            ) { setupShizuku() }
                        }

                        Section("Continuity") {
                            WideButton(
                                if (SyncFolder.isEnabled(this@MainActivity) && SyncFolder.hasAllFilesAccess())
                                    "Folder sync: on ✔"
                                else "Enable folder sync"
                            ) { setupFolderSync() }
                            WideButton(
                                if (dndAccessGranted()) "Do Not Disturb sync: on ✔"
                                else "Enable Do Not Disturb sync"
                            ) { setupDndSync() }
                            WideButton(
                                if (notificationAccessGranted()) "Notification mirror: on ✔"
                                else "Enable notification mirror"
                            ) { openNotificationAccessSettings() }
                        }

                        Section("Camera & mic") {
                            WideButton("📷 Webcam / 🎤 Mic (phone → PC)") {
                                startActivity(Intent(this@MainActivity, WebcamActivity::class.java))
                            }
                        }

                        Spacer(Modifier.height(8.dp))
                    }
                }
            }
        }
    }

    @Composable
    private fun StatusCard(text: String) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.extraLarge,
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
            ),
        ) {
            Row(
                modifier = Modifier.padding(20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("🔗", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.width(14.dp))
                Text(text, style = MaterialTheme.typography.bodyLarge)
            }
        }
    }

    @Composable
    private fun Section(title: String, content: @Composable ColumnScope.() -> Unit) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.large,
        ) {
            Column(
                modifier = Modifier.padding(18.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.primary,
                )
                content()
            }
        }
    }

    @Composable
    private fun WideButton(label: String, onClick: () -> Unit) {
        FilledTonalButton(
            onClick = onClick,
            modifier = Modifier
                .fillMaxWidth()
                .height(52.dp),
            shape = MaterialTheme.shapes.large,
        ) {
            Text(label, textAlign = TextAlign.Center)
        }
    }

    private fun startPairing() {
        qrScanner.launch(
            ScanOptions()
                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setCaptureActivity(PairScanActivity::class.java)
                .setOrientationLocked(true)
                .setBeepEnabled(false)
        )
    }

    private fun pairOverWifi(payload: QrPayload) {
        status = "Connecting to ${payload.name}…"
        uiScope.launch {
            try {
                val identity = Identity.loadOrCreate(this@MainActivity)
                val client = LinkClient(identity).also { it.expectedFingerprint = payload.fingerprint }
                // The QR carries every address the PC answers on (Ethernet and
                // Wi-Fi, typically). Try them in order instead of betting the
                // whole pairing on the first one.
                var host: String? = null
                for (candidate in payload.addrs) {
                    try {
                        client.connect(candidate, payload.port, timeoutMillis = 3_000)
                        host = candidate
                        break
                    } catch (e: Exception) {
                        Log.d(TAG, "$candidate unreachable: ${e.message}")
                        client.close()
                    }
                }
                val address = host ?: error("PC unreachable at ${payload.addrs.joinToString()}")
                val pcName = client.pair(payload.token, Build.MODEL)
                client.close()

                PairedPc.save(
                    this@MainActivity,
                    PairedPc(pcName, address, payload.port, payload.fingerprint)
                )
                // Keep the others as fallbacks for the rediscovery path.
                PcLocator.rememberAlternates(
                    this@MainActivity, payload.addrs.filter { it != address }
                )
                status = "Paired with $pcName ✔ — companion association…"
                pendingPairing = payload
                requestCompanionAssociation()
            } catch (e: Exception) {
                status = "Pairing failed: ${e.message}"
                Log.e(TAG, "pairing", e)
            }
        }
    }

    private fun requestCompanionAssociation() {
        val cdm = getSystemService(CompanionDeviceManager::class.java)

        val bleFilter = BluetoothLeDeviceFilter.Builder()
            .setScanFilter(
                ScanFilter.Builder()
                    .setServiceUuid(ParcelUuid(UUID.fromString(LINK_SERVICE_UUID)))
                    .build()
            )
            .build()

        val request = AssociationRequest.Builder()
            .addDeviceFilter(bleFilter)
            .setSingleDevice(true)
            .build()

        cdm.associate(request, object : CompanionDeviceManager.Callback() {
            @Deprecated("old signature, still called on API < 33")
            override fun onDeviceFound(chooserLauncher: IntentSender) {
                cdmChooser.launch(IntentSenderRequest.Builder(chooserLauncher).build())
            }

            override fun onFailure(error: CharSequence?) {
                status = "Companion association failed: $error\n" +
                    "(check that the PC is advertising over BLE — `linkd run` without --no-ble)"
            }
        }, null)
    }

    private fun onCompanionAssociated(data: Intent?) {
        ensurePresenceObservation()
        status = "Everything is ready: the phone will connect on its own.\n" +
            "Turn the PC off/on to verify it."
    }

    private fun ensurePresenceObservation() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        val cdm = getSystemService(CompanionDeviceManager::class.java)
        val associations = cdm.myAssociations
        if (associations.isEmpty()) {
            if (PairedPc.load(this) != null) {
                status = "⚠ Paired, but no companion association: pair again " +
                    "(scan the QR) and accept the Android dialog."
            }
            return
        }
        var armed = 0
        associations.forEach { assoc ->
            val mac = assoc.deviceMacAddress?.toString()
            if (mac == null) {
                Log.w(TAG, "association ${assoc.id} without MAC address")
                return@forEach
            }
            runCatching {
                @Suppress("DEPRECATION")
                cdm.startObservingDevicePresence(mac)
                armed++
                Log.i(TAG, "presence observation armed for $mac")
            }.onFailure {
                Log.e(TAG, "startObservingDevicePresence($mac)", it)
                status = "⚠ Presence observation refused: ${it.message}"
            }
        }
        if (armed > 0) {
            Log.i(TAG, "$armed association(s) under presence observation")
        }
    }

    private fun toggleAutoClipboard() {
        if (!ShizukuClipboard.ready() && !android.provider.Settings.canDrawOverlays(this)) {
            status = "For automatic mode: connect Shizuku (recommended) OR allow " +
                "\"Display over other apps\"."
            startActivity(
                Intent(
                    android.provider.Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                    android.net.Uri.parse("package:$packageName")
                )
            )
            return
        }
        val newState = !ClipboardAutoSync.isEnabled(this)
        ClipboardAutoSync.setEnabled(this, newState)
        val i = Intent(this, LinkForegroundService::class.java)
        startForegroundService(i)
        status = if (newState)
            "Automatic clipboard enabled ✔ (${if (ShizukuClipboard.ready()) "via Shizuku" else "via overlay"})."
        else
            "Automatic clipboard disabled."
    }

    private fun setupShizuku() {
        if (!ShizukuClipboard.isRunning()) {
            status = "Shizuku is not started. Install the Shizuku app, launch it, " +
                "then come back here. (See the instructions sent.)"
            return
        }
        if (ShizukuClipboard.ready()) {
            status = "Shizuku already connected ✔"
            return
        }
        rikka.shizuku.Shizuku.addRequestPermissionResultListener(shizukuPermListener)
        ShizukuClipboard.requestPermission()
    }

    private val shizukuPermListener =
        rikka.shizuku.Shizuku.OnRequestPermissionResultListener { _, result ->
            status = if (result == android.content.pm.PackageManager.PERMISSION_GRANTED) {
                ClipboardAutoSync.setEnabled(this, true)
                startForegroundService(Intent(this, LinkForegroundService::class.java))
                "Shizuku connected ✔ — reliable automatic clipboard enabled."
            } else {
                "Shizuku access denied."
            }
        }

    private fun pcVolume(action: String) {
        LinkBus.send(org.json.JSONObject().apply {
            put("type", "pc_volume")
            put("action", action)
        })
    }

    private fun pcMedia(action: String) {
        LinkBus.send(org.json.JSONObject().apply {
            put("type", "pc_media")
            put("action", action)
        })
    }

    private fun setupFolderSync() {
        if (!SyncFolder.hasAllFilesAccess()) {
            status = "Allow all-files access for Linux Link, then come back."
            startActivity(
                Intent(
                    android.provider.Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                    android.net.Uri.parse("package:$packageName")
                )
            )
            return
        }
        SyncFolder.setEnabled(this, true)
        SyncFolder.ensureDirs()
        startForegroundService(Intent(this, LinkForegroundService::class.java))
        status = "Sync enabled ✔ — Downloads, Documents, Pictures and LinuxLink " +
            "sync both ways."
    }

    private fun connectNow() {
        if (PairedPc.load(this) == null) {
            status = "Pair a PC first"
            return
        }
        startForegroundService(
            Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_CONNECT)
        )
        status = "Connecting…"
    }

    private fun dndAccessGranted(): Boolean =
        getSystemService(android.app.NotificationManager::class.java).isNotificationPolicyAccessGranted

    private fun setupDndSync() {
        if (dndAccessGranted()) {
            status = "Do Not Disturb sync already on ✔"
            startForegroundService(Intent(this, LinkForegroundService::class.java))
            return
        }
        status = "Allow Linux Link to change \"Do Not Disturb\", then come back."
        startActivity(Intent(android.provider.Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS))
    }

    private fun notificationAccessGranted(): Boolean =
        androidx.core.app.NotificationManagerCompat
            .getEnabledListenerPackages(this)
            .contains(packageName)

    private fun openNotificationAccessSettings() {
        startActivity(Intent(android.provider.Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
    }

    private fun testConnection() {
        val pc = PairedPc.load(this) ?: run {
            status = "Pair a PC first"
            return
        }
        status = "Ping to ${pc.name}…"
        uiScope.launch {
            try {
                val identity = Identity.loadOrCreate(this@MainActivity)
                val client = LinkClient(identity).also { it.expectedFingerprint = pc.fingerprint }
                client.connect(pc.lastAddress, pc.port)
                client.hello(Build.MODEL)
                val rtts = (0L..2L).map { client.ping(it) }
                client.close()
                status = "Connected to ${pc.name} — ping: ${rtts.joinToString(" / ")} ms"
            } catch (e: Exception) {
                status = "Failed: ${e.message}"
                Log.e(TAG, "ping", e)
            }
        }
    }

    companion object {
        private const val TAG = "MainActivity"
        const val LINK_SERVICE_UUID = "4c4c0001-6c69-6e75-786c-696e6b000001"
    }
}
