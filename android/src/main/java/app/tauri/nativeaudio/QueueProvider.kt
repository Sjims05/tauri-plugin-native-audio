package app.tauri.nativeaudio

import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.util.concurrent.Callable
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject

/**
 * An app's helper for when the player runs without the app's own screen (Android Auto with the app closed):
 * it can hand the player a queue when it starts with nothing loaded, put entries at the top of a library
 * folder (each with its own queue), and hear about playback. The app names its class in its manifest:
 *
 * ```xml
 * <meta-data android:name="app.tauri.nativeaudio.QUEUE_PROVIDER" android:value="com.example.CarQueue" />
 * ```
 *
 * The class needs a constructor without arguments. Every call runs on a background thread; the player waits a
 * few seconds at most, then goes on without the answer.
 */
interface QueueProvider {
    /** The player started with nothing loaded: a queue to load (paused) instead of the last one, or null. */
    fun queueAtStart(context: Context): ProvidedQueue? = null

    /** Entries shown first in a library folder (its id as given in setLibrary). */
    fun entries(context: Context, folderId: String): List<ProvidedEntry> = emptyList()

    /** An entry was picked: its queue (it plays), or null. */
    fun queueForEntry(context: Context, entryId: String): ProvidedQueue? = null

    /** What plays changed (the song, play or pause, shuffle or repeat; while playing, now and then). */
    fun onPlayback(context: Context, playback: ProvidedPlayback) {}

    /**
     * A setControls custom button was pressed (`active`: a toggle's new state). Return true when it's handled:
     * then it isn't handed to the app later (its press log entry is acknowledged).
     */
    fun onControlPress(context: Context, buttonId: String, itemId: Long?, active: Boolean?): Boolean = false

    /**
     * A playback event as logged for the app (`id`, `type` start / complete / skip, `itemId`, `positionMs`,
     * `atMs`). The app still gets it later: the ids let it count each one once.
     */
    fun onPlaybackEvent(context: Context, event: JSONObject) {}
}

/** A queue from the provider: items as setQueue takes them. */
data class ProvidedQueue(
    val items: List<SetSourceArgs>,
    val index: Int,
    val positionMs: Long,
    val shuffle: Boolean,
    /** "off", "all" or "one". */
    val repeatMode: String,
)

/** A library entry from the provider (playable: picking it plays [QueueProvider.queueForEntry]). */
data class ProvidedEntry(val id: String, val title: String, val subtitle: String?)

/** What plays, for [QueueProvider.onPlayback]. */
data class ProvidedPlayback(
    /** The queue's item ids, in queue order (null: an item without one). */
    val itemIds: List<Long?>,
    val index: Int,
    val positionMs: Long,
    val playing: Boolean,
    val shuffle: Boolean,
    val repeatMode: String,
    val title: String?,
    val artist: String?,
)

/** The app's provider, if it has one, and asking it with a time limit. */
internal object QueueProviders {
    private const val TAG = "NativeAudio"
    private const val META_DATA = "app.tauri.nativeaudio.QUEUE_PROVIDER"

    @Volatile private var loaded = false
    @Volatile private var provider: QueueProvider? = null
    private val askers: ExecutorService = Executors.newCachedThreadPool { Thread(it, "native-audio-provider") }
    private val playbackThread: ExecutorService = Executors.newSingleThreadExecutor { Thread(it, "native-audio-provider-playback") }
    // Presses and playback events: each one, in order.
    private val eventThread: ExecutorService = Executors.newSingleThreadExecutor { Thread(it, "native-audio-provider-events") }
    // The latest playback not yet handed over (older ones are dropped: only the latest matters).
    private val pendingPlayback = AtomicReference<ProvidedPlayback?>(null)
    private val main = Handler(Looper.getMainLooper())

    fun get(context: Context): QueueProvider? {
        if (loaded) return provider
        synchronized(this) {
            if (loaded) return provider
            provider = runCatching {
                val info = context.packageManager.getApplicationInfo(context.packageName, PackageManager.GET_META_DATA)
                val name = info.metaData?.getString(META_DATA) ?: return@runCatching null
                Class.forName(name).getDeclaredConstructor().newInstance() as QueueProvider
            }.onFailure { Log.w(TAG, "can't load the queue provider: $it") }.getOrNull()
            loaded = true
            return provider
        }
    }

    /** Asks the provider, waiting at most [timeoutMs] (here, on this thread). Null: no provider, no answer. */
    fun <T> ask(context: Context, timeoutMs: Long, question: (QueueProvider) -> T): T? {
        val p = get(context) ?: return null
        val answer = askers.submit(Callable { question(p) })
        return runCatching { answer.get(timeoutMs, TimeUnit.MILLISECONDS) }
            .onFailure { Log.w(TAG, "the queue provider didn't answer: $it") }
            .getOrNull()
    }

    /** Asks the provider in the background; [then] gets the answer (or null) on the main thread. */
    fun <T> askLater(context: Context, timeoutMs: Long, question: (QueueProvider) -> T, then: (T?) -> Unit) {
        if (get(context) == null) return
        askers.execute {
            val answer = ask(context, timeoutMs, question)
            main.post { then(answer) }
        }
    }

    /** Hands a button press to the provider (in the background, in order); a handled one is acknowledged. */
    fun controlPressed(context: Context, press: PlaybackControls.Press) {
        val p = get(context) ?: return
        eventThread.execute {
            val handled = runCatching { p.onControlPress(context, press.buttonId, press.itemId, press.active) }
                .onFailure { Log.w(TAG, "the queue provider failed: $it") }
                .getOrDefault(false)
            if (handled) PlaybackControls.acknowledge(context, setOf(press.id))
        }
    }

    /** Hands a playback event to the provider (in the background, in order). */
    fun playbackEvent(context: Context, event: JSONObject) {
        val p = get(context) ?: return
        eventThread.execute {
            runCatching { p.onPlaybackEvent(context, event) }.onFailure { Log.w(TAG, "the queue provider failed: $it") }
        }
    }

    /** Hands playback to the provider (in the background, the latest only). */
    fun playbackChanged(context: Context, playback: ProvidedPlayback) {
        val p = get(context) ?: return
        if (pendingPlayback.getAndSet(playback) != null) return
        playbackThread.execute {
            val latest = pendingPlayback.getAndSet(null) ?: return@execute
            runCatching { p.onPlayback(context, latest) }.onFailure { Log.w(TAG, "the queue provider failed: $it") }
        }
    }
}
