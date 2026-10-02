package app.tauri.nativeaudio

import android.content.Context
import android.content.SharedPreferences

private const val SETTINGS_PREFS_NAME = "tauri_native_audio_settings"
private const val KEY_RESUME_LAST_QUEUE = "resume_last_queue"
private const val KEY_REPEAT_ADDED_TRACKS = "repeat_added_tracks"
private const val KEY_SKIP_INTERVAL_MS = "skip_interval_ms"
private const val KEY_CONTROLS = "controls"
private const val KEY_PAUSED_KEEP_ALIVE_MINUTES = "paused_keep_alive_minutes"
private const val KEY_KEEP_ALIVE_WHILE_CAR_CONNECTED = "keep_alive_while_car_connected"
private const val KEY_TRACK_PROGRESS = "track_progress"
private const val KEY_KEEP_QUEUE_ON_STOP = "keep_queue_on_stop"
private const val DEFAULT_PAUSED_KEEP_ALIVE_MINUTES = 30.0

/**
 * Settings the app sets from JS, saved because the audio service can start without the app's JS
 * running (Android Auto, a headset button), and should still behave the way the app set it up.
 */
internal object PluginSettings {
    /** Automatically load the last queue when the service starts with nothing loaded. */
    fun resumeLastQueue(context: Context): Boolean = prefs(context).getBoolean(KEY_RESUME_LAST_QUEUE, true)

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

    fun setOptions(
        context: Context,
        resumeLastQueue: Boolean?,
        repeatAddedTracks: Boolean?,
        pausedKeepAliveMinutes: Double?,
        keepAliveWhileCarConnected: Boolean?,
        trackProgress: Boolean?,
        keepQueueOnStop: Boolean?,
    ) {
        prefs(context).edit().apply {
            keepQueueOnStop?.let { putBoolean(KEY_KEEP_QUEUE_ON_STOP, it) }
            trackProgress?.let { putBoolean(KEY_TRACK_PROGRESS, it) }
            resumeLastQueue?.let { putBoolean(KEY_RESUME_LAST_QUEUE, it) }
            repeatAddedTracks?.let { putBoolean(KEY_REPEAT_ADDED_TRACKS, it) }
            pausedKeepAliveMinutes?.let { putFloat(KEY_PAUSED_KEEP_ALIVE_MINUTES, it.toFloat()) }
            keepAliveWhileCarConnected?.let { putBoolean(KEY_KEEP_ALIVE_WHILE_CAR_CONNECTED, it) }
        }.apply()
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
