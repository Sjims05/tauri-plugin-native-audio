package app.tauri.nativeaudio

import android.content.Context
import android.os.Bundle
import android.util.Log
import androidx.media3.common.Player
import androidx.media3.session.CommandButton
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionCommand
import androidx.media3.session.SessionCommands
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "plugin/native-audio"
private const val ACTION_PREFIX = "app.tauri.nativeaudio.control."
private const val PRESS_PREFS_NAME = "tauri_native_audio_control_presses"
private const val KEY_PRESSES = "presses"
private const val KEY_TOGGLE_STATES = "toggle_states"
private const val MAX_LOGGED_PRESSES = 1000
private const val DEFAULT_DEBOUNCE_MS = 500L

/**
 * The extra buttons set with `setControls`, shown by Android Auto and the Android 13+ media
 * controls next to play / pause / previous / next.
 *
 * Built-in buttons (seek, shuffle, repeat, speed) act natively. Custom buttons are for the app:
 * each press is logged with its time and saved, so the app gets it even when it wasn't running
 * at the time, and toggle buttons (like "like") keep an on / off state per track.
 */
internal object PlaybackControls {
    sealed class Button(val key: String) {
        val action get() = ACTION_PREFIX + key

        class SeekBack(val seconds: Double) : Button("seekBack")
        class SeekForward(val seconds: Double) : Button("seekForward")
        object Shuffle : Button("shuffle")
        object Repeat : Button("repeat")
        class Speed(val rates: List<Double>) : Button("speed")
        /** Cycles off -> each option -> off. An option is minutes, or null for "end of track". */
        class SleepTimer(val options: List<Double?>, val fadeOutSeconds: Double) : Button("sleepTimer")
        class Custom(
            val id: String,
            val label: String,
            val icon: String?,
            val iconResource: String?,
            val toggle: Boolean,
        ) : Button("custom.$id")
    }

    class Config(val buttons: List<Button>, val debounceMs: Long) {
        companion object {
            val EMPTY = Config(emptyList(), DEFAULT_DEBOUNCE_MS)
        }
    }

    /** What the buttons show, read from the player. */
    class PlayerView(
        val shuffle: Boolean,
        val repeatMode: Int,
        val speed: Double,
        val itemId: Long?,
        val sleepTimerMinutes: Double?,
        val sleepTimerEndOfTrack: Boolean,
    )

    class Press(
        val id: String,
        val buttonId: String,
        val itemId: Long?,
        /** New state of a toggle button, null for a plain button. */
        val active: Boolean?,
        val pressedAtMs: Long,
    ) {
        fun toJson(): JSONObject = JSONObject()
            .put("id", id)
            .put("buttonId", buttonId)
            .put("itemId", itemId ?: JSONObject.NULL)
            .put("active", active ?: JSONObject.NULL)
            .put("pressedAtMs", pressedAtMs)
    }

    private val lock = Any()
    private val pressLog = EventLog(PRESS_PREFS_NAME, KEY_PRESSES, MAX_LOGGED_PRESSES)
    private var config: Config? = null
    private val lastAcceptedPressMs = HashMap<String, Long>()

    // ---- configuration

    /** Validates and saves a setControls configuration. Throws [IllegalArgumentException]. */
    fun set(context: Context, json: JSONObject) {
        val parsed = parse(json)
        PluginSettings.setControlsJson(context, json.toString())
        synchronized(lock) {
            config = parsed
        }
    }

    fun get(context: Context): Config {
        synchronized(lock) {
            config?.let { return it }
            val loaded = PluginSettings.controlsJson(context)?.let { saved ->
                runCatching { parse(JSONObject(saved)) }
                    .onFailure { Log.w(TAG, "failed to load saved controls", it) }
                    .getOrNull()
            } ?: Config.EMPTY
            config = loaded
            return loaded
        }
    }

    fun find(context: Context, action: String): Button? = get(context).buttons.firstOrNull { it.action == action }

    /**
     * The library session's default commands plus one per button. Without search in the library
     * (setLibrary `search: false`), the search command is left out, which hides Android Auto's
     * search button.
     */
    fun sessionCommands(session: MediaSession, context: Context): SessionCommands {
        val builder = MediaSession.ConnectionResult.DEFAULT_SESSION_AND_LIBRARY_COMMANDS.buildUpon()
        get(context).buttons.forEach { builder.add(SessionCommand(it.action, Bundle.EMPTY)) }
        if (!MediaLibraryStore.get(context).search) builder.remove(SessionCommand.COMMAND_CODE_LIBRARY_SEARCH)
        return builder.build()
    }

