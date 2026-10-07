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
 * Foreground service running the automation loop:
 * screen events -> sleep profile, foreground app -> mapped/base profile.
 *
 * Foreground detection is event-driven: a root `logcat -b events` stream of
 * `am_resume_activity` events (instant), with a dumpsys peek as fallback when
 * the stream is down.
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
        private const val SETTLE_MS = 700L
        private const val SUPERVISE_MS = 3_000L
        private const val WATCHER_RESTART_MS = 10_000L

        const val ACTION_START = "com.mifinetune.action.START"
        const val ACTION_STOP = "com.mifinetune.action.STOP"

        fun start(context: Context) {
            val i = Intent(context, AutomationService::class.java).setAction(ACTION_START)
            if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(i)
            else context.startService(i)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, AutomationService::class.java))
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var config: AutomationConfig
    private lateinit var watcher: ForegroundWatcher
    private lateinit var reader: DeviceContextReader

    private var sleepJob: Job? = null
    private var supervisorJob: Job? = null
    private var applyWorker: Job? = null
    private var pendingDecision: Decision.Apply? = null
    private var screenOn = true
    private var locked = false
    private var lastSeenPkg: String? = null
    private var lastRealPkg: String? = null
    private var lastWatcherRestart = 0L
    private val labelCache = mutableMapOf<String, String>()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            Log.d(TAG, "receiver: ${intent.action}")
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> onScreenOff()
                Intent.ACTION_SCREEN_ON -> onScreenOn()
                Intent.ACTION_USER_PRESENT -> {
                    locked = false
                    seedForeground()
                }
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        config = AutomationConfig.get(this)
        reader = DeviceContextReader(this)
        watcher = ForegroundWatcher(Tuner.bridge)

        createChannel()
        startForeground(NOTIF_ID, buildNotification("starting…"))
        AutomationState.running.value = true

        screenOn = reader.screenOn
        locked = reader.keyguardLocked
        Log.d(TAG, "service created: screenOn=$screenOn locked=$locked")

        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
        }
        ContextCompat.registerReceiver(this, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)

        watcher.start(scope) { pkg -> onForegroundEvent(pkg) }

        scope.launch(Dispatchers.IO) {
            if (screenOn && !locked) seedForeground() else evaluate("start")
        }
        startSupervisor()
        Tuner.ensureGuard(scope)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(receiver) }
        watcher.stop()
        scope.cancel()
        AutomationState.running.value = false
        AutomationState.reason.value = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    // --- screen lifecycle -------------------------------------------------

    private fun onScreenOff() {
        screenOn = false
        // keep tracking the foreground (events still arrive while locked) so
        // the unlock decision has the freshest non-transient package
        sleepJob = scope.launch {
            delay(SLEEP_DELAY_MS)
            if (!screenOn) {
                Log.d(TAG, "sleep timer fired")
                evaluate("sleep")
            }
        }
    }

    private fun onScreenOn() {
        screenOn = true
        sleepJob?.cancel()
        locked = reader.keyguardLocked
        if (!locked) seedForeground()
    }

    /**
     * Right after wake/unlock the event stream may not carry a fresh resume
     * (the activity was only paused). Seed the foreground once via a root peek
     * so the decision is deterministic.
     */
    private fun seedForeground() {
        scope.launch(Dispatchers.IO) {
            val peeked = runCatching { watcher.peekEvents() }.getOrNull()
            val best = when {
                peeked != null && !ModeArbiter.isTransient(peeked) -> peeked
                lastRealPkg != null -> lastRealPkg
                else -> peeked
            }
            Log.d(TAG, "seed: peeked=$peeked best=$best")
            if (best != null) {
                AutomationState.lastForeground.value = best
                lastSeenPkg = best
            }
            // Always re-evaluate after wake/unlock: a later resume event will
            // correct the guess instantly if it was wrong.
            evaluate("unlock")
        }
    }

    // --- foreground events ------------------------------------------------

    private fun onForegroundEvent(pkg: String) {
        if (!ModeArbiter.isTransient(pkg)) lastRealPkg = pkg
        if (pkg == lastSeenPkg) return
        lastSeenPkg = pkg
        AutomationState.lastForeground.value = pkg
        if (screenOn && !locked) evaluate("event")
    }

    /**
     * Supervises the event stream: while it is down, fall back to a root peek
     * every tick and try to restart the stream (bounded).
     */
    private fun startSupervisor() {
        if (supervisorJob?.isActive == true) return
        supervisorJob = scope.launch {
            var ticks = 0
            while (isActive) {
                delay(SUPERVISE_MS)
                ticks++
                if (ticks % 10 == 0) {
                    Log.d(TAG, "supervisor tick $ticks: watcherAlive=${watcher.isAlive} screenOn=$screenOn locked=$locked lastSeen=$lastSeenPkg")
                }
                if (watcher.isAlive) continue
                if (screenOn && !locked) {
                    val fg = runCatching { watcher.peekEvents() }.getOrNull()
                    if (fg != null && fg != lastSeenPkg) {
                        lastSeenPkg = fg
                        AutomationState.lastForeground.value = fg
                        evaluate("peek")
                    }
                }
                val now = System.currentTimeMillis()
                if (now - lastWatcherRestart > WATCHER_RESTART_MS) {
                    lastWatcherRestart = now
                    Log.w(TAG, "foreground stream down — restarting")
                    watcher.start(scope) { pkg -> onForegroundEvent(pkg) }
                }
            }
        }
    }

    // --- decision + apply -------------------------------------------------

    private fun evaluate(trigger: String) {
        if (!config.enabled) {
            stopSelf()
            return
        }
        val input = ArbiterInput(
            automationEnabled = true,
            screenOn = screenOn,
            keyguardLocked = locked,
            foregroundPkg = AutomationState.lastForeground.value,
            appMap = config.appMap(),
            baseProfile = config.baseProfile,
            sleepProfile = ModeArbiter.SLEEP_PROFILE,
        )
        when (val d = ModeArbiter.decide(input)) {
            is Decision.None -> Log.d(TAG, "evaluate($trigger): no-op")
            is Decision.Apply -> {
                Log.d(TAG, "evaluate($trigger): -> ${d.profileId} (${d.reason})")
                enqueue(d)
            }
        }
    }

    /**
     * Coalescing worker: bursts of foreground events (task restores, keyguard
     * transitions) collapse into a single apply of the *latest* decision after
     * a short settle window. While an apply runs, newer decisions supersede
     * the pending one instead of queueing behind it.
     */
    private fun enqueue(d: Decision.Apply) {
        pendingDecision = d
        if (applyWorker?.isActive == true) return
        applyWorker = scope.launch {
            while (true) {
                delay(SETTLE_MS)
                val decision = pendingDecision ?: break
                pendingDecision = null
                performApply(decision)
            }
        }
    }

    private suspend fun performApply(d: Decision.Apply) {
        val reasonText = describe(d)
        val current = runCatching { Tuner.status().active }.getOrNull()
        if (current == d.profileId) {
            // already in place — only refresh visible state
            AutomationState.appliedProfile.value = d.profileId
            AutomationState.reason.value = reasonText
            updateNotification(d.profileId, reasonText)
            return
        }
        runCatching { Tuner.apply(d.profileId) }
            .onSuccess { rep ->
                val finalRep = if (rep.ok) rep else {
                    // MIUI (Game Turbo/PowerKeeper) can race the apply with
                    // its own transient writes -> one retry usually wins.
                    Log.w(
                        TAG,
                        "apply ${d.profileId} failed=${rep.failed}: " +
                            rep.results.filter { it.error != null }
                                .joinToString { "${it.key}=${it.resolved}: ${it.error}" },
                    )
                    delay(2_000)
                    runCatching { Tuner.apply(d.profileId) }.getOrNull() ?: rep
                }
                if (finalRep.ok) {
                    AutomationState.appliedProfile.value = d.profileId
                    AutomationState.reason.value = reasonText
                    updateNotification(d.profileId, reasonText)
                    Tuner.ensureGuard(scope)
                } else {
                    Log.w(TAG, "apply ${d.profileId} failed after retry (${finalRep.failed})")
                    AutomationState.reason.value = "apply failed (${finalRep.failed})"
                    updateNotification(d.profileId, "apply failed — open the app")
                }
            }
            .onFailure { e ->
                Log.w(TAG, "apply ${d.profileId} error: $e")
                AutomationState.reason.value = "error: ${e.message}"
                updateNotification(d.profileId, "error — open the app")
            }
    }

    private fun describe(d: Decision.Apply): String = when (d.reason) {
        "base" -> "base"
        "screen off" -> "screen off"
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
            "Service",
            NotificationManager.IMPORTANCE_MIN,
        ).apply {
            description = "Automation status (silent)"
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
            .addAction(0, "Turn off", stopPi)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()
    }

    private fun updateNotification(profileId: String, reason: String) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIF_ID, buildNotification("active · $profileId · $reason"))
    }
}
