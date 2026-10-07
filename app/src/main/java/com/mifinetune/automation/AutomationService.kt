package com.mifinetune.automation

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

/**
 * Foreground service that runs the automation loop:
 * screen events -> sleep profile, foreground app -> mapped/default profile.
 *
 * Responsibility: lifecycle + timers + applying the arbiter's decision.
 * Non-goals: deciding (ModeArbiter), engine IO (Tuner), persistence (AutomationConfig).
 */
class AutomationService : Service() {

    companion object {
        private const val TAG = "MiFineTune"
        private const val NOTIF_ID = 41
        private const val CHANNEL_ID = "automation"
        private const val SLEEP_DELAY_MS = 10_000L
        private const val POLL_MS = 1_500L

        const val ACTION_START = "com.mifinetune.action.START"
        const val ACTION_STOP = "com.mifinetune.action.STOP"
        const val ACTION_REFRESH = "com.mifinetune.action.REFRESH"

        fun start(context: Context) {
            val i = Intent(context, AutomationService::class.java).setAction(ACTION_START)
            if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(i)
            else context.startService(i)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, AutomationService::class.java))
        }

        /** Re-evaluate now (used after the user changes config from the UI). */
        fun refresh(context: Context) {
            context.startService(
                Intent(context, AutomationService::class.java).setAction(ACTION_REFRESH),
            )
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var config: AutomationConfig
    private lateinit var detector: ForegroundDetector
    private lateinit var reader: DeviceContextReader

    private var pollJob: Job? = null
    private var sleepJob: Job? = null
    private var screenOn = true
    private var locked = false
    private var charging = false
    private var pollTick = 0
    private var lastSeenPkg: String? = null
    private val labelCache = mutableMapOf<String, String>()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> onScreenOff()
                Intent.ACTION_SCREEN_ON -> onScreenOn()
                Intent.ACTION_USER_PRESENT -> {
                    locked = false
                    seedForeground()
                    startPolling()
                }
                Intent.ACTION_POWER_CONNECTED, Intent.ACTION_POWER_DISCONNECTED -> {
                    charging = reader.charging()
                    evaluate("power")
                }
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        config = AutomationConfig.get(this)
        reader = DeviceContextReader(this)
        detector = ForegroundDetector(this, Tuner.bridge)

        createChannel()
        startForeground(NOTIF_ID, buildNotification("menunggu…"))
        AutomationState.running.value = true

        screenOn = reader.screenOn
        locked = reader.keyguardLocked
        charging = reader.charging()

        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
            addAction(Intent.ACTION_POWER_CONNECTED)
            addAction(Intent.ACTION_POWER_DISCONNECTED)
        }
        ContextCompat.registerReceiver(this, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)

        scope.launch(Dispatchers.IO) {
            detector.ensureUsageAccess(Tuner.bridge)
            detector.reset()
            if (screenOn && !locked) {
                // seed the current foreground so a service that starts while
                // an app is already in front makes the right decision at once
                seedForeground()
                startPolling()
            } else {
                evaluate("start")
            }
        }
        Tuner.ensureGuard(scope)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_REFRESH -> scope.launch { evaluate("refresh") }
            else -> {}
        }
        return START_STICKY
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(receiver) }
        scope.cancel()
        AutomationState.running.value = false
        AutomationState.reason.value = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    // --- screen lifecycle -------------------------------------------------

    private fun onScreenOff() {
        screenOn = false
        stopPolling()
        lastSeenPkg = null
        AutomationState.lastForeground.value = null
        sleepJob = scope.launch {
            delay(SLEEP_DELAY_MS)
            if (!screenOn) evaluate("sleep")
        }
    }

    private fun onScreenOn() {
        screenOn = true
        sleepJob?.cancel()
        locked = reader.keyguardLocked
        if (!locked) {
            seedForeground()
            startPolling()
        }
    }

    /**
     * Right after wake/unlock the UsageStats stream may not carry a fresh
     * resume event (the activity was only paused). Seed the foreground once
     * via the root shell so the decision is deterministic.
     */
    private fun seedForeground() {
        scope.launch(Dispatchers.IO) {
            val fg = runCatching { detector.peekRoot() }.getOrNull() ?: return@launch
            if (fg != AutomationState.lastForeground.value) {
                AutomationState.lastForeground.value = fg
                lastSeenPkg = fg
            }
            evaluate("seed")
        }
    }

    // --- foreground polling ----------------------------------------------

    private fun startPolling() {
        if (pollJob?.isActive == true) return
        pollJob = scope.launch {
            while (isActive) {
                delay(POLL_MS)
                if (!screenOn || locked) continue
                pollTick++
                // UsageStats is a cheap fast path, but MIUI rarely delivers
                // resume events -> root-peek (dumpsys) every 2nd tick (~3 s)
                // is the dependable backbone.
                val fast = detector.poll()
                val fg = fast ?: if (pollTick % 2 == 0) detector.peekRoot() else null
                if (fg == null) continue
                AutomationState.lastForeground.value = fg
                if (fg != lastSeenPkg) {
                    lastSeenPkg = fg
                    evaluate("app")
                }
            }
        }
    }

    private fun stopPolling() {
        pollJob?.cancel()
        pollJob = null
    }

    // --- decision + apply -------------------------------------------------

    private fun evaluate(trigger: String) {
        if (!config.enabled) {
            stopSelf()
            return
        }

        // A manual card tap while automation runs sets overrideProfile; it
        // lasts until the next trigger, then the normal rules resume.
        val override = AutomationState.overrideProfile
        if (override != null) {
            AutomationState.overrideProfile = null
            Log.d(TAG, "manual override consumed ($override) at $trigger")
            return
        }

        val input = ArbiterInput(
            automationEnabled = true,
            screenOn = screenOn,
            keyguardLocked = locked,
            foregroundPkg = AutomationState.lastForeground.value,
            appMap = config.appMap(),
            defaultProfile = config.defaultProfile,
            sleepEnabled = config.sleepEnabled,
            sleepProfile = config.sleepProfile,
            skipOnMusic = config.skipOnMusic,
            musicActive = reader.musicActive,
            skipOnCharging = config.skipOnCharging,
            charging = charging,
        )

        when (val d = ModeArbiter.decide(input)) {
            is Decision.None -> Unit
            is Decision.Apply -> applyDecision(d)
        }
    }

    private fun applyDecision(d: Decision.Apply) {
        scope.launch {
            val reasonText = describe(d)
            val current = runCatching { Tuner.status().active }.getOrNull()
            if (current == d.profileId) {
                // already in place — only refresh visible state
                AutomationState.appliedProfile.value = d.profileId
                AutomationState.reason.value = reasonText
                updateNotification(d.profileId, reasonText)
                return@launch
            }
            runCatching { Tuner.apply(d.profileId) }
                .onSuccess { rep ->
                    if (rep.ok) {
                        AutomationState.appliedProfile.value = d.profileId
                        AutomationState.reason.value = reasonText
                        updateNotification(d.profileId, reasonText)
                        Tuner.ensureGuard(scope)
                    } else {
                        Log.w(TAG, "apply ${d.profileId} finished with failed=${rep.failed}")
                        AutomationState.reason.value = "gagal apply (${rep.failed} error)"
                        updateNotification(d.profileId, "gagal apply — cek aplikasi")
                    }
                }
                .onFailure { e ->
                    Log.w(TAG, "apply ${d.profileId} error: $e")
                    AutomationState.reason.value = "error: ${e.message}"
                    updateNotification(d.profileId, "error — cek aplikasi")
                }
        }
    }

    private fun describe(d: Decision.Apply): String = when (d.reason) {
        "default" -> "default harian"
        "layar mati" -> "layar mati"
        else -> AutomationState.lastForeground.value?.let { labelFor(it) } ?: d.profileId
    }

    private fun labelFor(pkg: String): String = labelCache.getOrPut(pkg) {
        runCatching {
            val pm = packageManager
            pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
        }.getOrDefault(pkg)
    }

    // --- notification -----------------------------------------------------

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        val ch = NotificationChannel(
            CHANNEL_ID,
            "Automation",
            NotificationManager.IMPORTANCE_MIN,
        ).apply {
            description = "Status automasi profile (senyap)"
            setShowBadge(false)
            enableVibration(false)
            setSound(null, null)
        }
        nm.createNotificationChannel(ch)
    }

    private fun buildNotification(status: String): Notification {
        val openPi = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stopPi = PendingIntent.getService(
            this, 1,
            Intent(this, AutomationService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_app)
            .setContentTitle("MiFineTune")
            .setContentText(status)
            .setOngoing(true)
            .setShowWhen(false)
            .setContentIntent(openPi)
            .addAction(0, "Matikan automasi", stopPi)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()
    }

    private fun updateNotification(profileId: String, reason: String) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIF_ID, buildNotification("auto · $profileId · $reason"))
    }
}
