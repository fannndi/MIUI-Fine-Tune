package com.mifinetune.dynamic

import android.app.NotificationManager
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.content.ContextCompat
import com.mifinetune.core.Tuner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * Foreground service: Android lifecycle shell around the Rust daemon.
 *
 * Responsibility: FGS + notification, spawn/restart the daemon, forward
 * Android-only signals (screen, keyguard, ultra saver) as commands, relay
 * daemon events into [DynamicProfileState] + notifications.
 * Non-goals: any decision or tuning logic (all in the Rust daemon).
 */
class DynamicProfileService : Service() {

    companion object {
        private const val TAG = "MiFineTune"
        private const val SUPERVISE_MS = 3_000L
        private const val RESTART_BACKOFF_MS = 10_000L

        const val ACTION_START = "com.mifinetune.action.START"
        const val ACTION_STOP = "com.mifinetune.action.STOP"

        /** MIUI Ultra battery saver broadcast (framework owns the device). */
        const val ACTION_EXTREME = "miui.intent.action.EXTREME_POWER_SAVE_MODE_CHANGED"
        const val EXTRA_ENABLE = "enabled"

        fun start(context: Context) {
            val i = Intent(context, DynamicProfileService::class.java).setAction(ACTION_START)
            if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(i)
            else context.startService(i)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, DynamicProfileService::class.java))
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var config: DynamicProfileConfig
    private lateinit var reader: DeviceContextReader
    private lateinit var dnd: DndController
    private var client: DaemonClient? = null
    private var lastClientStart = 0L
    private var lastScreen: Boolean? = null
    private var lastLocked: Boolean? = null
    private var lastNotified: Pair<String, String>? = null
    private val labelCache = mutableMapOf<String, String>()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            Log.d(TAG, "receiver: ${intent.action}")
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> sendScreen()
                Intent.ACTION_SCREEN_ON -> sendScreen()
                Intent.ACTION_USER_PRESENT -> {
                    // keyguard dismissed: keep the reconcile dedupe in sync
                    // so the supervisor does not re-send screen and re-peek
                    lastScreen = true
                    lastLocked = false
                    client?.send(JSONObject().put("cmd", "user_present"))
                }
                ACTION_EXTREME ->
                    client?.send(
                        JSONObject()
                            .put("cmd", "ultra")
                            .put("on", intent.getBooleanExtra(EXTRA_ENABLE, false)),
                    )
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        config = DynamicProfileConfig.get(this)
        reader = DeviceContextReader(this)
        dnd = DndController(this)

