package org.linuxlink.app

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import org.json.JSONObject
import java.io.DataInputStream
import java.io.EOFException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The tablet as a second monitor.
 *
 * The heavy lifting happens on the PC: it creates a real virtual monitor,
 * encodes it to H.264 and streams access units down the existing encrypted
 * QUIC connection. This activity is deliberately dumb — decode to a surface,
 * report every touch/pen/key back as a compact JSON line, reconnect when the
 * link hiccups. Latency lives and dies here, so the decoder runs in
 * low-latency mode and never queues more than the codec asks for.
 */
class SecondScreenActivity : Activity(), SurfaceHolder.Callback {

    private lateinit var surface: SurfaceView
    private lateinit var status: TextView

    private val running = AtomicBoolean(false)
    private var ioThread: Thread? = null
    private var channel: LinkClient.DisplayChannel? = null

    /** Input events leave on their own thread so touch handling never blocks. */
    private val sender = Executors.newSingleThreadExecutor()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
        surface = SurfaceView(this)
        root.addView(
            surface,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT
            )
        )
        status = TextView(this).apply {
            text = "Connecting to the PC…"
            setTextColor(Color.WHITE)
            textSize = 16f
            gravity = Gravity.CENTER
        }
        root.addView(
            status,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.CENTER
            )
        )
        setContentView(root)
        hideSystemBars()
        surface.holder.addCallback(this)
    }

    private fun hideSystemBars() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false)
            window.insetsController?.let {
                it.hide(android.view.WindowInsets.Type.systemBars())
                it.systemBarsBehavior =
                    android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            }
        } else {
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility =
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY or
                View.SYSTEM_UI_FLAG_FULLSCREEN or
                View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
        }
    }

    // ------------------------------------------------------------ session --

    override fun surfaceCreated(holder: SurfaceHolder) {
        running.set(true)
        ioThread = Thread { sessionLoop(holder) }.apply {
            name = "second-screen-io"
            start()
        }
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        running.set(false)
        runCatching { channel?.output?.close() }
        ioThread?.interrupt()
        ioThread = null
    }

    /**
     * Connect → decode until the stream dies → reconnect. IO errors mean the
     * link blinked (PC rebooting, Wi-Fi roaming) and deserve a retry; an
     * explicit `display_error` from the daemon means retrying cannot help.
     */
    private fun sessionLoop(holder: SurfaceHolder) {
        while (running.get()) {
            val client = LinkForegroundService.activeClient
            if (client == null) {
                post("PC not connected — waiting…")
                if (!sleepQuietly(1_500)) return
                continue
            }
            try {
                val w = (surface.width / 2) * 2
                val h = (surface.height / 2) * 2
                val ch = client.openDisplay(w, h, 60)
                channel = ch

                val header = readLine(ch.input)
                val reply = JSONObject(header)
                if (reply.optString("type") == "display_error") {
                    post("PC error: ${reply.optString("reason")}")
                    return
                }
                post("")
                decode(ch, holder)
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                Log.w(TAG, "second screen session ended", e)
            }
            channel = null
            if (running.get()) {
                post("Reconnecting…")
                if (!sleepQuietly(1_500)) return
            }
        }
    }

    /** Feeds length-prefixed H.264 access units into a low-latency decoder. */
    private fun decode(ch: LinkClient.DisplayChannel, holder: SurfaceHolder) {
        val format = MediaFormat.createVideoFormat("video/avc", surface.width, surface.height).apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }
        }
        val codec = MediaCodec.createDecoderByType("video/avc")
        try {
            codec.configure(format, holder.surface, null, 0)
            codec.start()

            val drain = Thread {
                val info = MediaCodec.BufferInfo()
                while (running.get()) {
                    try {
                        val index = codec.dequeueOutputBuffer(info, 10_000)
                        if (index >= 0) codec.releaseOutputBuffer(index, true)
                    } catch (e: Exception) {
                        break
                    }
                }
            }.apply { name = "second-screen-drain"; start() }

            val input = DataInputStream(ch.input)
            val lenBuf = ByteArray(4)
            var pts = 0L
            while (running.get()) {
                input.readFully(lenBuf)
                val len = ((lenBuf[0].toInt() and 0xFF) shl 24) or
                    ((lenBuf[1].toInt() and 0xFF) shl 16) or
                    ((lenBuf[2].toInt() and 0xFF) shl 8) or
                    (lenBuf[3].toInt() and 0xFF)
                if (len <= 0 || len > 16 * 1024 * 1024) throw EOFException("bad frame length $len")
                val unit = ByteArray(len)
                input.readFully(unit)

                val index = codec.dequeueInputBuffer(100_000)
                if (index < 0) continue // codec busy — drop, never queue latency
                codec.getInputBuffer(index)?.apply { clear(); put(unit) }
                codec.queueInputBuffer(index, 0, len, pts, 0)
                pts += 16_666
            }
            drain.join(500)
        } finally {
            runCatching { codec.stop() }
            runCatching { codec.release() }
        }
    }

    /** One text line before the binary stream starts. */
    private fun readLine(input: java.io.InputStream): String {
        val sb = StringBuilder()
        while (true) {
            val b = input.read()
            if (b == -1) throw EOFException("stream closed during handshake")
            if (b == '\n'.code) return sb.toString()
            sb.append(b.toChar())
        }
    }

    // -------------------------------------------------------------- input --

    private fun send(obj: JSONObject) {
        val ch = channel ?: return
        sender.execute {
            runCatching {
                ch.output.write((obj.toString() + "\n").toByteArray())
                ch.output.flush()
            }
        }
    }

    private fun norm(x: Float, y: Float): Pair<Double, Double> {
        val w = surface.width.coerceAtLeast(1)
        val h = surface.height.coerceAtLeast(1)
        return (x / w).toDouble().coerceIn(0.0, 1.0) to (y / h).toDouble().coerceIn(0.0, 1.0)
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        val stylus = event.getToolType(event.actionIndex) == MotionEvent.TOOL_TYPE_STYLUS
        if (stylus) return handleStylus(event)

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val i = event.actionIndex
                touch(event.getPointerId(i), 0, event.getX(i), event.getY(i))
            }
            MotionEvent.ACTION_MOVE -> {
                for (i in 0 until event.pointerCount) {
                    touch(event.getPointerId(i), 1, event.getX(i), event.getY(i))
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val i = event.actionIndex
                touch(event.getPointerId(i), 2, event.getX(i), event.getY(i))
            }
            MotionEvent.ACTION_CANCEL -> {
                for (i in 0 until event.pointerCount) {
                    touch(event.getPointerId(i), 3, event.getX(i), event.getY(i))
                }
            }
        }
        return true
    }

    private fun touch(id: Int, phase: Int, x: Float, y: Float) {
        val (u, v) = norm(x, y)
        send(JSONObject().apply {
            put("t", "tc"); put("id", id); put("ph", phase); put("x", u); put("y", v)
        })
    }

    private fun handleStylus(event: MotionEvent): Boolean {
        val (u, v) = norm(event.x, event.y)
        val down = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE -> true
            else -> false
        }
        send(JSONObject().apply {
            put("t", "pn"); put("x", u); put("y", v)
            put("p", event.pressure.toDouble().coerceIn(0.0, 1.0)); put("d", down)
        })
        return true
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            // Stylus hover and attached mice both preview the cursor.
            MotionEvent.ACTION_HOVER_MOVE -> {
                val (u, v) = norm(event.x, event.y)
                send(JSONObject().apply { put("t", "mv"); put("x", u); put("y", v) })
                return true
            }
            MotionEvent.ACTION_SCROLL -> {
                val dy = -event.getAxisValue(MotionEvent.AXIS_VSCROLL) * 40
                val dx = -event.getAxisValue(MotionEvent.AXIS_HSCROLL) * 40
                send(JSONObject().apply {
                    put("t", "sc"); put("dx", dx.toDouble()); put("dy", dy.toDouble())
                })
                return true
            }
        }
        return super.onGenericMotionEvent(event)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // The system keeps its own keys; a hardware keyboard types on the PC.
        when (event.keyCode) {
            KeyEvent.KEYCODE_BACK, KeyEvent.KEYCODE_HOME,
            KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_VOLUME_DOWN,
            KeyEvent.KEYCODE_APP_SWITCH -> return super.dispatchKeyEvent(event)
        }
        val code = KeyMap.toLinux(event.keyCode) ?: return super.dispatchKeyEvent(event)
        when (event.action) {
            KeyEvent.ACTION_DOWN -> send(JSONObject().apply { put("t", "ky"); put("c", code); put("d", true) })
            KeyEvent.ACTION_UP -> send(JSONObject().apply { put("t", "ky"); put("c", code); put("d", false) })
        }
        return true
    }

    // ------------------------------------------------------------- plumbing --

    private fun post(text: String) {
        runOnUiThread {
            status.text = text
            status.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
        }
    }

    private fun sleepQuietly(ms: Long): Boolean = try {
        Thread.sleep(ms)
        true
    } catch (e: InterruptedException) {
        false
    }

    override fun onDestroy() {
        super.onDestroy()
        running.set(false)
        runCatching { channel?.output?.close() }
        sender.shutdown()
    }

    companion object {
        private const val TAG = "SecondScreen"
    }
}
