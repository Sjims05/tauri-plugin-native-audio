package app.tauri.nativeaudio

import android.content.Context
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

private const val MAX_LOGGED_EVENTS = 500

/**
 * A saved log of what played, so the app can keep e.g. a recently played list, play counts or
 * listening progress up to date, including for playback while it wasn't running (Android Auto).
 *
 * - `start`: a track started playing.
 * - `complete`: it played to the end.
 * - `skip`: it was left before the end (`positionMs` is where).
 *
 * Only tracks with an `id` are logged.
 */
internal object PlaybackEvents {
    private val log = EventLog("tauri_native_audio_playback_events", "events", MAX_LOGGED_EVENTS)

    fun record(context: Context, type: String, itemId: Long, positionMs: Long): JSONObject {
        val event = JSONObject()
            .put("id", UUID.randomUUID().toString())
            .put("type", type)
            .put("itemId", itemId)
            .put("positionMs", positionMs)
            .put("atMs", System.currentTimeMillis())
        log.append(context, event)
        return event
    }

    fun pending(context: Context): JSONArray = log.pending(context)

    fun acknowledge(context: Context, ids: Set<String>) = log.acknowledge(context, ids)
}
