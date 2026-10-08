package com.mifinetune.dynamic

import android.util.Log
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Tuner
import org.json.JSONObject
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.File
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * Thin client for the Rust daemon (`miui-ft serve`).
 *
 * Responsibility: process lifecycle, JSON-lines framing, log relay.
 * Non-goals: decisions (Rust daemon), UI state ([DynamicProfileState]).
 *
 * Protocol: one JSON object per line on stdin/stdout; stderr carries human
 * logs and is relayed to logcat under the `MiFineTune` tag (E2E greps).
 * Closing stdin is the shutdown signal — the daemon exits by itself.
 */
class DaemonClient(
    private val configPath: String,
    private val onEvent: (JSONObject) -> Unit,
    private val onExit: () -> Unit,
) {

    companion object {
        private const val TAG = "MiFineTune"
        private const val EXIT_GRACE_MS = 2_000L
    }

    @Volatile
    var isAlive: Boolean = false
        private set

    private var process: Process? = null
    private var writer: BufferedWriter? = null
    private val writeLock = Any()

    /** Spawns `su -c "miui-ft serve ..."` and starts the reader threads. */
    fun start(): Boolean {
        stop()
        val cmd = "${RootBridge.BIN} serve" +
            " --state-dir ${RootBridge.STATE_DIR}" +
            " --config $configPath"
        val p = runCatching { Tuner.bridge.spawn(cmd) }.getOrNull()
        if (p == null) {
            Log.w(TAG, "daemon: failed to spawn su process")
            return false
        }
        process = p
        writer = p.outputStream.bufferedWriter()
        isAlive = true

        thread(name = "daemon-stdout") {
            val reader: BufferedReader = p.inputStream.bufferedReader()
            runCatching {
                reader.forEachLine { line ->
                    val trimmed = line.trim()
                    if (trimmed.isEmpty()) return@forEachLine
                    val json = runCatching { JSONObject(trimmed) }.getOrNull()
                    if (json == null) {
                        Log.w(TAG, "daemon: bad json line: ${trimmed.take(200)}")
                    } else {
                        onEvent(json)
                    }
                }
            }
            isAlive = false
            Log.w(TAG, "daemon: stdout closed")
            onExit()
        }
        thread(name = "daemon-stderr") {
            runCatching {
                p.errorStream.bufferedReader().forEachLine { line ->
                    if (line.isNotBlank()) Log.d(TAG, line)
                }
            }
        }
        return true
    }

    /** Sends one command; false when the pipe is gone. */
    fun send(json: JSONObject): Boolean {
        val w = writer ?: return false
        return runCatching {
            synchronized(writeLock) {
                w.write(json.toString())
                w.newLine()
                w.flush()
            }
        }.isSuccess
    }

    /** Closes stdin (daemon exits by itself), then destroys after a grace. */
    fun stop() {
        val p = process ?: return
        process = null
        isAlive = false
        runCatching { writer?.close() }
        writer = null
        thread(name = "daemon-reaper") {
            val exited = runCatching { p.waitFor(EXIT_GRACE_MS, TimeUnit.MILLISECONDS) }
                .getOrDefault(false)
            if (!exited) {
                Log.w(TAG, "daemon: no clean exit, killing")
                runCatching { p.destroy() }
            }
        }
    }
}

/**
 * Process-wide handle the UI/config store uses to reach the running daemon.
 * The service sets/clears [client] on its lifecycle; everything else only
 * sends commands through here (no-op when the service is off).
 */
object DaemonLink {
    @Volatile
    var client: DaemonClient? = null

    fun connected(): Boolean = client?.isAlive == true

    fun send(json: JSONObject): Boolean = client?.send(json) ?: false

    /** Hint: config.json changed on disk — reload + re-evaluate now. */
    fun configChanged() {
        send(JSONObject().put("cmd", "config_changed"))
    }
}

/** Absolute path of the app-owned config.json. */
fun daemonConfigPath(context: android.content.Context): String =
    File(context.filesDir, "config.json").absolutePath
