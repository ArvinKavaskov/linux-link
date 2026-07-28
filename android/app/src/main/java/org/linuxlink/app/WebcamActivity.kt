package org.linuxlink.app

import android.graphics.ImageFormat
import android.graphics.Rect
import android.graphics.YuvImage
import android.os.Bundle
import android.util.Log
import android.util.Size
import android.widget.Button
import android.widget.LinearLayout
import android.widget.Toast
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

class WebcamActivity : androidx.activity.ComponentActivity() {

    private lateinit var previewView: PreviewView
    private val analysisExecutor = Executors.newSingleThreadExecutor()
    private var cameraProvider: ProcessCameraProvider? = null
    private var lensFacing = CameraSelector.LENS_FACING_BACK
    private var streaming = false
    private var webcam: WebcamWriter? = null

    @Volatile private var micThread: Thread? = null
    private var micWriter: MicWriter? = null
    private val micPermLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission()
    ) { granted -> if (granted) startMic() }

    private val targetSize = Size(1280, 720)

    private val cameraPermLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) startCamera()
        else { Toast.makeText(this, "Camera denied", Toast.LENGTH_SHORT).show(); finish() }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        previewView = PreviewView(this)
        val startBtn = Button(this).apply { text = "Start webcam" }
        val flipBtn = Button(this).apply { text = "Switch camera" }
        val micBtn = Button(this).apply { text = "🎤 Enable microphone" }

        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(previewView, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 3f))
            addView(startBtn, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 0f))
            addView(micBtn, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 0f))
            addView(flipBtn, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 0f))
        })

        micBtn.setOnClickListener {
            if (micThread == null) {
                if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
                    android.content.pm.PackageManager.PERMISSION_GRANTED) startMic()
                else micPermLauncher.launch(android.Manifest.permission.RECORD_AUDIO)
            } else {
                stopMic()
            }
            micBtn.text = if (micThread != null) "🎤 Mute microphone" else "🎤 Enable microphone"
        }

        startBtn.setOnClickListener {
            streaming = !streaming
            if (streaming) {
                val client = LinkForegroundService.activeClient
                if (client == null) {
                    Toast.makeText(this, "PC not connected", Toast.LENGTH_SHORT).show()
                    streaming = false
                } else {
                    runCatching { webcam = client.openWebcam(targetSize.width, targetSize.height) }
                        .onFailure {
                            Toast.makeText(this, "Unable to open stream: ${it.message}", Toast.LENGTH_SHORT).show()
                            streaming = false
                        }
                }
            } else {
                stopStream()
            }
            startBtn.text = if (streaming) "Stop webcam" else "Start webcam"
        }

        flipBtn.setOnClickListener {
            lensFacing = if (lensFacing == CameraSelector.LENS_FACING_BACK)
                CameraSelector.LENS_FACING_FRONT else CameraSelector.LENS_FACING_BACK
            bindCamera()
        }

        if (checkSelfPermission(android.Manifest.permission.CAMERA) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED) {
            startCamera()
        } else {
            cameraPermLauncher.launch(android.Manifest.permission.CAMERA)
        }
    }

    private fun startCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            cameraProvider = future.get()
            bindCamera()
        }, androidx.core.content.ContextCompat.getMainExecutor(this))
    }

    private fun bindCamera() {
        val provider = cameraProvider ?: return
        provider.unbindAll()

        val preview = Preview.Builder().build().also {
            it.setSurfaceProvider(previewView.surfaceProvider)
        }

        val analysis = ImageAnalysis.Builder()
            .setTargetResolution(targetSize)
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
            .also { runCatching { it.setOutputImageRotationEnabled(true) } }
            .build()
        analysis.setAnalyzer(analysisExecutor) { image -> onFrame(image) }

        val selector = CameraSelector.Builder().requireLensFacing(lensFacing).build()
        runCatching { provider.bindToLifecycle(this, selector, preview, analysis) }
            .onFailure { Log.e(TAG, "bindToLifecycle", it) }
    }

    private fun onFrame(image: ImageProxy) {
        try {
            if (streaming) {
                val jpeg = image.toJpeg(70)
                webcam?.sendFrame(jpeg)
            }
        } catch (e: Exception) {
            Log.w(TAG, "frame drop: ${e.message}")
        } finally {
            image.close()
        }
    }

    private fun stopStream() {
        webcam?.close()
        webcam = null
    }

    private fun startMic() {
        val client = LinkForegroundService.activeClient ?: run {
            Toast.makeText(this, "PC not connected", Toast.LENGTH_SHORT).show(); return
        }
        val rate = 48000
        val minBuf = android.media.AudioRecord.getMinBufferSize(
            rate, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT
        )
        val recorder = try {
            @Suppress("MissingPermission")
            android.media.AudioRecord(
                android.media.MediaRecorder.AudioSource.VOICE_COMMUNICATION, rate,
                android.media.AudioFormat.CHANNEL_IN_MONO,
                android.media.AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuf, 8192)
            )
        } catch (e: Exception) {
            Toast.makeText(this, "Microphone unavailable: ${e.message}", Toast.LENGTH_SHORT).show(); return
        }

        val sid = recorder.audioSessionId
        runCatching {
            if (android.media.audiofx.NoiseSuppressor.isAvailable())
                android.media.audiofx.NoiseSuppressor.create(sid)?.enabled = true
            if (android.media.audiofx.AutomaticGainControl.isAvailable())
                android.media.audiofx.AutomaticGainControl.create(sid)?.enabled = true
            if (android.media.audiofx.AcousticEchoCanceler.isAvailable())
                android.media.audiofx.AcousticEchoCanceler.create(sid)?.enabled = true
        }
        micWriter = runCatching { client.openMic(rate, 1) }.getOrElse {
            recorder.release(); Toast.makeText(this, "Microphone stream refused", Toast.LENGTH_SHORT).show(); return
        }
        recorder.startRecording()
        micThread = Thread {
            val buf = ByteArray(4096)
            while (!Thread.currentThread().isInterrupted) {
                val n = recorder.read(buf, 0, buf.size)
                if (n > 0) {
                    try { micWriter?.write(buf, n) } catch (e: Exception) { break }
                } else if (n < 0) break
            }
            runCatching { recorder.stop() }
            recorder.release()
        }.also { it.start() }
    }

    private fun stopMic() {
        micThread?.interrupt()
        micThread = null
        micWriter?.close()
        micWriter = null
    }

    override fun onDestroy() {
        stopStream()
        stopMic()
        analysisExecutor.shutdown()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "WebcamActivity"
    }
}

