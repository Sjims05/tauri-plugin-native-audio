package app.tauri.nativeaudio

import android.content.Context
import android.util.Log
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "plugin/native-audio"
private const val LIBRARY_FILE_NAME = "native_audio_library.json"

/**
 * The browsable library set from JS with `setLibrary`. It's saved to a file, because Android Auto
 * can start the audio service to browse while the app (and its JS) isn't running.
 *
 * The plugin gives the tree no meaning of its own: folders can be albums, playlists, podcasts or
 * anything else, and items are defined once and referenced by id from any number of folders.
 */
internal object MediaLibraryStore {
    class Item(
        val id: Long,
        val src: String,
        val title: String?,
        val artist: String?,
        val album: String?,
        val artworkUrl: String?,
        /** Header Android Auto shows above a run of items with the same group, e.g. "A". */
        val group: String?,
        /** 0 to 1, shown as not played / partly played / played. Null shows nothing. */
        val progress: Double?,
    )

    class Folder(
        val id: String,
        val title: String,
        val subtitle: String?,
        val artworkUrl: String?,
        /** Whether the folder can be played as a whole (albums, playlists). */
        val playable: Boolean,
        /** How Android Auto lays out this folder's children: "list" or "grid". */
        val style: String?,
        val children: List<Node>,
        /** Shows a tracked list instead of [children] (see TrackedLists). */
        val trackedList: String?,
        val group: String?,
    )

    sealed interface Node {
        class FolderNode(val folder: Folder) : Node
        class ItemNode(val itemId: Long) : Node
    }

    class Library(
        val items: Map<Long, Item>,
        val root: List<Folder>,
        val folders: Map<String, Folder>,
        /** Whether Android Auto offers search (and "play ... on <app>" by voice). */
        val search: Boolean,
        /** Shown in Android Auto when [root] is empty, e.g. "Open the app to sync". */
        val emptyMessage: String?,
        /** Items without artworkUrl show the cover embedded in their audio file. */
        val embeddedArtwork: Boolean,
        /** Folders without artworkUrl: "collage" (2x2 of their covers), "first" cover, or "none". */
        val folderArtwork: String,
    ) {
        companion object {
            val EMPTY = Library(
                emptyMap(), emptyList(), emptyMap(),
                search = true, emptyMessage = null, embeddedArtwork = true, folderArtwork = "collage",
            )
        }
    }

    private val lock = Any()
    private var library: Library? = null

    /** Validates and saves a library. Throws [IllegalArgumentException] describing what's wrong. */
    fun set(context: Context, json: JSONObject) {
        val parsed = parse(json)
        file(context).writeText(json.toString())
        synchronized(lock) {
            library = parsed
        }
    }

    fun get(context: Context): Library {
        synchronized(lock) {
            library?.let { return it }
            val loaded = runCatching {
                val file = file(context)
                if (file.exists()) parse(JSONObject(file.readText())) else Library.EMPTY
            }.getOrElse {
                Log.w(TAG, "failed to load saved library", it)
                Library.EMPTY
            }
            library = loaded
            return loaded
        }
    }

    private fun file(context: Context) = File(context.applicationContext.filesDir, LIBRARY_FILE_NAME)

    private fun parse(json: JSONObject): Library {
        val items = LinkedHashMap<Long, Item>()
        val itemsJson = json.optJSONArray("items") ?: JSONArray()
        for (i in 0 until itemsJson.length()) {
            val itemJson = itemsJson.getJSONObject(i)
            require(itemJson.has("id")) { "items[$i] requires id" }
            val id = itemJson.getLong("id")
            val src = itemJson.optStringOrNull("src")
            require(!src.isNullOrBlank()) { "item $id requires src" }
            require(id !in items) { "duplicate item id $id" }
            items[id] = Item(
                id = id,
                src = src.trim(),
                title = itemJson.optStringOrNull("title"),
                artist = itemJson.optStringOrNull("artist"),
                album = itemJson.optStringOrNull("album"),
                artworkUrl = itemJson.optStringOrNull("artworkUrl"),
                group = itemJson.optStringOrNull("group"),
                progress = parseProgress(itemJson, "item $id"),
            )
        }

        val folders = HashMap<String, Folder>()
        fun parseFolder(folderJson: JSONObject, path: String): Folder {
            val id = folderJson.optStringOrNull("id")
            require(!id.isNullOrEmpty()) { "$path requires id" }
            require(id !in folders) { "duplicate folder id $id" }
            val title = folderJson.optStringOrNull("title")
            require(!title.isNullOrBlank()) { "folder $id requires title" }
            val style = folderJson.optStringOrNull("style")
            require(style == null || style == "list" || style == "grid") { "folder $id: style must be list or grid" }

            val children = ArrayList<Node>()
            val childrenJson = folderJson.optJSONArray("children") ?: JSONArray()
            for (i in 0 until childrenJson.length()) {
                when (val child = childrenJson.get(i)) {
                    is JSONObject -> children.add(Node.FolderNode(parseFolder(child, "folder $id children[$i]")))
                    is Number -> {
                        val itemId = child.toLong()
                        require(itemId in items) { "folder $id refers to unknown item $itemId" }
                        children.add(Node.ItemNode(itemId))
                    }
                    else -> throw IllegalArgumentException("folder $id children[$i] must be a folder or an item id")
                }
            }

            val folder = Folder(
                id = id,
                title = title,
                subtitle = folderJson.optStringOrNull("subtitle"),
                artworkUrl = folderJson.optStringOrNull("artworkUrl"),
                playable = folderJson.optBoolean("playable", false),
                style = style,
                children = children,
                trackedList = folderJson.optStringOrNull("trackedList")?.takeIf { it.isNotEmpty() },
                group = folderJson.optStringOrNull("group"),
            )
            folders[id] = folder
            return folder
        }

        val rootJson = json.optJSONArray("root") ?: JSONArray()
        val root = (0 until rootJson.length()).map { parseFolder(rootJson.getJSONObject(it), "root[$it]") }
        return Library(
            items = items,
            root = root,
            folders = folders,
            search = json.optBoolean("search", true),
            emptyMessage = json.optStringOrNull("emptyMessage")?.takeIf { it.isNotBlank() },
            embeddedArtwork = json.optBoolean("embeddedArtwork", true),
            folderArtwork = (json.optStringOrNull("folderArtwork") ?: "collage").also {
                require(it == "collage" || it == "first" || it == "none") { "folderArtwork must be collage, first or none" }
            },
        )
    }

    private fun parseProgress(json: JSONObject, path: String): Double? {
        if (!json.has("progress") || json.isNull("progress")) return null
        val progress = json.getDouble("progress")
        require(progress.isFinite() && progress in 0.0..1.0) { "$path: progress must be between 0 and 1" }
        return progress
    }

    private fun JSONObject.optStringOrNull(key: String): String? =
        if (has(key) && !isNull(key)) getString(key) else null
}
