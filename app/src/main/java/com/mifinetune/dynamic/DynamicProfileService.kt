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
import com.mifinetune.miui.MiBridgeState
import com.mifinetune.miui.MiStateBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Foreground service running the dynamic profile loop:
 * screen events -> sleep profile, foreground app -> mapped/base profile.
 *
 * Foreground detection is event-driven: a root `logcat -b events` stream of
 * `am_resume_activity` events (instant), with a dumpsys peek as fallback when
 * the stream is down.
 *
 * Responsibility: lifecycle + timers + applying the arbiter's decision.
 * Non-goals: deciding (ModeArbiter), engine IO (Tuner), persistence (DynamicProfileConfig).
 */
class DynamicProfileService : Service() {

    companion object {
        private const val TAG = "MiFineTune"
        private const val NOTIF_ID = 41
        private const val CHANNEL_ID = "automation"
        private const val GM_CHANNEL_ID = "gmode"
        private const val GM_NOTIF_ID = 42
        private const val SLEEP_DELAY_MS = 10_000L

        /**
         * Coalescing window: one app transition emits its event pair within
         * ~50 ms, so this only needs to absorb bursts — not deliberate
         * user hops. 400 ms measured on device 2026-10-08: same final state,
         * ~300 ms faster than the old 700 ms, no extra applies.
         */
        private const val SETTLE_MS = 400L
        private const val SUPERVISE_MS = 3_000L
        private const val WATCHER_RESTART_MS = 10_000L

        const val ACTION_START = "com.mifinetune.action.START"
        const val ACTION_STOP = "com.mifinetune.action.STOP"

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
    private lateinit var watcher: ForegroundWatcher
    private lateinit var reader: DeviceContextReader
    private lateinit var miState: MiStateBridge
    private val mwWatcher = MultiWindowWatcher(Tuner.bridge)
    private var multiWindow = false

    private var sleepJob: Job? = null
    private var supervisorJob: Job? = null
    private var applyWorker: Job? = null
    private var pendingDecision: Decision.Apply? = null
    private var pendingSince = 0L
    private var screenOn = true
    private var locked = false
    private var lastSeenPkg: String? = null
    private var lastRealPkg: String? = null
    private var lastWatcherRestart = 0L
    private var retired = false
    private var gameModeWarnedFor: String? = null
    private var pendingSaver = false
    private val labelCache = mutableMapOf<String, String>()

    /** Bridge timeline entry (also visible in Settings → MIUI bridge). */
    private fun logEvent(msg: String) {
        Log.d(TAG, "bridge: $msg")
        DynamicProfileState.pushBridgeEvent(msg)
    }

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
        config = DynamicProfileConfig.get(this)
        reader = DeviceContextReader(this)
        watcher = ForegroundWatcher(Tuner.bridge)
        miState = MiStateBridge(this, Tuner.bridge)

        createChannel()
        startForeground(NOTIF_ID, buildNotification("starting…"))
        DynamicProfileState.running.value = true

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
        miState.start()
        recoverBridge()

        // Dynamic Profile toggle: react instantly. The foreground at toggle
        // time is our own app (transient) — use the last real package so the
        // decision flips the profile in front of the user right away.
        scope.launch {
            config.dynamicEnabledFlow.drop(1).collect {
                evaluate("dynamic", fgOverride = lastRealPkg ?: DynamicProfileState.lastForeground.value)
            }
        }
        mwWatcher.start(scope) { st ->
            if (st.active != multiWindow) {
                multiWindow = st.active
                DynamicProfileState.secondWindow.value = st.otherPkg
                logEvent(if (st.active) "multi-window ON (${st.otherPkg})" else "multi-window off")
                evaluate("multiwindow")
            }
        }

        scope.launch(Dispatchers.IO) {
            if (screenOn && !locked) seedForeground() else evaluate("start")
        }
        startSupervisor()
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
        mwWatcher.stop()
        // release bridge holds on a detached thread: scope is about to be
        // cancelled and the su writes take ~0.5 s. Prefs carry the restore
        // points regardless, so an abrupt death still self-heals next start.
        Thread {
            runCatching { releaseBridgeHolds() }
            runCatching { miState.stop() }
        }.start()
        scope.cancel()
        DynamicProfileState.running.value = false
        DynamicProfileState.reason.value = null
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
                DynamicProfileState.lastForeground.value = best
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
        DynamicProfileState.lastForeground.value = pkg
        if (screenOn && !locked) evaluate("event")
    }