fun ImageProxy.toJpeg(quality: Int): ByteArray {
    val nv21 = yuv420ToNv21(this)
    val yuv = YuvImage(nv21, ImageFormat.NV21, width, height, null)
    val out = ByteArrayOutputStream()
    yuv.compressToJpeg(Rect(0, 0, width, height), quality, out)
    return out.toByteArray()
}

private fun yuv420ToNv21(image: ImageProxy): ByteArray {
    val w = image.width
    val h = image.height
    val ySize = w * h
    val nv21 = ByteArray(ySize + ySize / 2)

    val yPlane = image.planes[0]
    val uPlane = image.planes[1]
    val vPlane = image.planes[2]

    val yBuffer = yPlane.buffer
    val yRowStride = yPlane.rowStride
    if (yRowStride == w) {
        yBuffer.get(nv21, 0, ySize)
    } else {
        val row = ByteArray(yRowStride)
        for (r in 0 until h) {
            yBuffer.position(r * yRowStride)
            yBuffer.get(row, 0, w)
            System.arraycopy(row, 0, nv21, r * w, w)
        }
    }

    val uBuffer = uPlane.buffer
    val vBuffer = vPlane.buffer
    val uRowStride = uPlane.rowStride
    val vRowStride = vPlane.rowStride
    val uPixStride = uPlane.pixelStride
    val vPixStride = vPlane.pixelStride
    val chromaH = h / 2
    val chromaW = w / 2
    var out = ySize
    for (r in 0 until chromaH) {
        for (c in 0 until chromaW) {
            val uIndex = r * uRowStride + c * uPixStride
            val vIndex = r * vRowStride + c * vPixStride
            nv21[out++] = vBuffer.get(vIndex)
            nv21[out++] = uBuffer.get(uIndex)
        }
    }
    return nv21
}
