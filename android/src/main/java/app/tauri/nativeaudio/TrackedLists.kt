package app.tauri.nativeaudio

import android.content.Context
import android.util.Log
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "plugin/native-audio"
private const val PREFS_NAME = "tauri_native_audio_tracked_lists"
private const val KEY_CONFIGS = "configs"
private const val KEY_LISTS = "lists"
private const val KEY_CHANGES = "changes"
private const val DEFAULT_LIMIT = 10

/**
 * Lists the app declares with `setTrackedLists` and the plugin keeps up to date natively, such as
 * "the last 5 playlists played": newest first, no duplicates, capped at a limit. They update while
 * the app isn't running too (Android Auto), and changes are synced to the app like button presses.
 * A library folder can show a list in Android Auto with `trackedList`.
 *
 * Each entry keeps when it was last played, so versions of a list from elsewhere (the app's
 * database, another device) can be merged: by id, newest time wins.
 */
internal object TrackedLists {
    class ListConfig(
        val id: String,
        /** "folder": the playable folder (playlist, album) a queue was started from. "item": tracks. */
        val track: String,
        val limit: Int,
        /** How long something has to play before it counts; 0 = as soon as it starts. */
        val countAfterMs: Long,
    )

    /** [id] is a folder id (String) or an item id (Long). */
    class Entry(val id: Any, val playedAtMs: Long) {
        val key get() = id.toString()

        fun toJson(): JSONObject = JSONObject().put("id", id).put("playedAtMs", playedAtMs)
    }

    private val lock = Any()
    private var configs: List<ListConfig>? = null

    /** Validates and saves the list definitions. Lists that are no longer defined are dropped. */
    fun set(context: Context, json: JSONObject) {
        val listsJson = json.optJSONArray("lists") ?: JSONArray()
        val ids = HashSet<String>()
        val parsed = (0 until listsJson.length()).map { i ->
            val listJson = listsJson.getJSONObject(i)
            val id = listJson.optString("id")
            require(id.isNotEmpty()) { "lists[$i] requires id" }
            require(ids.add(id)) { "duplicate list id $id" }
            val track = listJson.optString("track")
            require(track == "folder" || track == "item") { "list $id: track must be folder or item" }
            val limit = listJson.optInt("limit", DEFAULT_LIMIT)
            require(limit > 0) { "list $id: limit must be > 0" }
            val countAfterSeconds = listJson.optDouble("countAfterSeconds", 0.0)
            require(countAfterSeconds.isFinite() && countAfterSeconds >= 0) { "list $id: countAfterSeconds must be >= 0" }
            ListConfig(id, track, limit, (countAfterSeconds * 1000.0).toLong())
        }

        synchronized(lock) {
            val lists = listsJson(context)
            val kept = JSONObject()
            for (config in parsed) {
                kept.put(config.id, toJson(entriesOf(lists, config.id).take(config.limit)))
            }
            val changes = changesJson(context)
            val keptChanges = JSONObject()
            for (config in parsed) changes.optJSONObject(config.id)?.let { keptChanges.put(config.id, it) }
            prefs(context).edit()
                .putString(KEY_CONFIGS, json.toString())
                .putString(KEY_LISTS, kept.toString())
                .putString(KEY_CHANGES, keptChanges.toString())
                .apply()
            configs = parsed
        }
    }

    fun configs(context: Context): List<ListConfig> {
        synchronized(lock) {
            configs?.let { return it }
            val loaded = prefs(context).getString(KEY_CONFIGS, null)?.let { saved ->
                runCatching {
                    val listsJson = JSONObject(saved).optJSONArray("lists") ?: JSONArray()
                    (0 until listsJson.length()).map { i ->
                        val listJson = listsJson.getJSONObject(i)
                        ListConfig(
                            id = listJson.getString("id"),
                            track = listJson.getString("track"),
                            limit = listJson.optInt("limit", DEFAULT_LIMIT),
                            countAfterMs = (listJson.optDouble("countAfterSeconds", 0.0) * 1000.0).toLong(),
                        )
                    }
                }.onFailure { Log.w(TAG, "failed to load tracked lists", it) }.getOrNull()
            } ?: emptyList()
            configs = loaded
            return loaded
        }
    }

    /** The list's entries, newest first. */
    fun entries(context: Context, listId: String): List<Entry> =
        synchronized(lock) { entriesOf(listsJson(context), listId) }

