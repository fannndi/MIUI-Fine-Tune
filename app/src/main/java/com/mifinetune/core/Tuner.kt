package com.mifinetune.core

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * Single owner of every `miui-ft` engine call, shared by the UI and the
 * dynamic profile service.
 *
 * Responsibility: serializing engine access (one apply at a time), hosting the
 * shared drift guard, exposing guard state.
 * Non-goals: deciding *which* profile to apply (ModeArbiter / UI do that).
 */
object Tuner {

    val bridge = RootBridge()
    val client = FtClient(bridge)

    /** Serializes apply/restore/verify across UI + service + guard. */
    private val mutex = Mutex()

    private var guardJob: Job? = null

    /** True while the periodic verify loop runs. */
    val guardActive = MutableStateFlow(false)

    /** Number of drifted keys corrected by the guard since app start. */
    val driftFixed = MutableStateFlow(0)

    suspend fun deploy(context: Context): String? = bridge.deploy(context)

    suspend fun status(): Status = withContext(Dispatchers.IO) { client.status() }

    suspend fun plan(profileId: String): Plan = withContext(Dispatchers.IO) { client.plan(profileId) }

    suspend fun apply(profileId: String): ApplyReport =
        mutex.withLock { withContext(Dispatchers.IO) { client.apply(profileId) } }

    /** Diagnostics only (read-only drift check); the guard applies directly. */
    suspend fun verify(profileId: String): ApplyReport =
        mutex.withLock { withContext(Dispatchers.IO) { client.verify(profileId) } }

    suspend fun restore(): ApplyReport =
        mutex.withLock { withContext(Dispatchers.IO) { client.restore() } }

    /**
     * Shared drift guard: every 15 s re-apply the active profile. The engine
     * skips unchanged keys (plan sees them as Unchanged) and only re-writes
     * keys the framework drifted (perf HAL boosts, PowerKeeper, network
     * stack) — one su round-trip per tick, repair decided inside the Rust
     * core. Idempotent — the first caller's scope owns the job; the loop
     * ends by itself when nothing is active.
     */
    fun ensureGuard(scope: CoroutineScope) {
        if (guardJob?.isActive == true) return
        guardJob = scope.launch(Dispatchers.IO) {
            guardActive.value = true
            while (isActive) {
                delay(15_000)
                val active = runCatching { client.status().active }.getOrNull() ?: continue
                val id = active ?: break
                runCatching { mutex.withLock { client.apply(id) } }
                    .onSuccess { rep ->
                        // re-written keys = keys the framework had drifted
                        if (rep.wrote > 0) driftFixed.value += rep.wrote
                    }
            }
            guardActive.value = false
        }
    }

    fun stopGuard() {
        guardJob?.cancel()
        guardJob = null
        guardActive.value = false
    }
}
