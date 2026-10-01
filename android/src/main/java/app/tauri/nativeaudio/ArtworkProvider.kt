package app.tauri.nativeaudio

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Rect
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Log
import java.io.File
import java.io.FileNotFoundException
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "plugin/native-audio"
private const val REGISTRY_FILE_NAME = "native_audio_artwork.json"
private const val CACHE_DIR_NAME = "native_audio_artwork"
private const val COLLAGE_SIZE = 512
private val IMAGE_EXTENSIONS = setOf("jpg", "jpeg", "png", "webp", "gif", "bmp")

/**
 * Artwork Android Auto (and the system media controls) can't read on their own: image files
 * inside the app, covers embedded in audio files, and 2x2 collages for folders. Each is registered
 * under an opaque key, and only registered images are ever served, so the provider doesn't expose
 * other files of the app. Embedded covers and collages are made on first request and cached.
 */
internal object ArtworkRegistry {
    private class Entry(val kind: String, val sources: List<String>)

    private val lock = Any()
    private var entries: MutableMap<String, Entry>? = null
    private val saveHandler = android.os.Handler(android.os.Looper.getMainLooper())
    private var saveScheduled = false

    /**
     * The URI to give Android Auto for [artworkUrl]: remote and content URIs as they are, local
     * image files through the provider. Without one, the cover embedded in [audioSrc] when
     * [embedded] is on. Null when there's nothing to show.
     */
    fun artworkUri(context: Context, artworkUrl: String?, audioSrc: String?, embedded: Boolean): Uri? {
        if (!artworkUrl.isNullOrBlank()) {
            val path = localPath(artworkUrl) ?: return runCatching { Uri.parse(artworkUrl) }.getOrNull()
            if (File(path).extension.lowercase() !in IMAGE_EXTENSIONS) return null
            return uri(context, register(context, "file", listOf(path)))
        }
        if (!embedded) return null
        val path = audioSrc?.let(::localPath) ?: return null
        return uri(context, register(context, "embedded", listOf(path)))
    }

    /** A collage of up to 4 covers (from [artworkUri] calls), or the single cover when there's one. */
    fun collageUri(context: Context, covers: List<Uri>): Uri? {
        if (covers.isEmpty()) return null
        if (covers.size < 4) return covers.first()
        val keys = covers.take(4).map { cover -> keyOf(cover) ?: return covers.first() }
        return uri(context, register(context, "collage", keys))
    }

    /** The registered entry for a provider URI's key, as (kind, sources). */
    fun lookup(context: Context, key: String): Pair<String, List<String>>? =
        synchronized(lock) { load(context)[key]?.let { it.kind to it.sources } }

    private fun register(context: Context, kind: String, sources: List<String>): String {
        val key = sha1("$kind|${sources.joinToString("|")}")
        synchronized(lock) {
            val loaded = load(context)
            if (key !in loaded) {
                loaded[key] = Entry(kind, sources)
                // Browsing a large library registers many covers at once: save them together.
                if (!saveScheduled) {
                    saveScheduled = true
                    val appContext = context.applicationContext
                    saveHandler.postDelayed({
                        synchronized(lock) {
                            saveScheduled = false
                            save(appContext, load(appContext))
                        }
                    }, 1_000)
                }
            }
        }
        return key
    }

    private fun uri(context: Context, key: String): Uri =
        Uri.Builder().scheme("content").authority(authority(context)).appendPath(key).build()

    /** The registry key of a URI this registry made, or null for other URIs. */
    private fun keyOf(uri: Uri): String? = uri.lastPathSegment?.takeIf { uri.scheme == "content" }

    fun authority(context: Context) = "${context.packageName}.nativeaudio.artwork"

    private fun localPath(url: String): String? = when {
        url.startsWith("/") -> url
        url.startsWith("file://") -> Uri.parse(url).path
        else -> null
    }

    private fun load(context: Context): MutableMap<String, Entry> {
        entries?.let { return it }
        val loaded = HashMap<String, Entry>()
        runCatching {
            val file = File(context.applicationContext.filesDir, REGISTRY_FILE_NAME)
            if (file.exists()) {
                val json = JSONObject(file.readText())
                for (key in json.keys()) {
                    val entry = json.getJSONObject(key)
                    val sources = entry.getJSONArray("sources")
                    loaded[key] = Entry(entry.getString("kind"), (0 until sources.length()).map { sources.getString(it) })
                }
            }
        }.onFailure { Log.w(TAG, "failed to load artwork registry", it) }
        entries = loaded
        return loaded
    }

