package app.tauri.nativeaudio

import android.content.Context
import android.content.SharedPreferences

private const val SETTINGS_PREFS_NAME = "tauri_native_audio_settings"
private const val KEY_RESUME_LAST_QUEUE = "resume_last_queue"
private const val KEY_RESUME_MODE = "resume_mode"
private val RESUME_MODES = setOf("off", "paused", "play")
private const val KEY_REPEAT_ADDED_TRACKS = "repeat_added_tracks"
private const val KEY_SKIP_INTERVAL_MS = "skip_interval_ms"
private const val KEY_CONTROLS = "controls"
private const val KEY_PAUSED_KEEP_ALIVE_MINUTES = "paused_keep_alive_minutes"
private const val KEY_KEEP_ALIVE_WHILE_CAR_CONNECTED = "keep_alive_while_car_connected"
private const val KEY_TRACK_PROGRESS = "track_progress"
private const val KEY_KEEP_QUEUE_ON_STOP = "keep_queue_on_stop"
private const val KEY_VOLUME_CURVE = "volume_curve"
private const val KEY_VOLUME = "volume"
private const val KEY_PREVIOUS_RESTARTS_AFTER_SECONDS = "previous_restarts_after_seconds"
private const val KEY_NEXT_AT_END = "next_at_end"
private val NEXT_AT_END_VALUES = setOf("nothing", "first")
private val VOLUME_CURVES = setOf("linear", "quadratic", "cubic")
private const val DEFAULT_PAUSED_KEEP_ALIVE_MINUTES = 30.0

/**
 * Settings the app sets from JS, saved because the audio service can start without the app's JS
 * running (Android Auto, a headset button), and should still behave the way the app set it up.
 */
internal object PluginSettings {
    /** Automatically load the last queue when the service starts with nothing loaded. */
    fun resumeLastQueue(context: Context): Boolean = resumeMode(context) != "off"

    /** "off", "paused" (load it) or "play" (load and play it). Saved as a boolean before "play" existed. */
    fun resumeMode(context: Context): String {
        val prefs = prefs(context)
        prefs.getString(KEY_RESUME_MODE, null)?.let { return it }
        return if (prefs.getBoolean(KEY_RESUME_LAST_QUEUE, true)) "paused" else "off"
    }

    /** Keep tracks added with addToQueue when the queue repeats; false (default) plays them once. */
    fun repeatAddedTracks(context: Context): Boolean = prefs(context).getBoolean(KEY_REPEAT_ADDED_TRACKS, false)

    /**
     * How long the playback controls (notification, lock screen, Android Auto's player) stay after a
     * pause, so playback is easy to resume. 0 = they can go right away.
     */
    fun pausedKeepAliveMs(context: Context): Long =
        (prefs(context).getFloat(KEY_PAUSED_KEEP_ALIVE_MINUTES, DEFAULT_PAUSED_KEEP_ALIVE_MINUTES.toFloat()) * 60_000.0).toLong()

    /** While Android Auto is connected, keep the controls after a pause with no time limit. */
    fun keepAliveWhileCarConnected(context: Context): Boolean =
        prefs(context).getBoolean(KEY_KEEP_ALIVE_WHILE_CAR_CONNECTED, true)

    /** A stop from outside the app (system media panel, Bluetooth, Android Auto) pauses instead, keeping the queue. */
    fun keepQueueOnStop(context: Context): Boolean = prefs(context).getBoolean(KEY_KEEP_QUEUE_ON_STOP, true)

    /** Remember how far each item played (podcasts, audiobooks), see ItemProgress. */
    fun trackProgress(context: Context): Boolean = prefs(context).getBoolean(KEY_TRACK_PROGRESS, false)

    /** How a setVolume value becomes loudness: linear, quadratic (default) or cubic. */
    fun volumeCurve(context: Context): String = prefs(context).getString(KEY_VOLUME_CURVE, null) ?: "quadratic"

