package com.mifinetune.dynamic

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.mifinetune.MainActivity
import com.mifinetune.R
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
        private const val NOTIF_ID = 41
        private const val CHANNEL_ID = "automation"
        private const val GM_CHANNEL_ID = "gmode"
        private const val GM_NOTIF_ID = 42

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
    private var client: DaemonClient? = null
    private var lastClientStart = 0L
    private var lastScreen: Boolean? = null
    private var lastLocked: Boolean? = null
    private val labelCache = mutableMapOf<String, String>()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            Log.d(TAG, "receiver: ${intent.action}")
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> sendScreen()
                Intent.ACTION_SCREEN_ON -> sendScreen()
                Intent.ACTION_USER_PRESENT -> client?.send(JSONObject().put("cmd", "user_present"))
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

        createChannel()
        startForeground(NOTIF_ID, buildNotification("starting…"))
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
        )
        if (c.start()) {
            client = c
            DaemonLink.client = c
            lastClientStart = System.currentTimeMillis()
            c.send(JSONObject().put("cmd", "hello"))
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

            "game_mode_conflict" -> notifyGameModeConflict(ev.optString("pkg"))

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

    // --- notifications ------------------------------------------------------

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        val ch = NotificationChannel(
            CHANNEL_ID,
            "Service",
            NotificationManager.IMPORTANCE_MIN,
        ).apply {
            description = "Dynamic profile status (silent)"
            setShowBadge(false)
            enableVibration(false)
            setSound(null, null)
        }
        nm.createNotificationChannel(ch)
        val gm = NotificationChannel(
            GM_CHANNEL_ID,
            "MIUI bridge warnings",
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply { description = "MIUI Game mode conflicts" }
        nm.createNotificationChannel(gm)
    }

    private fun buildNotification(status: String): Notification {
        val openPi = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stopPi = PendingIntent.getService(
            this, 1,
            Intent(this, DynamicProfileService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_app)
            .setContentTitle("MiFineTune")
            .setContentText(status)
            .setOngoing(true)
            .setShowWhen(false)
            .setContentIntent(openPi)
            .addAction(0, "Turn off", stopPi)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()
    }

    private fun updateNotification(profileId: String, reason: String) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIF_ID, buildNotification("active · $profileId · $reason"))
    }

    /** One-shot conflict notice: MIUI Game Booster holds the tuned game. */
    private fun notifyGameModeConflict(pkg: String) {
        val label = labelFor(pkg)
        val openPi = PendingIntent.getActivity(
            this, 2,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val n = NotificationCompat.Builder(this, GM_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_app)
            .setContentTitle("MIUI Game mode is boosting $label")
            .setContentText(
                "Our Game profile is already applied — exclude $label from " +
                    "MIUI Game Booster (or turn Game mode off) so it stays out of the way."
            )
            .setContentIntent(openPi)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(GM_NOTIF_ID, n)
    }
}
