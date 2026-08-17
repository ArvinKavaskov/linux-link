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
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.Monitor
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VolumeDown
import androidx.compose.material.icons.filled.VolumeOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Shapes
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.UUID

private val LinkDark = darkColorScheme(
    primary = Color(0xFFF5F5F7),
    onPrimary = Color(0xFF111113),
    primaryContainer = Color(0xFF232326),
    onPrimaryContainer = Color(0xFFF5F5F7),
    secondary = Color(0xFFC9C9CE),
    onSecondary = Color(0xFF1A1A1C),
    secondaryContainer = Color(0xFF2C2C2E),
    onSecondaryContainer = Color(0xFFEDEDEF),
    tertiary = Color(0xFFC9C9CE),
    onTertiary = Color(0xFF1A1A1C),
    tertiaryContainer = Color(0xFF2C2C2E),
    onTertiaryContainer = Color(0xFFEDEDEF),
    background = Color(0xFF000000),
    onBackground = Color(0xFFF5F5F7),
    surface = Color(0xFF000000),
    onSurface = Color(0xFFF5F5F7),
    surfaceVariant = Color(0xFF1C1C1E),
    onSurfaceVariant = Color(0xFF98989E),
    surfaceContainerLowest = Color(0xFF0A0A0B),
    surfaceContainerLow = Color(0xFF141416),
    surfaceContainer = Color(0xFF1C1C1E),
    surfaceContainerHigh = Color(0xFF242427),
    surfaceContainerHighest = Color(0xFF2C2C2E),
    outline = Color(0xFF3A3A3D),
    outlineVariant = Color(0xFF2C2C2E),
    error = Color(0xFFE56B6B),
    onError = Color(0xFF1A1A1C),
)

private val LinkLight = lightColorScheme(
    primary = Color(0xFF111113),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFEDEDF0),
    onPrimaryContainer = Color(0xFF111113),
    secondary = Color(0xFF4A4A4F),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFF0F0F2),
    onSecondaryContainer = Color(0xFF1C1C1E),
    tertiary = Color(0xFF4A4A4F),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFF0F0F2),
    onTertiaryContainer = Color(0xFF1C1C1E),
    background = Color(0xFFF2F2F7),
    onBackground = Color(0xFF111113),
    surface = Color(0xFFF2F2F7),
    onSurface = Color(0xFF111113),
    surfaceVariant = Color(0xFFEAEAED),
    onSurfaceVariant = Color(0xFF6E6E73),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainer = Color(0xFFFAFAFB),
    surfaceContainerHigh = Color(0xFFF0F0F2),
    surfaceContainerHighest = Color(0xFFEAEAED),
    outline = Color(0xFFD6D6DA),
    outlineVariant = Color(0xFFE5E5E8),
    error = Color(0xFFC94141),
    onError = Color(0xFFFFFFFF),
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
    val colors = if (isSystemInDarkTheme()) LinkDark else LinkLight
    MaterialTheme(colorScheme = colors, shapes = LinkShapes, content = content)
}

class MainActivity : ComponentActivity() {
    private var status by mutableStateOf("No PC paired")
    private var pendingPairing: QrPayload? = null
    private val uiScope = CoroutineScope(Dispatchers.Main)

    private var speakerOn by mutableStateOf(false)
    private var micOn by mutableStateOf(false)

    private var pairedName by mutableStateOf<String?>(null)
    private var autoClipOn by mutableStateOf(false)
    private var shizukuOk by mutableStateOf(false)
    private var folderSyncOn by mutableStateOf(false)
    private var dndSyncOn by mutableStateOf(false)
    private var notifMirrorOn by mutableStateOf(false)
    private var knownPcs by mutableStateOf(listOf<PairedPc>())
    private var activeFp by mutableStateOf("")
    private var secondScreenOn by mutableStateOf(false)

