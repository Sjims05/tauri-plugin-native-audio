package app.tauri.nativeaudio

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

private const val PREFS_NAME = "tauri_native_audio_item_progress"
private const val KEY_PROGRESS = "progress"
/** From here on an item counts as played (e.g. skipped credits at the end of an episode). */
private const val PLAYED_AT = 0.98

/**
 * How far each item was played (setOptions `trackProgress`), for podcasts and audiobooks: shown as
 * played / partly played in Android Auto, and partly played items continue where they stopped when
 * picked there. Kept per item with the time it changed, so versions from elsewhere can be merged.
 */
internal object ItemProgress {
    class Entry(val itemId: Long, val progress: Double, val positionMs: Long, val updatedAtMs: Long) {
        fun toJson(): JSONObject = JSONObject()
            .put("itemId", itemId)
            .put("progress", progress)
            .put("positionMs", positionMs)
            .put("updatedAtMs", updatedAtMs)

        /** Started but not finished: where to continue from. */
        val resumePositionMs: Long? get() = positionMs.takeIf { progress > 0.0 && progress < 1.0 }
    }

    private val lock = Any()
    private var cache: MutableMap<Long, Entry>? = null

    fun get(context: Context, itemId: Long): Entry? = synchronized(lock) { load(context)[itemId] }

    /** All entries, or those of [itemIds]. */
    fun list(context: Context, itemIds: List<Long>?): List<Entry> = synchronized(lock) {
        val all = load(context)
        itemIds?.mapNotNull { all[it] } ?: all.values.toList()
    }

    /** Records playback of [itemId] at [positionMs] of [durationMs]; [completed] counts as played. */
    fun record(context: Context, itemId: Long, positionMs: Long, durationMs: Long, completed: Boolean) {
        if (!completed && durationMs <= 0) return
        val progress = if (completed) 1.0 else (positionMs.toDouble() / durationMs).coerceIn(0.0, 1.0)
        val entry = Entry(
            itemId = itemId,
            progress = if (progress >= PLAYED_AT) 1.0 else progress,
            positionMs = if (completed) durationMs.coerceAtLeast(positionMs) else positionMs.coerceAtLeast(0L),
            updatedAtMs = System.currentTimeMillis(),
        )
        synchronized(lock) {
            val all = load(context)
            all[itemId] = entry
            save(context, all)
        }
    }

    /**
     * Sets entries from the app. [merge] keeps whichever version of each item is newer
     * (updatedAtMs); otherwise the given entries replace the stored ones. Returns the stored result
     * for the given items.
     */
    fun set(context: Context, entries: List<Entry>, merge: Boolean): List<Entry> = synchronized(lock) {
        val all = load(context)
        for (entry in entries) {
            val current = all[entry.itemId]
            if (!merge || current == null || entry.updatedAtMs > current.updatedAtMs) all[entry.itemId] = entry
        }
        save(context, all)
        entries.mapNotNull { all[it.itemId] }
    }

    fun parse(json: JSONArray): List<Entry> = (0 until json.length()).map { i ->
        val entryJson = json.optJSONObject(i) ?: throw IllegalArgumentException("entries[$i] must be an object")
        require(entryJson.has("itemId")) { "entries[$i] requires itemId" }
        val progress = entryJson.optDouble("progress", Double.NaN)
        require(progress.isFinite() && progress in 0.0..1.0) { "entries[$i]: progress must be between 0 and 1" }
        Entry(
            itemId = entryJson.getLong("itemId"),
            progress = progress,
            positionMs = entryJson.optLong("positionMs", 0L).coerceAtLeast(0L),
            updatedAtMs = entryJson.optLong("updatedAtMs", System.currentTimeMillis()),
        )
    }

    private fun load(context: Context): MutableMap<Long, Entry> {
        cache?.let { return it }
        val loaded = HashMap<Long, Entry>()
        runCatching {
            val json = JSONObject(prefs(context).getString(KEY_PROGRESS, "{}")!!)
            for (key in json.keys()) {
                val entry = json.getJSONObject(key)
                loaded[key.toLong()] = Entry(
                    itemId = key.toLong(),
                    progress = entry.getDouble("progress"),
                    positionMs = entry.getLong("positionMs"),
                    updatedAtMs = entry.getLong("updatedAtMs"),
                )
            }
        }
        cache = loaded
        return loaded
    }

    private fun save(context: Context, all: Map<Long, Entry>) {
        val json = JSONObject()
        for ((itemId, entry) in all) {
            json.put(itemId.toString(), JSONObject().put("progress", entry.progress).put("positionMs", entry.positionMs).put("updatedAtMs", entry.updatedAtMs))
        }
        prefs(context).edit().putString(KEY_PROGRESS, json.toString()).apply()
    }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