    fun layout(context: Context, view: PlayerView): List<CommandButton> =
        get(context).buttons.map { button ->
            val (icon, name) = iconAndName(context, button, view)
            CommandButton.Builder(icon)
                .setDisplayName(name)
                .setSessionCommand(SessionCommand(button.action, Bundle.EMPTY))
                .apply {
                    if (button is Button.SleepTimer) {
                        val on = view.sleepTimerMinutes != null || view.sleepTimerEndOfTrack
                        setIconResId(if (on) R.drawable.native_audio_ic_sleep_timer_on else R.drawable.native_audio_ic_sleep_timer)
                    }
                    if (button is Button.Custom && button.iconResource != null) {
                        val resId = context.resources.getIdentifier(button.iconResource, "drawable", context.packageName)
                        if (resId != 0) setIconResId(resId)
                    }
                }
                .build()
        }

    private fun parse(json: JSONObject): Config {
        val buttonsJson = json.optJSONArray("buttons") ?: JSONArray()
        val ids = HashSet<String>()
        val buttons = (0 until buttonsJson.length()).map { i ->
            val buttonJson = buttonsJson.getJSONObject(i)
            when (val action = buttonJson.optString("action")) {
                "seekBack", "seekForward" -> {
                    val seconds = buttonJson.optDouble("seconds", Double.NaN)
                    require(seconds.isFinite() && seconds > 0) { "buttons[$i]: $action requires seconds > 0" }
                    if (action == "seekBack") Button.SeekBack(seconds) else Button.SeekForward(seconds)
                }
                "shuffle" -> Button.Shuffle
                "repeat" -> Button.Repeat
                "speed" -> {
                    val ratesJson = buttonJson.optJSONArray("rates")
                    val rates = if (ratesJson == null) listOf(1.0, 1.25, 1.5, 2.0) else (0 until ratesJson.length()).map { ratesJson.getDouble(it) }
                    require(rates.isNotEmpty() && rates.all { it.isFinite() && it > 0 }) { "buttons[$i]: speed rates must be > 0" }
                    Button.Speed(rates)
                }
                "sleepTimer" -> {
                    val optionsJson = buttonJson.optJSONArray("options")
                    val options: List<Double?> = if (optionsJson == null) {
                        listOf(15.0, 30.0, 60.0, null)
                    } else {
                        (0 until optionsJson.length()).map { o ->
                            when (val option = optionsJson.get(o)) {
                                "endOfTrack" -> null
                                is Number -> option.toDouble().also { require(it > 0) { "buttons[$i]: sleep timer minutes must be > 0" } }
                                else -> throw IllegalArgumentException("buttons[$i]: sleep timer options are minutes or \"endOfTrack\"")
                            }
                        }
                    }
                    require(options.isNotEmpty()) { "buttons[$i]: sleepTimer needs at least one option" }
                    val fadeOutSeconds = buttonJson.optDouble("fadeOutSeconds", 10.0)
                    require(fadeOutSeconds.isFinite() && fadeOutSeconds >= 0) { "buttons[$i]: fadeOutSeconds must be >= 0" }
                    Button.SleepTimer(options, fadeOutSeconds)
                }
                "custom" -> {
                    val id = buttonJson.optString("id")
                    require(id.isNotEmpty()) { "buttons[$i]: custom requires id" }
                    val label = buttonJson.optString("label")
                    require(label.isNotEmpty()) { "custom button $id requires label" }
                    val icon = buttonJson.optString("icon").takeIf { it.isNotEmpty() }
                    require(icon == null || icon in TOGGLE_ICONS || icon in PLAIN_ICONS) { "custom button $id: unknown icon $icon" }
                    Button.Custom(
                        id = id,
                        label = label,
                        icon = icon,
                        iconResource = buttonJson.optString("iconResource").takeIf { it.isNotEmpty() },
                        toggle = buttonJson.optBoolean("toggle", false),
                    )
                }
                else -> throw IllegalArgumentException("buttons[$i]: unknown action $action")
            }.also { require(ids.add(it.key)) { "duplicate button ${it.key}" } }
        }
        val debounceMs = json.optLong("debounceMs", DEFAULT_DEBOUNCE_MS).coerceAtLeast(0L)
        return Config(buttons, debounceMs)
    }