    /**
     * Sets a list from the app: [merge] combines [incoming] with the current entries (by id, newest
     * time wins), otherwise it replaces them. Not reported back as a change. Returns the result.
     */
    fun set(context: Context, listId: String, incoming: List<Entry>, merge: Boolean): List<Entry> {
        synchronized(lock) {
            val config = configs(context).firstOrNull { it.id == listId }
                ?: throw IllegalArgumentException("unknown list $listId")
            val lists = listsJson(context)
            val base = if (merge) entriesOf(lists, listId) else emptyList()
            val result = combine(base + incoming, config.limit)
            lists.put(listId, toJson(result))
            prefs(context).edit().putString(KEY_LISTS, lists.toString()).apply()
            return result
        }
    }

    /**
     * Records that [value] (a folder id or an item id) was played now. Returns the change to report
     * to the app, or null when the list didn't change.
     */
    fun record(context: Context, config: ListConfig, value: Any, nowMs: Long = System.currentTimeMillis()): JSONObject? {
        synchronized(lock) {
            val lists = listsJson(context)
            val current = entriesOf(lists, config.id)
            val updated = combine(current + Entry(value, nowMs), config.limit)
            if (toJson(updated).toString() == toJson(current).toString()) return null
            lists.put(config.id, toJson(updated))

            // Only the latest state of each list is waiting for the app.
            val change = JSONObject()
                .put("id", UUID.randomUUID().toString())
                .put("listId", config.id)
                .put("entries", toJson(updated))
                .put("ids", JSONArray().apply { updated.forEach { put(it.id) } })
                .put("changedAtMs", nowMs)
            val changes = changesJson(context).put(config.id, change)
            prefs(context).edit()
                .putString(KEY_LISTS, lists.toString())
                .putString(KEY_CHANGES, changes.toString())
                .apply()
            return change
        }
    }

    /** List changes the app hasn't acknowledged yet: the latest one per list. */
    fun pendingChanges(context: Context): JSONArray {
        synchronized(lock) {
            val changes = changesJson(context)
            val pending = changes.keys().asSequence().mapNotNull { changes.optJSONObject(it) }
                .sortedBy { it.optLong("changedAtMs") }
            return JSONArray().apply { pending.forEach { put(it) } }
        }
    }

    fun acknowledge(context: Context, ids: Set<String>) {
        synchronized(lock) {
            val changes = changesJson(context)
            val remaining = JSONObject()
            for (listId in changes.keys()) {
                val change = changes.optJSONObject(listId) ?: continue
                if (change.optString("id") !in ids) remaining.put(listId, change)
            }
            prefs(context).edit().putString(KEY_CHANGES, remaining.toString()).apply()
        }
    }

    /** Parses `[{ id, playedAtMs }]` from the app. Throws [IllegalArgumentException]. */
    fun parseEntries(json: JSONArray): List<Entry> = (0 until json.length()).map { i ->
        val entryJson = json.optJSONObject(i) ?: throw IllegalArgumentException("entries[$i] must be { id, playedAtMs }")
        require(entryJson.has("id") && !entryJson.isNull("id")) { "entries[$i] requires id" }
        require(entryJson.has("playedAtMs")) { "entries[$i] requires playedAtMs" }
        Entry(normalizeId(entryJson.get("id")), entryJson.getLong("playedAtMs"))
    }

    fun toJson(entries: List<Entry>): JSONArray = JSONArray().apply { entries.forEach { put(it.toJson()) } }

    /** One entry per id with its newest time, newest first, at most [limit]. */
    internal fun combine(entries: List<Entry>, limit: Int): List<Entry> =
        entries.groupBy { it.key }
            .map { (_, sameId) -> sameId.maxBy { it.playedAtMs } }
            .sortedByDescending { it.playedAtMs }
            .take(limit)

    private fun entriesOf(lists: JSONObject, listId: String): List<Entry> {
        val json = lists.optJSONArray(listId) ?: return emptyList()
        return (0 until json.length()).map { i ->
            when (val value = json.get(i)) {
                is JSONObject -> Entry(normalizeId(value.get("id")), value.optLong("playedAtMs", 0L))
                // Saved before entries had times.
                else -> Entry(normalizeId(value), 0L)
            }
        }
    }

    /** Item ids as Long, folder ids as String. */
    private fun normalizeId(value: Any): Any = if (value is Number) value.toLong() else value.toString()

    private fun listsJson(context: Context): JSONObject =
        runCatching { JSONObject(prefs(context).getString(KEY_LISTS, "{}")!!) }.getOrElse { JSONObject() }

    private fun changesJson(context: Context): JSONObject =
        runCatching { JSONObject(prefs(context).getString(KEY_CHANGES, "{}")!!) }.getOrElse { JSONObject() }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
