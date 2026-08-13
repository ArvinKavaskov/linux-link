package org.linuxlink.app

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject
import tech.kwik.core.QuicClientConnection
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.URI
import java.security.MessageDigest
import java.time.Duration

class LinkClient(private val identity: Identity) {
    private var connection: QuicClientConnection? = null
    private var writer: java.io.OutputStream? = null
    private var reader: BufferedReader? = null

    var expectedFingerprint: String? = null

    suspend fun connect(host: String, port: Int, timeoutMillis: Long = 8_000) {
        withContext(Dispatchers.IO) {
            val conn = QuicClientConnection.newBuilder()
                .uri(URI.create("linuxlink://$host:$port"))
                .applicationProtocol("linuxlink/1")
                .noServerCertificateCheck()
                .clientCertificate(identity.certificate)
                .clientCertificateKey(identity.privateKey)
                .connectTimeout(Duration.ofMillis(timeoutMillis))
                .maxIdleTimeout(Duration.ofSeconds(90))
                .build()
            conn.connect()
            verifyServerFingerprint(conn)
            val stream = conn.createStream(true)
            connection = conn
            writer = stream.outputStream
            reader = BufferedReader(InputStreamReader(stream.inputStream))
        }
    }

    private fun verifyServerFingerprint(conn: QuicClientConnection) {
        val expected = expectedFingerprint ?: return
        val serverCert = conn.serverCertificateChain?.firstOrNull()
            ?: error("The PC presented no certificate. Refusing the connection.")
        val actual = MessageDigest.getInstance("SHA-256")
            .digest(serverCert.encoded)
            .joinToString("") { "%02x".format(it) }
        require(actual.equals(expected, ignoreCase = true)) {
            "Unexpected PC fingerprint: possible spoofing. Refusing the connection."
        }
    }

    private val ioMutex = Mutex()

    private suspend fun exchange(msg: JSONObject): JSONObject = withContext(Dispatchers.IO) {
        ioMutex.withLock {
            val w = writer ?: error("not connected")
            val r = reader ?: error("not connected")
            w.write((msg.toString() + "\n").toByteArray())
            w.flush()
            val line = r.readLine() ?: error("connection closed by the PC")
            JSONObject(line)
        }
    }

    suspend fun sendRaw(msg: JSONObject) {
        exchange(msg)
    }

    suspend fun sendFile(name: String, size: Long, input: java.io.InputStream) =
        withContext(Dispatchers.IO) {
            val conn = connection ?: error("not connected")
            val stream = conn.createStream(true)
            val out = stream.outputStream
            val header = JSONObject().apply {
                put("type", "file_start"); put("name", name); put("size", size)
            }
            out.write((header.toString() + "\n").toByteArray())
            input.use { it.copyTo(out, 64 * 1024) }
            out.flush()
            runCatching {
                BufferedReader(InputStreamReader(stream.inputStream)).readLine()
            }
        }

    suspend fun pullFile(id: String, size: Long, output: java.io.OutputStream) =
        withContext(Dispatchers.IO) {
            val conn = connection ?: error("not connected")
            val stream = conn.createStream(true)
            val out = stream.outputStream
            out.write(("{\"type\":\"file_pull\",\"id\":\"$id\"}\n").toByteArray())
            out.flush()
            val inp = stream.inputStream
            var remaining = size
            val buf = ByteArray(64 * 1024)
            output.use { dst ->
                while (remaining > 0) {
                    val want = minOf(buf.size.toLong(), remaining).toInt()
                    val n = inp.read(buf, 0, want)
                    if (n < 0) break
                    dst.write(buf, 0, n)
                    remaining -= n
                }
                dst.flush()
            }
        }

    suspend fun subscribe(onMessage: suspend (JSONObject) -> Unit): Nothing =
        withContext(Dispatchers.IO) {
            val conn = connection ?: error("not connected")
            val stream = conn.createStream(true)
            val w = stream.outputStream
            val r = BufferedReader(InputStreamReader(stream.inputStream))
            w.write("{\"type\":\"subscribe\"}\n".toByteArray())
            w.flush()
            r.readLine() ?: error("push channel refused")
            while (true) {
                val pushLine = r.readLine() ?: error("push channel closed by the PC")
                onMessage(JSONObject(pushLine))
                w.write("{\"type\":\"ok\"}\n".toByteArray())
                w.flush()
            }
            @Suppress("UNREACHABLE_CODE")
            error("unreachable")
        }

    suspend fun pair(token: String, deviceName: String): String {
        val reply = exchange(JSONObject().apply {
            put("type", "pair_request")
            put("version", 1)
            put("token", token)
            put("device_name", deviceName)
        })
        return when (reply.getString("type")) {
            "pair_ok" -> reply.getString("device_name")
            else -> error("pairing refused: ${reply.optString("reason", "unknown reason")}")
        }
    }

    suspend fun hello(deviceName: String): String {
        val reply = exchange(JSONObject().apply {
            put("type", "hello")
            put("version", 1)
            put("device_name", deviceName)
        })
        return when (reply.getString("type")) {
            "hello_ok" -> reply.getString("device_name")
            else -> error("PC not paired with this device")
        }
    }

    suspend fun ping(seq: Long): Long {
        val sentAt = System.currentTimeMillis()
        val reply = exchange(JSONObject().apply {
            put("type", "ping")
            put("seq", seq)
            put("sent_at_ms", sentAt)
        })
        check(reply.getString("type") == "pong")
        return System.currentTimeMillis() - sentAt
    }