    private fun save(context: Context, loaded: Map<String, Entry>) {
        val json = JSONObject()
        for ((key, entry) in loaded) {
            json.put(key, JSONObject().put("kind", entry.kind).put("sources", JSONArray(entry.sources)))
        }
        runCatching { File(context.applicationContext.filesDir, REGISTRY_FILE_NAME).writeText(json.toString()) }
            .onFailure { Log.w(TAG, "failed to save artwork registry", it) }
    }

    private fun sha1(text: String): String =
        MessageDigest.getInstance("SHA-1").digest(text.toByteArray()).joinToString("") { "%02x".format(it) }
}

/** Serves [ArtworkRegistry] images to Android Auto and the system media controls. */
class ArtworkProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    override fun getType(uri: Uri): String = "image/*"

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("artwork is read-only")
        val context = context ?: throw FileNotFoundException("no context")
        val key = uri.lastPathSegment ?: throw FileNotFoundException("no key")
        val (kind, sources) = ArtworkRegistry.lookup(context, key) ?: throw FileNotFoundException("unknown artwork")
        val file = when (kind) {
            "file" -> File(sources.first())
            "embedded" -> cached(context, key) { extractEmbedded(sources.first()) }
            "collage" -> cached(context, key) { buildCollage(context, sources) }
            else -> throw FileNotFoundException("unknown artwork")
        }
        if (!file.exists()) throw FileNotFoundException("artwork missing")
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    /** The cached image for [key], made with [create] (image bytes) on first use. */
    private fun cached(context: Context, key: String, create: () -> ByteArray?): File {
        val dir = File(context.cacheDir, CACHE_DIR_NAME).apply { mkdirs() }
        val file = File(dir, key)
        synchronized(this) {
            if (!file.exists()) {
                val bytes = create() ?: throw FileNotFoundException("no artwork")
                val partial = File(dir, "$key.tmp")
                partial.writeBytes(bytes)
                partial.renameTo(file)
            }
        }
        return file
    }

    private fun extractEmbedded(audioPath: String): ByteArray? {
        val retriever = MediaMetadataRetriever()
        return try {
            retriever.setDataSource(audioPath)
            retriever.embeddedPicture
        } catch (e: RuntimeException) {
            Log.w(TAG, "can't read embedded artwork of $audioPath", e)
            null
        } finally {
            retriever.release()
        }
    }

    /** A 2x2 grid of the four covers behind the registry keys in [keys], as a JPEG. */
    private fun buildCollage(context: Context, keys: List<String>): ByteArray? {
        val tile = COLLAGE_SIZE / 2
        val bitmaps = keys.map { key -> loadSquare(context, key, tile) ?: return null }
        val collage = Bitmap.createBitmap(COLLAGE_SIZE, COLLAGE_SIZE, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(collage)
        bitmaps.forEachIndexed { i, bitmap ->
            val left = (i % 2) * tile
            val top = (i / 2) * tile
            canvas.drawBitmap(bitmap, null, Rect(left, top, left + tile, top + tile), null)
            bitmap.recycle()
        }
        val out = java.io.ByteArrayOutputStream()
        collage.compress(Bitmap.CompressFormat.JPEG, 90, out)
        collage.recycle()
        return out.toByteArray()
    }

    /** One cover, center-cropped to a [size] square. */
    private fun loadSquare(context: Context, key: String, size: Int): Bitmap? {
        val (kind, sources) = ArtworkRegistry.lookup(context, key) ?: return null
        val bytes = when (kind) {
            "file" -> runCatching { File(sources.first()).readBytes() }.getOrNull()
            "embedded" -> runCatching { cached(context, key) { extractEmbedded(sources.first()) }.readBytes() }.getOrNull()
            else -> null
        } ?: return null

        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= size) sample *= 2
        val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
            ?: return null

        val side = minOf(decoded.width, decoded.height)
        val square = Bitmap.createBitmap(decoded, (decoded.width - side) / 2, (decoded.height - side) / 2, side, side)
        val scaled = Bitmap.createScaledBitmap(square, size, size, true)
        if (square !== decoded) square.recycle()
        if (scaled !== decoded) decoded.recycle()
        return scaled
    }

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0
}