    private fun iconAndName(context: Context, button: Button, view: PlayerView): Pair<Int, String> = when (button) {
        is Button.SeekBack -> seekIcon(button.seconds, back = true) to "Back ${formatSeconds(button.seconds)}"
        is Button.SeekForward -> seekIcon(button.seconds, back = false) to "Forward ${formatSeconds(button.seconds)}"
        Button.Shuffle ->
            if (view.shuffle) CommandButton.ICON_SHUFFLE_ON to "Shuffle on" else CommandButton.ICON_SHUFFLE_OFF to "Shuffle off"
        Button.Repeat -> when (view.repeatMode) {
            Player.REPEAT_MODE_ALL -> CommandButton.ICON_REPEAT_ALL to "Repeat all"
            Player.REPEAT_MODE_ONE -> CommandButton.ICON_REPEAT_ONE to "Repeat one"
            else -> CommandButton.ICON_REPEAT_OFF to "Repeat off"
        }
        is Button.Speed -> speedIcon(view.speed) to "Speed ${formatRate(view.speed)}x"
        is Button.SleepTimer -> CommandButton.ICON_UNDEFINED to when {
            view.sleepTimerEndOfTrack -> "Sleep timer: end of track"
            view.sleepTimerMinutes != null -> "Sleep timer: ${formatRate(view.sleepTimerMinutes)} min"
            else -> "Sleep timer off"
        }
        is Button.Custom -> {
            val active = button.toggle && isActive(context, button.id, view.itemId)
            val icon = TOGGLE_ICONS[button.icon]?.let { (off, on) -> if (active) on else off }
                ?: PLAIN_ICONS[button.icon]
                ?: CommandButton.ICON_UNDEFINED
            icon to button.label
        }
    }

    // ---- presses

    /**
     * Records a press of a custom button: flips a toggle's state for [itemId] and logs the press.
     * Returns null when it's a repeat of the same button within the debounce window.
     */
    fun recordPress(context: Context, button: Button.Custom, itemId: Long?, nowMs: Long = System.currentTimeMillis()): Press? {
        synchronized(lock) {
            val debounceMs = get(context).debounceMs
            val last = lastAcceptedPressMs[button.id]
            if (last != null && nowMs - last < debounceMs) return null
            lastAcceptedPressMs[button.id] = nowMs

            val active = if (button.toggle) !isActive(context, button.id, itemId) else null
            if (active != null) setActiveLocked(context, button.id, listOf(itemId), active)

            val press = Press(UUID.randomUUID().toString(), button.id, itemId, active, nowMs)
            pressLog.append(context, press.toJson())
            return press
        }
    }

    /** Logged presses the app hasn't acknowledged yet, oldest first. */
    fun pendingPresses(context: Context): JSONArray = pressLog.pending(context)

    fun acknowledge(context: Context, ids: Set<String>) = pressLog.acknowledge(context, ids)

    // ---- toggle states

    fun isActive(context: Context, buttonId: String, itemId: Long?): Boolean = synchronized(lock) {
        toggleStates(context).optJSONObject(buttonId)?.optBoolean(itemKey(itemId), false) ?: false
    }

    /** Sets a toggle button's state for tracks, e.g. from the app's own "liked" data. */
    fun setActive(context: Context, buttonId: String, itemIds: List<Long?>, active: Boolean) {
        synchronized(lock) {
            setActiveLocked(context, buttonId, itemIds, active)
        }
    }

    private fun setActiveLocked(context: Context, buttonId: String, itemIds: List<Long?>, active: Boolean) {
        val states = toggleStates(context)
        val buttonStates = states.optJSONObject(buttonId) ?: JSONObject()
        for (itemId in itemIds) {
            if (active) buttonStates.put(itemKey(itemId), true) else buttonStates.remove(itemKey(itemId))
        }
        states.put(buttonId, buttonStates)
        pressPrefs(context).edit().putString(KEY_TOGGLE_STATES, states.toString()).apply()
    }

    private fun itemKey(itemId: Long?) = itemId?.toString() ?: ""

    private fun toggleStates(context: Context): JSONObject =
        runCatching { JSONObject(pressPrefs(context).getString(KEY_TOGGLE_STATES, "{}")!!) }.getOrElse { JSONObject() }

    private fun pressPrefs(context: Context) =
        context.applicationContext.getSharedPreferences(PRESS_PREFS_NAME, Context.MODE_PRIVATE)

    // ---- icons