    fun openWebcam(width: Int, height: Int): WebcamWriter {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val header = JSONObject().apply {
            put("type", "webcam_start"); put("width", width); put("height", height)
        }
        out.write((header.toString() + "\n").toByteArray())
        out.flush()
        return WebcamWriter(out)
    }

    fun openSpeaker(sampleRate: Int, channels: Int): java.io.InputStream {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val header = JSONObject().apply {
            put("type", "speaker_start"); put("sample_rate", sampleRate); put("channels", channels)
        }
        out.write((header.toString() + "\n").toByteArray())
        out.flush()
        return stream.inputStream
    }

    fun openPhoneAudio(sampleRate: Int, channels: Int): java.io.OutputStream {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val header = JSONObject().apply {
            put("type", "phone_audio_start"); put("sample_rate", sampleRate); put("channels", channels)
        }
        out.write((header.toString() + "\n").toByteArray())
        out.flush()
        return out
    }

    class DisplayChannel(val input: java.io.InputStream, val output: java.io.OutputStream)

    fun openDisplay(width: Int, height: Int, fps: Int): DisplayChannel {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val header = JSONObject().apply {
            put("type", "display_start"); put("width", width); put("height", height); put("fps", fps)
        }
        out.write((header.toString() + "\n").toByteArray())
        out.flush()
        return DisplayChannel(stream.inputStream, out)
    }

    fun openMic(sampleRate: Int, channels: Int): MicWriter {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val header = JSONObject().apply {
            put("type", "mic_start"); put("sample_rate", sampleRate); put("channels", channels)
        }
        out.write((header.toString() + "\n").toByteArray())
        out.flush()
        return MicWriter(out)
    }

    suspend fun runSync(folder: String, root: java.io.File) = withContext(Dispatchers.IO) {
        val conn = connection ?: error("not connected")
        val stream = conn.createStream(true)
        val out = stream.outputStream
        val inp = stream.inputStream
        root.mkdirs()

        fun writeLine(s: String) { out.write((s + "\n").toByteArray()); out.flush() }
        fun readLine(): String {
            val buf = java.io.ByteArrayOutputStream()
            while (true) {
                val b = inp.read()
                if (b < 0) error("sync stream closed")
                if (b == '\n'.code) break
                buf.write(b)
            }
            return buf.toString("UTF-8")
        }
        fun readExact(n: Int): ByteArray {
            val data = ByteArray(n); var off = 0
            while (off < n) {
                val r = inp.read(data, off, n - off)
                if (r < 0) error("stream truncated")
                off += r
            }
            return data
        }

        writeLine(JSONObject().apply {
            put("type", "sync_index"); put("folder", folder); put("files", SyncFolder.scan(root))
        }.toString())

        val plan = JSONObject(readLine())
        val pull = plan.optJSONArray("pull") ?: org.json.JSONArray()
        val delPhone = plan.optJSONArray("del_phone") ?: org.json.JSONArray()

        while (true) {
            val v = JSONObject(readLine())
            when (v.optString("type")) {
                "sync_file" -> {
                    val path = v.getString("path")
                    val size = v.getLong("size")
                    val bytes = readExact(size.toInt())
                    if (safeRel(path)) {
                        val dest = java.io.File(root, path)
                        dest.parentFile?.mkdirs()
                        dest.writeBytes(bytes)
                        v.optLong("mtime").let { if (it > 0) dest.setLastModified(it) }
                    }
                }
                "sync_push_end" -> break
                else -> break
            }
        }

        for (i in 0 until delPhone.length()) {
            java.io.File(root, delPhone.getString(i)).delete()
        }

        for (i in 0 until pull.length()) {
            val path = pull.getString(i)
            val f = java.io.File(root, path)
            val data = if (f.isFile) f.readBytes() else ByteArray(0)
            writeLine(JSONObject().apply {
                put("type", "sync_file"); put("path", path); put("size", data.size)
            }.toString())
            out.write(data); out.flush()
        }
        writeLine("{\"type\":\"sync_pull_end\"}")

        writeLine(JSONObject().apply {
            put("type", "sync_index2"); put("files", SyncFolder.scan(root))
        }.toString())

        readLine()
        runCatching { out.close() }
    }

    private fun safeRel(path: String): Boolean =
        path.isNotEmpty() && !path.startsWith('/') &&
            path.split('/').none { it == ".." || it == "." }

    fun close() {
        runCatching { connection?.close() }
        connection = null
        writer = null
        reader = null
    }
}

class MicWriter(private val out: java.io.OutputStream) {
    @Synchronized
    fun write(data: ByteArray, len: Int) {
        out.write(data, 0, len)
        out.flush()
    }
    fun close() { runCatching { out.close() } }
}

class WebcamWriter(private val out: java.io.OutputStream) {
    private val lenBuf = java.nio.ByteBuffer.allocate(4)

    @Synchronized
    fun sendFrame(jpeg: ByteArray) {
        lenBuf.clear()
        lenBuf.order(java.nio.ByteOrder.BIG_ENDIAN).putInt(jpeg.size)
        out.write(lenBuf.array())
        out.write(jpeg)
        out.flush()
    }

    fun close() {
        runCatching { out.close() }
    }
}