    private val micPermLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) {
            audioAction(PhoneAudioService.ACTION_MIC_START)
            micOn = true
        }
    }

    private val projectionLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { res ->
        val data = res.data
        if (res.resultCode == RESULT_OK && data != null) {
            PhonePlaybackService.start(this, res.resultCode, data)
        } else {
            status = "Phone audio needs the capture permission"
        }
    }

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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        refreshState()
        PairedPc.load(this)?.let { status = "Paired with ${it.name}" }
        ensurePresenceObservation()

        setContent {
            LinkTheme {
                if (pairedName == null) OnboardingScreen() else HomeScreen()
            }
        }
    }

    @Composable
    private fun OnboardingScreen() {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(
                modifier = Modifier.fillMaxSize().padding(32.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Surface(
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.primaryContainer,
                ) {
                    Icon(
                        Icons.Filled.Link,
                        contentDescription = null,
                        modifier = Modifier.padding(24.dp).size(48.dp),
                        tint = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                }
                Spacer(Modifier.height(24.dp))
                Text(
                    "Linux Link",
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    "Your phone and your Linux PC, working as one.",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(32.dp))
                Button(
                    onClick = { startPairing() },
                    modifier = Modifier.fillMaxWidth().height(52.dp),
                    shape = MaterialTheme.shapes.large,
                ) {
                    Text("Pair your PC", style = MaterialTheme.typography.titleMedium)
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    "You will scan a code shown on the computer.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (status.startsWith("Pairing failed") || status.startsWith("Invalid")) {
                    Spacer(Modifier.height(16.dp))
                    Text(
                        status,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun HomeScreen() {
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
                ConnectionCard()

                if (secondScreenOn) {
                    ActionCard(
                        icon = Icons.Filled.Monitor,
                        title = "Second screen",
                        sub = "Use this device as an extra display for the PC",
                    ) {
                        if (LinkForegroundService.activeClient == null) {
                            status = "PC not connected"
                        } else {
                            startActivity(Intent(this@MainActivity, SecondScreenActivity::class.java))
                        }
                    }
                }

                SectionCard("Remote") {
                    val up = LinkForegroundService.linkUp
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        RemoteKey(Icons.Filled.SkipPrevious, "Previous track", up) { pcMedia("previous") }
                        RemoteKey(Icons.Filled.PlayArrow, "Play or pause", up) { pcMedia("play_pause") }
                        RemoteKey(Icons.Filled.SkipNext, "Next track", up) { pcMedia("next") }
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        RemoteKey(Icons.Filled.VolumeDown, "Volume down", up) { pcVolume("down") }
                        RemoteKey(Icons.Filled.VolumeOff, "Mute", up) { pcVolume("mute") }
                        RemoteKey(Icons.Filled.VolumeUp, "Volume up", up) { pcVolume("up") }
                    }
                }

                SectionCard("Clipboard") {
                    SwitchRow(
                        "Automatic clipboard",
                        "What you copy here appears on the PC",
                        autoClipOn,
                    ) { toggleAutoClipboard(); refreshState() }
                    StateRow(
                        "Shizuku",
                        "Makes the automatic clipboard fully reliable",
                        on = shizukuOk,
                        onLabel = "Connected",
                        offLabel = "Set up",
                    ) { setupShizuku() }
                    ActionRow(Icons.Filled.ContentCopy, "Send the clipboard now", null) {
                        startActivity(Intent(this@MainActivity, SendClipboardActivity::class.java))
                    }
                }

                SectionCard("Continuity") {
                    StateRow(
                        "Folder sync",
                        "Downloads, Documents and Pictures, both ways",
                        on = folderSyncOn,
                    ) { setupFolderSync(); refreshState() }
                    StateRow(
                        "Do Not Disturb sync",
                        "Silence one device, silence both",
                        on = dndSyncOn,
                    ) { setupDndSync() }
                    StateRow(
                        "Notification mirror",
                        "Phone notifications on the PC, with quick reply",
                        on = notifMirrorOn,
                    ) { openNotificationAccessSettings() }
                }

                SectionCard("Sound and camera") {
                    SwitchRow(
                        "Phone audio on PC",
                        "Media plays on the PC — some apps (WhatsApp) forbid capture",
                        PhonePlaybackService.running,
                    ) { togglePhoneAudio() }
                    SwitchRow(
                        "PC speaker",
                        "The PC plays its sound through this device",
                        speakerOn,
                    ) { toggleSpeaker() }
                    SwitchRow(
                        "PC microphone",
                        "This device becomes the PC's microphone",
                        micOn,
                    ) { toggleMic() }
                    ActionRow(Icons.Filled.Videocam, "Webcam", "Use the camera on the PC") {
                        startActivity(Intent(this@MainActivity, WebcamActivity::class.java))
                    }
                }

                SectionCard("Your PCs") {
                    for (pc in knownPcs) {
                        val active = pc.fingerprint.equals(activeFp, ignoreCase = true)
                        StateRow(
                            pc.name,
                            when {
                                active && LinkForegroundService.linkUp -> "Connected"
                                active -> "Active — not connected"
                                else -> "Tap to switch"
                            },
                            on = active,
                            onLabel = "Active",
                            offLabel = "",
                        ) { if (!active) switchToPc(pc) }
                    }
                    ActionRow(
                        Icons.Filled.QrCodeScanner,
                        "Pair another PC",
                        "Scan the code on the computer — adds it here",
                    ) { startPairing() }
                }

                SectionCard("This device") {
                    StateRow(
                        "Second screen",
                        if (AppPrefs.isTablet(this@MainActivity))
                            "On by default on tablets"
                        else
                            "Off by default on phones",
                        on = secondScreenOn,
                    ) {
                        AppPrefs.setSecondScreenEnabled(this@MainActivity, !secondScreenOn)
                        refreshState()
                    }
                }

                Text(
                    if (LinkForegroundService.linkUp)
                        "Linux Link · Connected to ${pairedName ?: "PC"} over LAN"
                    else
                        "Linux Link · Not connected",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                    textAlign = TextAlign.Center,
                )

                Spacer(Modifier.height(8.dp))
            }
        }
    }

    @Composable
    private fun ConnectionCard() {
        val up = LinkForegroundService.linkUp
        val dot by animateColorAsState(
            targetValue = if (up) Color(0xFF34C759) else MaterialTheme.colorScheme.outline,
            label = "connection dot",
        )
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
                Box(Modifier.size(12.dp).background(dot, CircleShape))
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        pairedName ?: "",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        if (up) "Connected" else "Not connected",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                if (!up) {
                    TextButton(onClick = { connectNow() }) { Text("Connect") }
                }
            }
            val note = status
            if (note.isNotBlank() && !note.startsWith("Paired with")) {
                Text(
                    note,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 16.dp),
                )
            }
        }
    }

    @Composable
    private fun SectionCard(title: String, content: @Composable ColumnScope.() -> Unit) {
        Card(modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large) {
            Column(
                modifier = Modifier.padding(vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(0.dp),
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 18.dp, top = 10.dp, bottom = 4.dp),
                )
                content()
            }
        }
    }

    @Composable
    private fun ActionCard(icon: ImageVector, title: String, sub: String, onClick: () -> Unit) {
        Card(
            modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
            shape = MaterialTheme.shapes.large,
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            ),
        ) {
            Row(
                modifier = Modifier.padding(18.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(14.dp))
                Column {
                    Text(title, style = MaterialTheme.typography.titleMedium)
                    Text(
                        sub,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }

    @Composable
    private fun RowScope.RemoteKey(
        icon: ImageVector,
        label: String,
        enabled: Boolean,
        onClick: () -> Unit,
    ) {
        FilledTonalIconButton(
            onClick = onClick,
            enabled = enabled,
            modifier = Modifier.weight(1f).height(48.dp),
            shape = MaterialTheme.shapes.small,
        ) {
            Icon(icon, contentDescription = label)
        }
    }

    @Composable
    private fun SwitchRow(
        title: String,
        sub: String?,
        checked: Boolean,
        onToggle: () -> Unit,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(horizontal = 18.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge)
                if (sub != null) {
                    Text(
                        sub,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.width(12.dp))
            Switch(checked = checked, onCheckedChange = { onToggle() })
        }
    }

    @Composable
    private fun StateRow(
        title: String,
        sub: String?,
        on: Boolean,
        onLabel: String = "On",
        offLabel: String = "Off",
        onClick: () -> Unit,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 18.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge)
                if (sub != null) {
                    Text(
                        sub,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.width(12.dp))
            Text(
                if (on) onLabel else offLabel,
                style = MaterialTheme.typography.labelLarge,
                color = if (on) Color(0xFF34C759) else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }

    @Composable
    private fun ActionRow(icon: ImageVector, title: String, sub: String?, onClick: () -> Unit) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 18.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                icon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(22.dp),
            )
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge)
                if (sub != null) {
                    Text(
                        sub,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }

    private fun togglePhoneAudio() {
        if (PhonePlaybackService.running) {
            PhonePlaybackService.stop(this)
            return
        }
        if (LinkForegroundService.activeClient == null) {
            status = "PC not connected"
            return
        }
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            micPermLauncher.launch(android.Manifest.permission.RECORD_AUDIO)
            status = "Allow the microphone, then flip the switch again"
            return
        }
        val mgr = getSystemService(MEDIA_PROJECTION_SERVICE)
                as android.media.projection.MediaProjectionManager
        projectionLauncher.launch(mgr.createScreenCaptureIntent())
    }

    private fun switchToPc(pc: PairedPc) {
        PairedPc.save(this, pc)
        status = "Switching to ${pc.name}…"
        refreshState()
        startService(
            Intent(this, LinkForegroundService::class.java)
                .setAction(LinkForegroundService.ACTION_RECONNECT)
        )
    }

    private fun refreshState() {
        PairedPc.load(this)?.let { KnownPcs.remember(this, it) }
        knownPcs = KnownPcs.list(this)
        activeFp = PairedPc.load(this)?.fingerprint ?: ""
        secondScreenOn = AppPrefs.secondScreenEnabled(this)
        pairedName = PairedPc.load(this)?.name
        autoClipOn = ClipboardAutoSync.isEnabled(this)
        shizukuOk = ShizukuClipboard.ready()
        folderSyncOn = SyncFolder.isEnabled(this) && SyncFolder.hasAllFilesAccess()
        dndSyncOn = dndAccessGranted()
        notifMirrorOn = notificationAccessGranted()
        speakerOn = PhoneAudioService.speakerOn
        micOn = PhoneAudioService.micOn
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

    override fun onResume() {
        super.onResume()
        refreshState()
    }

    private fun audioAction(action: String) {
        val intent = Intent(this, PhoneAudioService::class.java).setAction(action)
        if (action == PhoneAudioService.ACTION_SPEAKER_START ||
            action == PhoneAudioService.ACTION_MIC_START
        ) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun toggleSpeaker() {
        if (speakerOn) {
            audioAction(PhoneAudioService.ACTION_SPEAKER_STOP)
            speakerOn = false
            return
        }
        if (LinkForegroundService.activeClient == null) {
            status = "PC not connected"
            return
        }
        audioAction(PhoneAudioService.ACTION_SPEAKER_START)
        speakerOn = true
        status = "Select “Phone (Linux Link)” as output on the PC"
    }

    private fun toggleMic() {
        if (micOn) {
            audioAction(PhoneAudioService.ACTION_MIC_STOP)
            micOn = false
            return
        }
        if (LinkForegroundService.activeClient == null) {
            status = "PC not connected"
            return
        }
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            audioAction(PhoneAudioService.ACTION_MIC_START)
            micOn = true
            status = "Select “Linux Link” as microphone on the PC"
        } else {
            micPermLauncher.launch(android.Manifest.permission.RECORD_AUDIO)
        }
    }

    private fun pairOverWifi(payload: QrPayload) {
        status = "Connecting to ${payload.name}…"
        uiScope.launch {
            try {
                val identity = Identity.loadOrCreate(this@MainActivity)
                val client = LinkClient(identity).also { it.expectedFingerprint = payload.fingerprint }
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

                val pc = PairedPc(pcName, address, payload.port, payload.fingerprint)
                PairedPc.save(this@MainActivity, pc)
                KnownPcs.remember(this@MainActivity, pc)
                PcLocator.rememberAlternates(
                    this@MainActivity, payload.addrs.filter { it != address }
                )
                refreshState()
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

    companion object {
        private const val TAG = "MainActivity"
        const val LINK_SERVICE_UUID = "4c4c0001-6c69-6e75-786c-696e6b000001"
    }
}
