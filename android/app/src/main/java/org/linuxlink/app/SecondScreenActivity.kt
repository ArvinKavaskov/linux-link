package org.linuxlink.app

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.util.Log
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.view.inputmethod.BaseInputConnection
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.io.DataInputStream
import java.io.EOFException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

/**
 * The tablet as a second monitor.
 *
 * The heavy lifting happens on the PC: it creates a real virtual monitor,
 * encodes it to H.264 and streams access units down the existing encrypted
 * QUIC connection. This activity is deliberately dumb — decode to a surface,
 * report every touch/pen/key back as a compact JSON line, reconnect when the
 * link hiccups. Latency lives and dies here, so the decoder runs in
 * low-latency mode and never queues more than the codec asks for.
 *
 * Two things are not dumb, because they cannot be: the geometry negotiation
 * (the PC may hand back a monitor of a different size than we asked for, and
 * the picture must then be letterboxed so pointer coordinates still land where
 * the user aimed) and the on-screen toolbar, which gives a keyboard-less
 * tablet the modifiers and the escape hatches a desktop takes for granted.
 */
class SecondScreenActivity : Activity(), SurfaceHolder.Callback {

    private lateinit var root: FrameLayout
    private lateinit var surface: SurfaceView
    private lateinit var status: TextView
    private lateinit var keyTarget: KeyTarget
    private lateinit var bar: LinearLayout

    private val running = AtomicBoolean(false)
    private var ioThread: Thread? = null
    private var channel: LinkClient.DisplayChannel? = null

    /** The monitor the PC actually created, in its own pixels. */
    @Volatile
    private var videoW = 0

    @Volatile
    private var videoH = 0

    /** Latched modifiers: evdev keycode → the key that latched it. */
    private val latched = LinkedHashMap<Int, TextView>()

    /** Input events leave on their own thread so touch handling never blocks. */
    private val sender = Executors.newSingleThreadExecutor()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
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
        // Invisible, but it is what the soft keyboard attaches to.
        keyTarget = KeyTarget(this)
        root.addView(keyTarget, FrameLayout.LayoutParams(1, 1))
        buildToolbar()

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

    // ---------------------------------------------------------- toolbar --

    /**
     * A translucent strip of the keys a tablet does not have. The modifiers
     * latch — tap Ctrl, then tap the screen, and the PC sees a Ctrl+click —
     * because there is no other way to hold a key while touching something
     * with the same hand.
     */
    private fun buildToolbar() {
        bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = pill(0xCC1B1B1FL.toInt())
            val p = dp(6)
            setPadding(p, p, p, p)
            visibility = View.GONE
        }
        for ((label, code) in MODIFIERS) {
            bar.addView(barKey(label, code, sticky = true))
        }
        for ((label, code) in ONE_SHOTS) {
            bar.addView(barKey(label, code, sticky = false))
        }
        bar.addView(action("⌨") { toggleKeyboard() })
        bar.addView(action("✕") { finish() })