    /**
     * Supervises the event stream: while it is down, fall back to a root peek
     * every tick and try to restart the stream (bounded). Also reconciles the
     * screen state: MIUI sometimes drops the SCREEN_ON broadcast entirely,
     * so the supervisor re-reads `PowerManager.isInteractive` directly —
     * broadcast-free truth every tick.
     *
     * Every 15 s it re-evaluates the arbiter periodicaliy (service-driven
     * drift guard): the decision is recomputed fresh (saver state, mapping,
     * base) and the coalescing apply skips unchanged keys — a broadcast or
     * apply hiccup self-heals within one period. The dynamic profile service is
     * the only writer here; the legacy Tuner guard stays for manual-only use.
     */
    private fun startSupervisor() {
        if (supervisorJob?.isActive == true) return
        supervisorJob = scope.launch {
            var ticks = 0
            while (isActive) {
                delay(SUPERVISE_MS)
                ticks++
                if (ticks % 5 == 0 && screenOn && !locked) evaluate("periodic")
                if (ticks % 10 == 0) {
                    Log.d(TAG, "supervisor tick $ticks: watcherAlive=${watcher.isAlive} mw=${mwWatcher.isAlive} screenOn=$screenOn locked=$locked lastSeen=$lastSeenPkg")
                }
                // screen-state reconciliation (missed SCREEN_ON/OFF recovery)
                val actualOn = reader.screenOn
                if (actualOn != screenOn) {
                    Log.d(TAG, "supervisor reconciled screenOn: $screenOn -> $actualOn")
                    screenOn = actualOn
                    if (screenOn) {
                        sleepJob?.cancel()
                        locked = reader.keyguardLocked
                        if (!locked) seedForeground()
                    } else {
                        onScreenOff()
                    }
                    continue
                }
                if (watcher.isAlive) continue
                if (screenOn && !locked) {
                    val fg = runCatching { watcher.peekEvents() }.getOrNull()
                    if (fg != null && fg != lastSeenPkg) {
                        lastSeenPkg = fg
                        DynamicProfileState.lastForeground.value = fg
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

    private fun evaluate(trigger: String, fgOverride: String? = null) {
        if (!config.enabled) {
            stopSelf()
            return
        }
        val liveSaver = miState.readSaver()
        val input = ArbiterInput(
            serviceEnabled = true,
            screenOn = screenOn,
            keyguardLocked = locked,
            foregroundPkg = fgOverride ?: DynamicProfileState.lastForeground.value,
            appMap = config.appMap(),
            baseProfile = config.baseProfile,
            sleepProfile = ModeArbiter.SLEEP_PROFILE,
            // attribution: a saver flag WE hold for a mapped app is invisible
            // to the decision — only the user's own saver forces the base
            saverOn = bridge.value.userSaver(liveSaver),
            ultraSaver = miState.ultra(),
            multiWindow = multiWindow,
            dynamicProfile = config.dynamicEnabled,
        )
        pendingSaver = input.saverOn
        when (val d = ModeArbiter.decide(input)) {
            is Decision.None -> Log.d(TAG, "evaluate($trigger): no-op")
            is Decision.Apply -> {
                Log.d(TAG, "evaluate($trigger): -> ${d.profileId} (${d.reason})")
                enqueue(d)
            }
            is Decision.Retire -> retire(trigger)
        }
        // MIUI bridge extras — serialized: the MIUI bridge writes take time
        // and the state machine must never race itself (two overlapping
        // evaluations produced an ON/restore yo-yo: the dialog's own resume
        // event re-entered the sync and cancelled the write in flight).
        scope.launch(Dispatchers.IO) {
            bridgeMutex.withLock {
                runCatching {
                    syncPerfLocked()
                    syncSaverLocked()
                    checkGameModeLocked()
                }.onFailure { Log.w(TAG, "bridge sync: $it") }
            }
        }
    }

    /**
     * Ultra battery saver: MIUI owns the device from here on. Restore our
     * snapshot (so no MiFineTune values linger), release bridge holds, and
     * hand over completely.
     */
    private fun retire(trigger: String) {
        if (retired) return
        retired = true
        Log.d(TAG, "evaluate($trigger): MIUI ultra saver — retiring service")
        scope.launch(Dispatchers.IO) {
            releaseBridgeHolds()
            runCatching { Tuner.restore() }
            config.enabled = false
            runCatching { DynamicProfileService.stop(this@DynamicProfileService) }
            Tuner.stopGuard()
        }
    }

    // --- MIUI bridge sync ---------------------------------------------------

    private val bridge = MutableStateFlow(MiBridgeState())
    private val bridgeMutex = Mutex()

    private fun persistBridge() {
        val b = bridge.value
        config.bridgeHoldPerf = b.perfHeld
        config.bridgeSavedPerf = b.perfSaved.key
        config.bridgeHoldSaver = b.saverHeld
        config.bridgeSavedSaver = b.saverSaved
    }

    /**
     * Service death while a hold is active: restore points survive in prefs
     * and are re-armed here. The following evaluate() decides whether the
     * hold still applies (RESTORE otherwise) — no stuck MIUI state.
     */
    private fun recoverBridge() {
        bridge.update { b ->
            var n = b
            if (config.bridgeHoldPerf) {
                n = n.copy(perfHeld = true, perfSaved = MiStateBridge.PowerMode.of(config.bridgeSavedPerf))
            }
            if (config.bridgeHoldSaver) {
                n = n.copy(saverHeld = true, saverSaved = config.bridgeSavedSaver)
            }
            n
        }
    }

    /** Write the user's captured values back (teardown/retire path). */
    private fun releaseBridgeHolds() {
        val b = bridge.value
        if (b.perfHeld) {
            runCatching { miState.writePowerMode(b.perfSaved) }
            logEvent("MIUI perf mirror restored (${b.perfSaved.key})")
        }
        if (b.saverHeld) {
            runCatching { miState.writeSaver(b.saverSaved) }
            logEvent("MIUI saver restored")
        }
        bridge.value = MiBridgeState()
        persistBridge()
    }

    /**
     * MIUI Performance-mode follow: ON while a mapped game is in front and
     * the user enabled the sync. The write goes through MIUI's own hidden
     * dialog (the only channel SELinux allows), so it is idempotent-checked
     * against the live property first — no flash when already correct.
     *
     * Runs inside the bridge mutex. Uses [lastRealPkg], never the raw
     * foreground: the dialog's own resume event (com.android.settings,
     * transient) must not flip "game in front" off and cancel the write.
     */
    private fun syncPerfLocked() {
        if (retired) return
        val fg = lastRealPkg
        val gameInFront = fg?.let { config.appMap()[it] } == "game"
        val want = config.syncMiuiPerf && config.dynamicEnabled && gameInFront && screenOn && !locked
        val live = miState.readPowerMode()
        lateinit var next: MiBridgeState
        var action = MiBridgeState.PerfAction.NONE
        bridge.update { b ->
            val (n, a) = b.requestPerf(live, want)
            next = n; action = a
            n
        }
        persistBridge()
        when (action) {
            MiBridgeState.PerfAction.WRITE -> {
                miState.writePowerMode(MiStateBridge.PowerMode.Performance)
                logEvent("MIUI perf mirror ON (game)")
            }
            MiBridgeState.PerfAction.RESTORE -> {
                miState.writePowerMode(next.perfSaved)
                logEvent("MIUI perf mirror restored (${next.perfSaved.key})")
            }
            else -> {}
        }
    }

    /**
     * MIUI battery-saver follow: ON while a frugal-mapped app is in front
     * (mapping semantics: apps mapped to powersave pull MIUI's saver along).
     * A mapped game wins instead — no saver sync during a game session.
     * Root-free read, live root write (verified). Runs inside the mutex.
     */
    private fun syncSaverLocked() {
        if (retired) return
        val fg = lastRealPkg
        val mappedProfile = fg?.let { config.appMap()[it] }
        val want = config.syncSaver &&
            config.dynamicEnabled &&
            mappedProfile == "powersave" &&
            screenOn && !locked
        val live = miState.readSaver()
        lateinit var next: MiBridgeState
        var action = MiBridgeState.SaverAction.NONE
        bridge.update { b ->
            val (n, a) = b.requestSaver(live, want)
            next = n; action = a
            n
        }
        persistBridge()
        when (action) {
            MiBridgeState.SaverAction.TURN_ON -> {
                runCatching { miState.writeSaver(true) }
                logEvent("MIUI saver ON (${labelFor(fg!!)})")
            }
            MiBridgeState.SaverAction.RESTORE -> {
                runCatching { miState.writeSaver(next.saverSaved) }
                logEvent("MIUI saver restored")
            }
            else -> {}
        }
    }

    /**
     * Game-mode checker (user verdict: MIUI's own Game Booster adds nothing
     * for mapped games, so our profile wins and MIUI's must stay out).
     * A mapped game in front + a held thermal scenario = Game Booster is
     * still fighting us → warn once per session (notification).
     */
    private fun checkGameModeLocked() {
        if (retired) return
        val fg = lastRealPkg
        val mapped = fg?.let { config.appMap()[it] } == "game"
        if (!mapped || !config.gameModeChecker || !config.dynamicEnabled) {
            gameModeWarnedFor = null
            return
        }
        val active = miState.readGameModeSignature()
        if (!active) {
            gameModeWarnedFor = null
            return
        }
        if (gameModeWarnedFor == fg) return
        gameModeWarnedFor = fg
        notifyGameModeConflict(fg)
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

    /**
     * Coalescing worker: bursts of foreground events (task restores, keyguard
     * transitions) collapse into a single apply of the *latest* decision after
     * a short settle window. While an apply runs, newer decisions supersede
     * the pending one instead of queueing behind it.
     */
    private fun enqueue(d: Decision.Apply) {
        pendingDecision = d
        if (pendingSince == 0L) pendingSince = System.currentTimeMillis()
        if (applyWorker?.isActive == true) return
        applyWorker = scope.launch {
            while (true) {
                delay(SETTLE_MS)
                val decision = pendingDecision ?: break
                pendingDecision = null
                val usedSaver = pendingSaver
                pendingSaver = false
                performApply(decision, usedSaver)
            }
        }
    }

    private suspend fun performApply(d: Decision.Apply, usedSaver: Boolean) {
        // latency instrumentation: settle = enqueue -> worker start,
        // total = worker start -> apply verified (log-only, no behaviour)
        val startedAt = System.currentTimeMillis()
        val settle = if (pendingSince > 0) startedAt - pendingSince else 0
        pendingSince = 0
        // base forced by the battery saver carries its own label
        val reasonText = if (usedSaver && d.reason == "base") "MIUI saver" else describe(d)
        val current = runCatching { Tuner.status().active }.getOrNull()
        if (current == d.profileId) {
            // already in place — only refresh visible state
            DynamicProfileState.appliedProfile.value = d.profileId
            DynamicProfileState.reason.value = reasonText
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
                    DynamicProfileState.appliedProfile.value = d.profileId
                    DynamicProfileState.reason.value = reasonText
                    updateNotification(d.profileId, reasonText)
                    Log.d(
                        TAG,
                        "apply ${d.profileId}: done in ${System.currentTimeMillis() - startedAt}ms (settle ${settle}ms)",
                    )
                    // drift handling is the supervisor's periodic evaluate —
                    // no separate guard loop here (it could stomp with a
                    // stale profile after a transient failure)
                } else {
                    Log.w(TAG, "apply ${d.profileId} failed after retry (${finalRep.failed})")
                    DynamicProfileState.reason.value = "apply failed (${finalRep.failed})"
                    updateNotification(d.profileId, "apply failed — open the app")
                }
            }
            .onFailure { e ->
                Log.w(TAG, "apply ${d.profileId} error: $e")
                DynamicProfileState.reason.value = "error: ${e.message}"
                updateNotification(d.profileId, "error — open the app")
            }
    }

    private fun describe(d: Decision.Apply): String = when (d.reason) {
        "base" -> "base"
        "screen off" -> "screen off"
        else -> DynamicProfileState.lastForeground.value?.let { labelFor(it) } ?: d.profileId
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
}