        DaemonNotifications.createChannels(this)
        startForeground(
            DaemonNotifications.NOTIF_ID,
            DaemonNotifications.buildServiceNotification(this, "starting…"),
        )
        DynamicProfileState.running.value = true
        DaemonLink.client = null

        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
            addAction(ACTION_EXTREME)
        }
        ContextCompat.registerReceiver(this, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)

        startClient()
        startSupervisor()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            // "Turn off" must behave like the UI switch: restore stock first.
            scope.launch { restoreThenStop() }
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(receiver) }
        // safety: never leave a DND filter we set behind (the daemon also
        // releases on service-off, this covers an abrupt service kill)
        runCatching { dnd.restore() }
        DaemonLink.client = null
        client?.stop()
        client = null
        scope.cancel()
        DynamicProfileState.running.value = false
        DynamicProfileState.reason.value = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    // --- daemon lifecycle ---------------------------------------------------

    private fun startClient() {
        val c = DaemonClient(
            configPath = daemonConfigPath(this),
            onEvent = { ev -> handleEvent(ev) },
            onExit = {
                // restart is the supervisor's job (single restart path)
                Log.w(TAG, "daemon exited")
            },
            onLog = { line -> DynamicProfileState.pushLog(line) },
        )
        if (c.start()) {
            client = c
            DaemonLink.client = c
            lastClientStart = System.currentTimeMillis()
            // reconcile any DND filter left from a previous service run, then
            // tell the daemon whether the official-API bridge is usable
            dnd.restore()
            c.send(JSONObject().put("cmd", "hello"))
            c.send(JSONObject().put("cmd", "dnd_access").put("granted", dnd.isGranted()))
            // seed the diagnostics screen (on-demand, cheap)
            c.send(JSONObject().put("cmd", "diag"))
            c.send(JSONObject().put("cmd", "stats"))
            // force the full context on every (re)start: the dedupe state
            // belongs to the previous daemon instance
            lastScreen = null
            lastLocked = null
            sendScreen()
        } else {
            Log.w(TAG, "daemon start failed (root?)")
        }
    }

    /**
     * 3 s supervisor: screen/keyguard reconcile (MIUI sometimes drops the
     * broadcasts) + daemon restart with backoff.
     */
    private fun startSupervisor() {
        scope.launch {
            while (isActive) {
                delay(SUPERVISE_MS)
                sendScreen()
                val alive = client?.isAlive == true
                if (!alive) {
                    val now = System.currentTimeMillis()
                    if (now - lastClientStart > RESTART_BACKOFF_MS) {
                        Log.w(TAG, "daemon down — restarting")
                        startClient()
                    }
                }
            }
        }
    }

    /** Forwards the screen/keyguard state only when it changed. */
    private fun sendScreen() {
        val on = reader.screenOn
        val locked = reader.keyguardLocked
        if (on == lastScreen && locked == lastLocked) return
        lastScreen = on
        lastLocked = locked
        client?.send(
            JSONObject()
                .put("cmd", "screen")
                .put("on", on)
                .put("locked", locked),
        )
    }

    /** Restore stock via the daemon, then disable + stop. */
    private suspend fun restoreThenStop() {
        val c = client
        if (c != null && c.isAlive) {
            val seq = DynamicProfileState.currentRestoredSeq()
            c.send(JSONObject().put("cmd", "restore"))
            var waited = 0L
            while (waited < 8_000 && DynamicProfileState.currentRestoredSeq() == seq) {
                delay(100)
                waited += 100
            }
        }
        config.enabled = false
        Tuner.stopGuard()
        stopSelf()
    }

    // --- daemon events ------------------------------------------------------

    private fun handleEvent(ev: JSONObject) {
        when (ev.optString("event")) {
            "hello" -> Log.d(TAG, "daemon: hello v${ev.optInt("version")} pid=${ev.optInt("pid")}")

            "state" -> {
                val st = ev.getJSONObject("state")
                val active = st.optString("active").ifEmpty { null }
                DynamicProfileState.appliedProfile.value = active
                st.optString("foreground").ifEmpty { null }?.let {
                    DynamicProfileState.lastForeground.value = it
                }
                DynamicProfileState.secondWindow.value =
                    st.optString("second_window").ifEmpty { null }
                val rawReason = st.optString("reason").ifEmpty { null }
                val srcPkg = st.optString("src_pkg").ifEmpty { null }
                val reason = rawReason?.let { displayReason(it, srcPkg) }
                DynamicProfileState.reason.value = reason
                if (active != null) {
                    updateNotification(active, reason ?: "base")
                }
            }

            "applied" -> {
                val profile = ev.optString("profile")
                val rawReason = ev.optString("reason")
                val srcPkg = ev.optString("src_pkg").ifEmpty { null }
                val ok = ev.optBoolean("ok")
                val reason = displayReason(rawReason, srcPkg)
                DynamicProfileState.appliedProfile.value = profile
                DynamicProfileState.reason.value = reason
                if (ok) {
                    updateNotification(profile, reason)
                } else {
                    updateNotification(profile, "apply failed — open the app")
                }
                DynamicProfileState.pushApplied(
                    AppliedEvent(
                        seq = 0,
                        profile = profile,
                        reason = reason,
                        srcPkg = srcPkg,
                        ok = ok,
                        wrote = ev.optInt("wrote"),
                        verified = ev.optInt("verified"),
                        failed = ev.optInt("failed"),
                        ms = ev.optLong("ms"),
                        settleMs = ev.optLong("settle_ms"),
                    ),
                )
            }

            "bridge" -> DynamicProfileState.pushBridgeEvent(ev.optString("msg"))

            "dnd" -> {
                // decided by the daemon, executed here through the official
                // API (zen_mode is framework-owned; never written directly)
                val mode = ev.optString("mode").ifEmpty { null }
                if (mode != null) {
                    if (!dnd.apply(mode)) {
                        Log.w(TAG, "dnd requested ($mode) but access is not granted")
                    }
                } else {
                    dnd.restore()
                }
            }

            "env" -> DynamicProfileState.env.value = DiagnosticsParse.env(ev)

            "diag" -> DiagnosticsParse.diag(ev)?.let { DynamicProfileState.diag.value = it }

            "stats" -> {
                DynamicProfileState.stats.value = DiagnosticsParse.stats(ev)
                DynamicProfileState.heals.value = DiagnosticsParse.heals(ev)
            }

            "charge_once_done" -> {
                // the daemon consumed charge-to-100%-once (unplug seen):
                // clear the flag; the write hint re-syncs the daemon
                Log.d(TAG, "charge-once done (${ev.optInt("pct")}%)")
                config.chargeOnce = false
            }

            "game_mode_conflict" ->
                DaemonNotifications.notifyGameModeConflict(this, labelFor(ev.optString("pkg")))

            "restored" -> {
                DynamicProfileState.pushRestored(
                    RestoredEvent(
                        seq = 0,
                        retire = false,
                        ok = ev.optBoolean("ok"),
                        wrote = ev.optInt("wrote"),
                        verified = ev.optInt("verified"),
                        failed = ev.optInt("failed"),
                    ),
                )
            }

            "retired" -> {
                Log.d(TAG, "daemon retired (ultra saver)")
                DynamicProfileState.pushRestored(
                    RestoredEvent(
                        seq = 0,
                        retire = true,
                        ok = ev.optBoolean("ok"),
                        wrote = ev.optInt("wrote"),
                        verified = ev.optInt("verified"),
                        failed = ev.optInt("failed"),
                    ),
                )
                config.enabled = false
                Tuner.stopGuard()
                stopSelf()
            }

            "error" -> Log.w(TAG, "daemon error: ${ev.optString("msg")}")

            "pong", "bye" -> Log.d(TAG, "daemon: ${ev.optString("event")}")
        }
    }

    /** "app" resolves to the app label; everything else passes through. */
    private fun displayReason(raw: String, srcPkg: String?): String = when (raw) {
        "app" -> srcPkg?.let { labelFor(it) } ?: "app"
        else -> raw
    }

    private fun labelFor(pkg: String): String = labelCache.getOrPut(pkg) {
        runCatching {
            val pm = packageManager
            pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
        }.getOrDefault(pkg)
    }
    private fun updateNotification(profileId: String, reason: String) {
        // state events arrive on every visible change; the notification only
        // needs a re-post when the profile or the reason changed
        val key = profileId to reason
        if (lastNotified == key) return
        lastNotified = key
        getSystemService(NotificationManager::class.java).notify(
            DaemonNotifications.NOTIF_ID,
            DaemonNotifications.buildServiceNotification(this, "active · $profileId · $reason"),
        )
        // keep the Quick Settings tile subtitle in step with the profile
        ServiceTile.refresh(this)
    }

    /** One-shot conflict notice: MIUI Game Booster holds the tuned game. */
}
