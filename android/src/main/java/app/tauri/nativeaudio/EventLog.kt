package app.tauri.nativeaudio

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/**
 * A saved list of events (button presses, playback events) for the app to pick up, including those
 * from while it wasn't running. Each entry has a unique `id`; entries stay until the app
 * acknowledges them. Only the newest [maxEntries] are kept.
 */
internal class EventLog(private val prefsName: String, private val key: String, private val maxEntries: Int) {
    private val lock = Any()

    fun append(context: Context, entry: JSONObject) {
        synchronized(lock) {
            val entries = read(context)
            entries.put(entry)
            val kept = if (entries.length() > maxEntries) {
                JSONArray().apply { for (i in entries.length() - maxEntries until entries.length()) put(entries.get(i)) }
            } else {
                entries
            }
            write(context, kept)
        }
    }

    /** Entries not acknowledged yet, oldest first. */
    fun pending(context: Context): JSONArray = synchronized(lock) { read(context) }

    fun acknowledge(context: Context, ids: Set<String>) {
        synchronized(lock) {
            val entries = read(context)
            val kept = JSONArray()
            for (i in 0 until entries.length()) {
                val entry = entries.getJSONObject(i)
                if (entry.optString("id") !in ids) kept.put(entry)
            }
            write(context, kept)
        }
    }

    private fun read(context: Context): JSONArray =
        runCatching { JSONArray(prefs(context).getString(key, "[]")) }.getOrElse { JSONArray() }

    private fun write(context: Context, entries: JSONArray) {
        prefs(context).edit().putString(key, entries.toString()).apply()
    }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(prefsName, Context.MODE_PRIVATE)
}