    // Media3's built-in icons, by the name the app passes. Toggle icons are (off, on) pairs.
    private val TOGGLE_ICONS = mapOf(
        "heart" to (CommandButton.ICON_HEART_UNFILLED to CommandButton.ICON_HEART_FILLED),
        "star" to (CommandButton.ICON_STAR_UNFILLED to CommandButton.ICON_STAR_FILLED),
        "thumbUp" to (CommandButton.ICON_THUMB_UP_UNFILLED to CommandButton.ICON_THUMB_UP_FILLED),
        "thumbDown" to (CommandButton.ICON_THUMB_DOWN_UNFILLED to CommandButton.ICON_THUMB_DOWN_FILLED),
        "bookmark" to (CommandButton.ICON_BOOKMARK_UNFILLED to CommandButton.ICON_BOOKMARK_FILLED),
        "flag" to (CommandButton.ICON_FLAG_UNFILLED to CommandButton.ICON_FLAG_FILLED),
        "checkCircle" to (CommandButton.ICON_CHECK_CIRCLE_UNFILLED to CommandButton.ICON_CHECK_CIRCLE_FILLED),
        "plusCircle" to (CommandButton.ICON_PLUS_CIRCLE_UNFILLED to CommandButton.ICON_PLUS_CIRCLE_FILLED),
        "minusCircle" to (CommandButton.ICON_MINUS_CIRCLE_UNFILLED to CommandButton.ICON_MINUS_CIRCLE_FILLED),
    )

    private val PLAIN_ICONS = mapOf(
        "plus" to CommandButton.ICON_PLUS,
        "minus" to CommandButton.ICON_MINUS,
        "playlistAdd" to CommandButton.ICON_PLAYLIST_ADD,
        "playlistRemove" to CommandButton.ICON_PLAYLIST_REMOVE,
        "queueAdd" to CommandButton.ICON_QUEUE_ADD,
        "queueNext" to CommandButton.ICON_QUEUE_NEXT,
        "queueRemove" to CommandButton.ICON_QUEUE_REMOVE,
        "block" to CommandButton.ICON_BLOCK,
        "share" to CommandButton.ICON_SHARE,
        "radio" to CommandButton.ICON_RADIO,
        "album" to CommandButton.ICON_ALBUM,
        "artist" to CommandButton.ICON_ARTIST,
        "feed" to CommandButton.ICON_FEED,
        "settings" to CommandButton.ICON_SETTINGS,
        "sync" to CommandButton.ICON_SYNC,
        "quality" to CommandButton.ICON_QUALITY,
        "signal" to CommandButton.ICON_SIGNAL,
    )

    private fun seekIcon(seconds: Double, back: Boolean): Int = when (seconds) {
        5.0 -> if (back) CommandButton.ICON_SKIP_BACK_5 else CommandButton.ICON_SKIP_FORWARD_5
        10.0 -> if (back) CommandButton.ICON_SKIP_BACK_10 else CommandButton.ICON_SKIP_FORWARD_10
        15.0 -> if (back) CommandButton.ICON_SKIP_BACK_15 else CommandButton.ICON_SKIP_FORWARD_15
        30.0 -> if (back) CommandButton.ICON_SKIP_BACK_30 else CommandButton.ICON_SKIP_FORWARD_30
        else -> if (back) CommandButton.ICON_SKIP_BACK else CommandButton.ICON_SKIP_FORWARD
    }

    private fun speedIcon(rate: Double): Int = when (rate) {
        0.5 -> CommandButton.ICON_PLAYBACK_SPEED_0_5
        0.8 -> CommandButton.ICON_PLAYBACK_SPEED_0_8
        1.0 -> CommandButton.ICON_PLAYBACK_SPEED_1_0
        1.2 -> CommandButton.ICON_PLAYBACK_SPEED_1_2
        1.5 -> CommandButton.ICON_PLAYBACK_SPEED_1_5
        1.8 -> CommandButton.ICON_PLAYBACK_SPEED_1_8
        2.0 -> CommandButton.ICON_PLAYBACK_SPEED_2_0
        else -> CommandButton.ICON_PLAYBACK_SPEED
    }

    private fun formatSeconds(seconds: Double): String =
        if (seconds % 1.0 == 0.0) "${seconds.toLong()}s" else "%.1fs".format(seconds)

    private fun formatRate(rate: Double): String =
        if (rate % 1.0 == 0.0) rate.toLong().toString() else rate.toString()
}
