package com.mifinetune.core

import android.content.Context
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Root shell access + first-run deployment of the Rust core binary.
 *
 * Layout on device:
 *   /data/local/tmp/mifinetune/miui-ft   executable (W^X safe: app data is not)
 *   /data/adb/mifinetune/                state (profiles.json, state.json)
 *
 * Responsibility: process execution, asset deployment.
 * Non-goals: parsing miui-ft output (FtClient), any tuning decision.
 */
class RootBridge {

    data class Sh(val code: Int, val out: String) {
        val ok: Boolean get() = code == 0
    }

    /**
     * Absolute su path: apps don't inherit the shell PATH, so `su` by name
     * fails with ENOENT on some setups even though the binary exists.
     */
    private val suBin: String by lazy {
        listOf("/system/bin/su", "/sbin/su", "/system/xbin/su", "/data/adb/ap/bin/su")
            .firstOrNull { File(it).exists() } ?: "su"
    }

    /** Runs `<su> -c <cmd>`; stdout+stderr merged, bounded by [timeoutMs]. */
    fun sh(cmd: String, timeoutMs: Long = 30_000): Sh {
        val process = ProcessBuilder(suBin, "-c", cmd)
            .redirectErrorStream(true)
            .start()
        val out = process.inputStream.bufferedReader().use { it.readText() }
        val finished = process.waitFor(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS)
        if (!finished) {
            process.destroyForcibly()
            return Sh(124, "$out\n[timeout after ${timeoutMs}ms]")
        }
        return Sh(process.exitValue(), out)
    }

    /**
     * Starts `<su> -c <cmd>` as a long-lived process with stdout/stderr
     * separate (protocol on stdout, logs on stderr). Used by the daemon
     * client; the caller owns the process lifecycle.
     */
    fun spawn(cmd: String): Process = ProcessBuilder(suBin, "-c", cmd)
        .redirectErrorStream(false)
        .start()

    /** True when `su` gives us uid 0. */
    fun isRoot(): Boolean = sh("id -u 2>/dev/null").out.trim() == "0"

    /**
     * Deploy binary + bundled profiles. Returns null on success or an error
     * description (never throws) so the UI can show a precise reason.
     */
    suspend fun deploy(context: Context): String? = withContext(Dispatchers.IO) {
        runCatching {
            val binBytes = context.assets.open(ASSET_BIN).use { it.readBytes() }
            val target = File(BIN)
            // Content-aware deploy: v0.10 revert proved size-equal binaries
            // (version-bump builds) silently skip the copy (AGENTS hard rule
            // 7). cmp: same content -> skip; different -> copy + chmod.
            val tmp = File(context.cacheDir, ASSET_BIN)
            tmp.writeBytes(binBytes)
            val sameSize = target.exists() && target.length() == binBytes.size.toLong()
            val sameContent = sameSize && sh(
                "cmp -s '${tmp.absolutePath}' '$BIN' && echo SAME || echo DIFF"
            ).out.trim() == "SAME"
            if (!sameContent) {
                val r = sh(
                    "mkdir -p '${target.parentFile}' && " +
                        "cp '${tmp.absolutePath}' '$BIN' && chmod 755 '$BIN'"
                )
                if (!r.ok || !target.exists()) {
                    return@runCatching "Binary deploy failed (code ${r.code}): ${r.out.trim()}"
                }
            }

            val profBytes = context.assets.open(ASSET_PROFILES).use { it.readBytes() }
            val tmpP = File(context.cacheDir, ASSET_PROFILES)
            tmpP.writeBytes(profBytes)
            // keep a user-edited copy untouched; sync only when content differs
            val r2 = sh(
                "mkdir -p '$STATE_DIR' && chmod 700 '$STATE_DIR'; " +
                    "if ! cmp -s '${tmpP.absolutePath}' '$STATE_DIR/$ASSET_PROFILES'; then " +
                    "cp '${tmpP.absolutePath}' '$STATE_DIR/$ASSET_PROFILES'; fi"
            )
            if (!r2.ok) {
                return@runCatching "Profiles deploy failed (code ${r2.code}): ${r2.out.trim()}"
            }
            null
        }.getOrElse { "Deploy error: ${it.message ?: it}" }
    }

    companion object {
        const val BIN = "/data/local/tmp/mifinetune/miui-ft"
        const val STATE_DIR = "/data/adb/mifinetune"
        const val ASSET_BIN = "miui-ft"
        const val ASSET_PROFILES = "profiles.json"
    }
}
