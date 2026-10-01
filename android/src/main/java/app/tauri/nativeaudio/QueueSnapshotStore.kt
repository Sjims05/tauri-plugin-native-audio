package app.tauri.nativeaudio

import android.content.Context
import android.util.Log
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "plugin/native-audio"
private const val QUEUE_FILE_NAME = "native_audio_queue.json"
private const val POSITION_PREFS_NAME = "tauri_native_audio_queue_position"
private const val KEY_INDEX = "index"
private const val KEY_POSITION_MS = "position_ms"

/**
 * The last queue, saved so it can be resumed after the app was closed: from Android Auto, media
 * resumption, or the app itself (`restoreLastQueue`).
 *
 * The queue itself (items, shuffle order, modes) is written to a file only when it changes; the
 * current index and position go to SharedPreferences, since they change every second.
 */
internal object QueueSnapshotStore {
    class Snapshot(
        val items: List<MediaItem>,
        val index: Int,
        val positionMs: Long,
        val shuffle: Boolean,
        /** Null when there's no valid saved order (it's rebuilt on restore). */
        val shuffleOrder: IntArray?,
        val repeatMode: Int,
        /** The playable folder the queue was started from (see setQueue's sourceId). */
        val sourceId: String?,
    )

    fun saveQueue(
        context: Context,
        items: List<MediaItem>,
        shuffle: Boolean,
        shuffleOrder: IntArray,
        repeatMode: Int,
        sourceId: String?,
    ) {
        val file = file(context)
        if (items.isEmpty()) {
            file.delete()
            return
        }
        val json = JSONObject()
            .put("items", JSONArray().apply { items.forEach { put(itemToJson(it)) } })
            .put("shuffle", shuffle)
            .put("shuffleOrder", JSONArray().apply { shuffleOrder.forEach { put(it) } })
            .put("repeatMode", repeatMode)
            .put("sourceId", sourceId ?: JSONObject.NULL)
        runCatching { file.writeText(json.toString()) }
            .onFailure { Log.w(TAG, "failed to save queue", it) }
    }

    fun savePosition(context: Context, index: Int, positionMs: Long) {
        prefs(context).edit()
            .putInt(KEY_INDEX, index)
            .putLong(KEY_POSITION_MS, positionMs)
            .apply()
    }

    fun load(context: Context): Snapshot? {
        val file = file(context)
        if (!file.exists()) return null
        return runCatching {
            val json = JSONObject(file.readText())
            val itemsJson = json.getJSONArray("items")
            val items = (0 until itemsJson.length()).mapNotNull { itemFromJson(itemsJson.getJSONObject(it)) }
            if (items.isEmpty()) return null

            val orderJson = json.optJSONArray("shuffleOrder") ?: JSONArray()
            val order = IntArray(orderJson.length()) { orderJson.getInt(it) }
            val validOrder = order.takeIf { it.size == items.size && it.sorted() == items.indices.toList() }

            val prefs = prefs(context)
            Snapshot(
                items = items,
                index = prefs.getInt(KEY_INDEX, 0).coerceIn(0, items.size - 1),
                positionMs = prefs.getLong(KEY_POSITION_MS, 0L).coerceAtLeast(0L),
                shuffle = json.optBoolean("shuffle", false),
                shuffleOrder = validOrder,
                repeatMode = json.optInt("repeatMode", Player.REPEAT_MODE_OFF),
                sourceId = json.optStringOrNull("sourceId"),
            )
        }.getOrElse {
            Log.w(TAG, "failed to load saved queue", it)
            null
        }
    }

    private fun itemToJson(item: MediaItem): JSONObject {
        val metadata = item.mediaMetadata
        return JSONObject()
            .put("src", item.localConfiguration?.uri?.toString())
            .put("id", item.mediaId.toLongOrNull())
            .put("title", metadata.title?.toString())
            .put("artist", metadata.artist?.toString())
            .put("artworkUrl", NativeAudioRuntime.artworkUrlOf(item))
            .put("added", NativeAudioRuntime.isAddedToQueue(item))
    }

    private fun itemFromJson(json: JSONObject): MediaItem? {
        val src = json.optStringOrNull("src") ?: return null
        return NativeAudioRuntime.buildMediaItem(
            src = src,
            id = if (json.has("id") && !json.isNull("id")) json.getLong("id") else null,
            title = json.optStringOrNull("title"),
            artist = json.optStringOrNull("artist"),
            artworkUrl = json.optStringOrNull("artworkUrl"),
            addedToQueue = json.optBoolean("added", false),
        )
    }

    private fun file(context: Context) = File(context.applicationContext.filesDir, QUEUE_FILE_NAME)

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(POSITION_PREFS_NAME, Context.MODE_PRIVATE)

    private fun JSONObject.optStringOrNull(key: String): String? =
        if (has(key) && !isNull(key)) getString(key) else null
}