    fun setOptions(
        context: Context,
        resumeLastQueue: Any?,
        repeatAddedTracks: Boolean?,
        pausedKeepAliveMinutes: Double?,
        keepAliveWhileCarConnected: Boolean?,
        trackProgress: Boolean?,
        keepQueueOnStop: Boolean?,
        volumeCurve: String?,
        previousRestartsAfterSeconds: Double?,
        nextAtEnd: String?,
    ) {
        require(volumeCurve == null || volumeCurve in VOLUME_CURVES) {
            "unknown volume curve \"$volumeCurve\" (linear, quadratic or cubic)"
        }
        require(previousRestartsAfterSeconds == null || (previousRestartsAfterSeconds.isFinite() && previousRestartsAfterSeconds >= 0)) {
            "previousRestartsAfterSeconds must be >= 0"
        }
        require(nextAtEnd == null || nextAtEnd in NEXT_AT_END_VALUES) { "nextAtEnd must be nothing or first" }
        // true / false from before "play" existed: paused / off.
        val resumeMode = when (resumeLastQueue) {
            null -> null
            true -> "paused"
            false -> "off"
            is String -> resumeLastQueue.takeIf { it in RESUME_MODES }
            else -> null
        }
        require(resumeLastQueue == null || resumeMode != null) { "resumeLastQueue must be off, paused or play" }
        prefs(context).edit().apply {
            previousRestartsAfterSeconds?.let { putFloat(KEY_PREVIOUS_RESTARTS_AFTER_SECONDS, it.toFloat()) }
            nextAtEnd?.let { putString(KEY_NEXT_AT_END, it) }
            volumeCurve?.let { putString(KEY_VOLUME_CURVE, it) }
            keepQueueOnStop?.let { putBoolean(KEY_KEEP_QUEUE_ON_STOP, it) }
            trackProgress?.let { putBoolean(KEY_TRACK_PROGRESS, it) }
            resumeMode?.let { putString(KEY_RESUME_MODE, it) }
            repeatAddedTracks?.let { putBoolean(KEY_REPEAT_ADDED_TRACKS, it) }
            pausedKeepAliveMinutes?.let { putFloat(KEY_PAUSED_KEEP_ALIVE_MINUTES, it.toFloat()) }
            keepAliveWhileCarConnected?.let { putBoolean(KEY_KEEP_ALIVE_WHILE_CAR_CONNECTED, it) }
        }.apply()
    }

    /** Previous restarts the current track when it's past this; 0 = previous always goes back a track. */
    fun previousRestartAfterMs(context: Context): Long =
        (prefs(context).getFloat(KEY_PREVIOUS_RESTARTS_AFTER_SECONDS, 3f) * 1000f).toLong()

    /** Next on the last track (repeat off): "nothing", or "first" (the first track, paused). */
    fun nextAtEndFirst(context: Context): Boolean = prefs(context).getString(KEY_NEXT_AT_END, null) == "first"

    /** The setVolume value, 0 to 1. */
    fun volume(context: Context): Float = prefs(context).getFloat(KEY_VOLUME, 1f).coerceIn(0f, 1f)

    fun setVolume(context: Context, value: Float) {
        prefs(context).edit().putFloat(KEY_VOLUME, value).apply()
    }

    fun skipIntervalMs(context: Context): Long = prefs(context).getLong(KEY_SKIP_INTERVAL_MS, 0L)

    fun setSkipIntervalMs(context: Context, value: Long) {
        prefs(context).edit().putLong(KEY_SKIP_INTERVAL_MS, value).apply()
    }

    /** The setControls configuration as JSON, or null when none was set. */
    fun controlsJson(context: Context): String? = prefs(context).getString(KEY_CONTROLS, null)

    fun setControlsJson(context: Context, json: String) {
        prefs(context).edit().putString(KEY_CONTROLS, json).apply()
    }

    private fun prefs(context: Context): SharedPreferences =
        context.applicationContext.getSharedPreferences(SETTINGS_PREFS_NAME, Context.MODE_PRIVATE)
}