        val handle = action("⋯") { bar.visibility = if (bar.visible()) View.GONE else View.VISIBLE }

        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(handle)
            addView(bar)
        }
        root.addView(
            row,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM or Gravity.START
            ).apply {
                val m = dp(8)
                setMargins(m, m, m, m)
            }
        )
    }

    private fun View.visible() = visibility == View.VISIBLE

    private fun barKey(label: String, code: Int, sticky: Boolean): TextView {
        val view = keyView(label)
        view.setOnClickListener {
            if (sticky) {
                if (latched.remove(code) != null) {
                    view.background = pill(KEY_BG)
                    sendKey(code, false)
                } else {
                    latched[code] = view
                    view.background = pill(KEY_ON)
                    sendKey(code, true)
                }
            } else {
                sendKey(code, true)
                sendKey(code, false)
            }
        }
        return view
    }

    private fun action(label: String, onTap: () -> Unit): TextView {
        val view = keyView(label)
        view.setOnClickListener { onTap() }
        return view
    }

    private fun keyView(label: String) = TextView(this).apply {
        text = label
        setTextColor(Color.WHITE)
        textSize = 13f
        gravity = Gravity.CENTER
        background = pill(KEY_BG)
        minWidth = dp(44)
        val px = dp(10)
        val py = dp(7)
        setPadding(px, py, px, py)
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.WRAP_CONTENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ).apply { marginEnd = dp(4) }
        isClickable = true
    }

    private fun pill(color: Int) = GradientDrawable().apply {
        shape = GradientDrawable.RECTANGLE
        cornerRadius = dp(10).toFloat()
        setColor(color)
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    /** Releases every latched modifier, so nothing stays stuck on the PC. */
    private fun clearLatched() {
        for ((code, view) in latched) {
            view.background = pill(KEY_BG)
            sendKey(code, false)
        }
        latched.clear()
    }

    private fun toggleKeyboard() {
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        keyTarget.isFocusableInTouchMode = true
        keyTarget.requestFocus()
        if (imm.isActive(keyTarget)) {
            imm.hideSoftInputFromWindow(keyTarget.windowToken, 0)
        } else {
            imm.showSoftInput(keyTarget, InputMethodManager.SHOW_IMPLICIT)
        }
    }

    /**
     * The view the IME talks to. Declaring `TYPE_NULL` asks the keyboard to
     * send raw key events rather than commit text: the PC then applies its own
     * layout, so an AZERTY desktop stays AZERTY even though Android believes
     * it is typing on something else.
     */
    private inner class KeyTarget(ctx: Context) : View(ctx) {
        init {
            isFocusable = true
            isFocusableInTouchMode = true
        }

        override fun onCheckIsTextEditor() = true

        override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
            outAttrs.inputType = InputType.TYPE_NULL
            outAttrs.imeOptions =
                EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_EXTRACT_UI
            return BaseInputConnection(this, false)
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
                val (w, h) = requestedSize()
                if (w == 0 || h == 0) {
                    if (!sleepQuietly(200)) return
                    continue
                }
                val ch = client.openDisplay(w, h, 60)
                channel = ch

                val header = readLine(ch.input)
                val reply = JSONObject(header)
                if (reply.optString("type") == "display_error") {
                    post("PC error: ${reply.optString("reason")}")
                    return
                }
                // The daemon rounds, clamps and sometimes flat out disagrees;
                // its answer is the truth the decoder and the pointer must use.
                videoW = reply.optInt("width", w)
                videoH = reply.optInt("height", h)
                fitVideo(videoW, videoH)
                post("")
                decode(ch, holder)
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                Log.w(TAG, "second screen session ended", e)
            }
            channel = null
            runOnUiThread { clearLatched() }
            if (running.get()) {
                post("Reconnecting…")
                if (!sleepQuietly(1_500)) return
            }
        }
    }

    /**
     * The size of monitor to ask for.
     *
     * The desktop treats the virtual monitor as an ordinary 1× output, so
     * asking a 300 dpi tablet for every one of its pixels produces microscopic
     * menus and a stream nobody's Wi-Fi enjoys. Past [NATIVE_LIMIT] the
     * request is halved, which is both a comfortable desktop size and an exact
     * integer scale on the way back up.
     */
    private fun requestedSize(): Pair<Int, Int> {
        var w = root.width
        var h = root.height
        if (w == 0 || h == 0) return 0 to 0
        if (maxOf(w, h) > NATIVE_LIMIT) {
            w /= 2
            h /= 2
        }
        return (w / 2) * 2 to (h / 2) * 2
    }

    /**
     * Letterboxes the surface to the monitor's aspect ratio. Without this a
     * 16:10 monitor stretched over a 4:3 tablet would put the pointer a
     * centimetre away from the finger that asked for it.
     */
    private fun fitVideo(vw: Int, vh: Int) = runOnUiThread {
        if (vw <= 0 || vh <= 0 || root.width == 0 || root.height == 0) return@runOnUiThread
        val scale = min(root.width.toFloat() / vw, root.height.toFloat() / vh)
        val lp = surface.layoutParams as FrameLayout.LayoutParams
        lp.width = (vw * scale).toInt()
        lp.height = (vh * scale).toInt()
        lp.gravity = Gravity.CENTER
        surface.layoutParams = lp
    }

    /** Feeds length-prefixed H.264 access units into a low-latency decoder. */
    private fun decode(ch: LinkClient.DisplayChannel, holder: SurfaceHolder) {
        val format = MediaFormat.createVideoFormat("video/avc", videoW, videoH).apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }
            // Realtime priority and an operating rate far above the actual
            // frame rate tell the codec to decode as fast as it can instead
            // of pacing itself against presentation timestamps.
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            setFloat(MediaFormat.KEY_OPERATING_RATE, 240f)
            // Vendor spellings of "low latency" for firmwares that predate or
            // ignore the standard key. Unknown keys are simply ignored, and
            // runCatching keeps any exotic OEM parser from hurting us.
            runCatching { setInteger("vendor.qti-ext-dec-low-latency.enable", 1) }
            runCatching { setInteger("vendor.rtc-ext-dec-low-latency.enable", 1) }
            runCatching { setInteger("vendor.hisi-ext-low-latency-video-dec.video-scene-for-low-latency-req", 1) }
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

    private fun sendKey(code: Int, down: Boolean) =
        send(JSONObject().apply { put("t", "ky"); put("c", code); put("d", down) })

    private val where = IntArray(2)

    /**
     * Window coordinates → the monitor's own [0,1] square. The surface is
     * letterboxed, so its offset inside the window matters; a touch on a black
     * band clamps to the nearest edge, exactly like a mouse hitting the border
     * of a screen.
     */
    private fun norm(x: Float, y: Float): Pair<Double, Double> {
        surface.getLocationInWindow(where)
        val w = surface.width.coerceAtLeast(1)
        val h = surface.height.coerceAtLeast(1)
        return ((x - where[0]) / w).toDouble().coerceIn(0.0, 1.0) to
            ((y - where[1]) / h).toDouble().coerceIn(0.0, 1.0)
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.getToolType(event.actionIndex)) {
            MotionEvent.TOOL_TYPE_STYLUS, MotionEvent.TOOL_TYPE_ERASER -> return handleStylus(event)
        }

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
        val down = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_POINTER_DOWN -> true
            else -> false
        }
        val inRange = event.actionMasked != MotionEvent.ACTION_HOVER_EXIT
        pen(event, down, inRange)
        return true
    }

    /**
     * One stylus sample. Android reports tilt as a single angle from the
     * vertical plus the direction it leans in; a digitizer wants that split
     * into two per-axis angles, which is the spherical-to-Cartesian turn
     * below. Positive X leans right, positive Y leans towards the user.
     */
    private fun pen(event: MotionEvent, down: Boolean, inRange: Boolean) {
        val i = if (event.actionIndex < event.pointerCount) event.actionIndex else 0
        val (u, v) = norm(event.getX(i), event.getY(i))
        val tilt = event.getAxisValue(MotionEvent.AXIS_TILT, i).toDouble()
        val orientation = event.getOrientation(i).toDouble()
        val lean = sin(tilt)
        val upright = cos(tilt)
        val tiltX = Math.toDegrees(atan2(sin(orientation) * lean, upright))
        val tiltY = Math.toDegrees(atan2(-cos(orientation) * lean, upright))
        val eraser = event.getToolType(i) == MotionEvent.TOOL_TYPE_ERASER
        val barrel = event.buttonState and
            (MotionEvent.BUTTON_STYLUS_PRIMARY or MotionEvent.BUTTON_SECONDARY) != 0
        send(JSONObject().apply {
            put("t", "pn")
            put("x", u); put("y", v)
            put("p", event.getPressure(i).toDouble().coerceIn(0.0, 1.0))
            put("d", down)
            put("tx", tiltX); put("ty", tiltY)
            put("e", eraser); put("bar", barrel); put("prox", inRange)
        })
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        val stylus = when (event.getToolType(0)) {
            MotionEvent.TOOL_TYPE_STYLUS, MotionEvent.TOOL_TYPE_ERASER -> true
            else -> false
        }
        when (event.actionMasked) {
            // A hovering stylus keeps the tablet tool in range: that is what
            // makes a brush preview follow the pen before it touches down.
            MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE,
            MotionEvent.ACTION_HOVER_EXIT -> {
                if (stylus) {
                    pen(event, down = false, inRange = event.actionMasked != MotionEvent.ACTION_HOVER_EXIT)
                } else {
                    val (u, v) = norm(event.x, event.y)
                    send(JSONObject().apply { put("t", "mv"); put("x", u); put("y", v) })
                }
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
            KeyEvent.ACTION_DOWN -> sendKey(code, true)
            KeyEvent.ACTION_UP -> sendKey(code, false)
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

    override fun onPause() {
        super.onPause()
        // Leaving with Ctrl held would poison the PC's keyboard state.
        clearLatched()
    }

    override fun onDestroy() {
        super.onDestroy()
        running.set(false)
        runCatching { channel?.output?.close() }
        sender.shutdown()
    }

    companion object {
        private const val TAG = "SecondScreen"

        /** Above this many pixels on the long edge, ask for half the tablet. */
        private const val NATIVE_LIMIT = 2048

        private val KEY_BG = 0xFF2E2E36L.toInt()
        private val KEY_ON = 0xFF3F7BE0L.toInt()

        /** Label → evdev keycode, for the keys that latch. */
        private val MODIFIERS = listOf(
            "Ctrl" to 29, "Alt" to 56, "Shift" to 42, "Super" to 125
        )

        /** Label → evdev keycode, for the keys that fire once. */
        private val ONE_SHOTS = listOf("Esc" to 1, "Tab" to 15)
    }
}
