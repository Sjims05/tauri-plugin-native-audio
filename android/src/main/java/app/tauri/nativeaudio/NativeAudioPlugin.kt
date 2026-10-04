package app.tauri.nativeaudio

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

// The plugin's commands: they check the arguments and hand over to NativeAudioRuntime.

@TauriPlugin
class NativeAudioPlugin(private val activity: Activity) : Plugin(activity) {

    init {
        activeInstance = this
    }

    @Command
    fun initialize(invoke: Invoke) {
        requestNotificationPermission()
        runCatching {
            NativeAudioRuntime.initialize(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "initialize failed")
        }
    }

    @Command
    fun register_listener(invoke: Invoke) {
        invoke.resolve()
    }

    @Command
    fun remove_listener(invoke: Invoke) {
        invoke.resolve()
    }

    @Command
    fun setSource(invoke: Invoke) {
        val args = invoke.parseArgs(SetSourceArgs::class.java)
        val src = args.src?.trim().orEmpty()
        if (src.isEmpty()) {
            invoke.reject("src is required")
            return
        }

        runCatching {
            NativeAudioRuntime.setSource(activity.applicationContext, src, args.id, args.title, args.artist, args.artworkUrl)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setSource failed")
        }
    }

    @Command
    fun setQueue(invoke: Invoke) {
        val args = invoke.parseArgs(SetQueueArgs::class.java)
        val items = args.items?.toList().orEmpty()
        if (items.isEmpty()) {
            invoke.reject("items must not be empty")
            return
        }
        if (items.any { it.src.isNullOrBlank() }) {
            invoke.reject("every item requires src")
            return
        }
        val startIndex = args.startIndex ?: 0
        if (startIndex < 0 || startIndex >= items.size) {
            invoke.reject("startIndex out of range")
            return
        }

        runCatching {
            NativeAudioRuntime.setQueue(activity.applicationContext, items, startIndex, args.startPosition ?: 0.0, args.sourceId)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setQueue failed")
        }
    }

    @Command
    fun next(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.next(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "next failed")
        }
    }

    @Command
    fun previous(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.previous(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "previous failed")
        }
    }

    @Command
    fun skipTo(invoke: Invoke) {
        val index = invoke.parseArgs(SkipToArgs::class.java).index
        if (index == null) {
            invoke.reject("index is required")
            return
        }

        runCatching {
            NativeAudioRuntime.skipTo(activity.applicationContext, index)
        }.onSuccess { inRange ->
            if (inRange) {
                invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
            } else {
                invoke.reject("index out of range")
            }
        }.onFailure {
            invoke.reject(it.message ?: "skipTo failed")
        }
    }

    @Command
    fun play(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.play(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "play failed")
        }
    }

    @Command
    fun pause(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.pause(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "pause failed")
        }
    }

    @Command
    fun seekTo(invoke: Invoke) {
        val args = invoke.parseArgs(SeekToArgs::class.java)
        val position = args.position
        if (position == null || !position.isFinite()) {
            invoke.reject("position is required")
            return
        }

        runCatching {
            NativeAudioRuntime.seekTo(activity.applicationContext, position)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "seekTo failed")
        }
    }

    @Command
    fun setRate(invoke: Invoke) {
        val args = invoke.parseArgs(SetRateArgs::class.java)
        val rate = args.rate
        if (rate == null || !rate.isFinite() || rate <= 0) {
            invoke.reject("rate must be > 0")
            return
        }

        runCatching {
            NativeAudioRuntime.setRate(activity.applicationContext, rate)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setRate failed")
        }
    }

    @Command
    fun setVolume(invoke: Invoke) {
        val volume = invoke.parseArgs(SetVolumeArgs::class.java).volume
        if (volume == null || !volume.isFinite()) {
            invoke.reject("volume must be a number from 0 to 1")
            return
        }

        runCatching {
            NativeAudioRuntime.setVolume(activity.applicationContext, volume)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setVolume failed")
        }
    }

    /** Desktop only: Android picks the output itself (speaker, headphones, Bluetooth, the car). */
    @Command
    fun getOutputDevices(invoke: Invoke) {
        val result = JSObject()
        result.put("devices", JSArray())
        result.put("selected", JSONObject.NULL)
        result.put("active", JSONObject.NULL)
        invoke.resolve(result)
    }

    /** Desktop only, see getOutputDevices. */
    @Command
    fun setOutputDevice(invoke: Invoke) {
        getOutputDevices(invoke)
    }

    @Command
    fun setShuffle(invoke: Invoke) {
        val enabled = invoke.parseArgs(SetShuffleArgs::class.java).enabled
        if (enabled == null) {
            invoke.reject("enabled is required")
            return
        }

        runCatching {
            NativeAudioRuntime.setShuffle(activity.applicationContext, enabled)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setShuffle failed")
        }
    }

    @Command
    fun addToQueue(invoke: Invoke) {
        val args = invoke.parseArgs(AddToQueueArgs::class.java)
        val items = args.items?.toList().orEmpty()
        if (items.isEmpty()) {
            invoke.reject("items must not be empty")
            return
        }
        if (items.any { it.src.isNullOrBlank() }) {
            invoke.reject("every item requires src")
            return
        }

        runCatching {
            NativeAudioRuntime.addToQueue(activity.applicationContext, items, args.playNext ?: false)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "addToQueue failed")
        }
    }

    @Command
    fun updateQueue(invoke: Invoke) {
        val args = invoke.parseArgs(UpdateQueueArgs::class.java)
        val items = args.items?.toList().orEmpty()
        if (items.any { it.src.isNullOrBlank() }) {
            invoke.reject("every item requires src")
            return
        }

        fun pick(value: String?, name: String, default: String, other: String): Boolean? = when (value ?: default) {
            default -> false
            other -> true
            else -> {
                invoke.reject("$name must be $default or $other")
                null
            }
        }
        val skipRemovedCurrent = pick(args.removedCurrent, "removedCurrent", "finish", "skip") ?: return
        val keepRemoved = pick(args.removedItems, "removedItems", "remove", "keep") ?: return
        val keepQueueOrder = pick(args.order, "order", "playlist", "queue") ?: return
        val newItems = args.newItems ?: "inPlace"
        if (newItems !in setOf("inPlace", "end", "next")) {
            invoke.reject("newItems must be inPlace, end or next")
            return
        }

        runCatching {
            NativeAudioRuntime.updateQueue(activity.applicationContext, items, args.sourceId, skipRemovedCurrent, keepRemoved, newItems, keepQueueOrder)
        }.onSuccess { (added, removed) ->
            val result = JSObject()
            result.put("state", toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
            result.put("added", added)
            result.put("removed", removed)
            invoke.resolve(result)
        }.onFailure {
            invoke.reject(it.message ?: "updateQueue failed")
        }
    }

    @Command
    fun removeFromQueue(invoke: Invoke) {
        val index = invoke.parseArgs(SkipToArgs::class.java).index
        if (index == null) {
            invoke.reject("index is required")
            return
        }

        runCatching {
            NativeAudioRuntime.removeFromQueue(activity.applicationContext, index)
        }.onSuccess { inRange ->
            if (inRange) {
                invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
            } else {
                invoke.reject("index out of range")
            }
        }.onFailure {
            invoke.reject(it.message ?: "removeFromQueue failed")
        }
    }

    @Command
    fun moveInQueue(invoke: Invoke) {
        val args = invoke.parseArgs(MoveInQueueArgs::class.java)
        val from = args.from
        val to = args.to
        if (from == null || to == null) {
            invoke.reject("from and to are required")
            return
        }

        runCatching {
            NativeAudioRuntime.moveInQueue(activity.applicationContext, from, to)
        }.onSuccess { inRange ->
            if (inRange) {
                invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
            } else {
                invoke.reject("index out of range")
            }
        }.onFailure {
            invoke.reject(it.message ?: "moveInQueue failed")
        }
    }

    @Command
    fun getQueue(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.getQueue(activity.applicationContext)
        }.onSuccess { queue ->
            val items = JSArray()
            queue.items.forEach { items.put(queueItemToJsObject(it)) }
            val playOrder = JSArray()
            queue.playOrder.forEach { playOrder.put(it) }
            val payload = JSObject()
            payload.put("items", items)
            payload.put("currentIndex", queue.currentIndex)
            payload.put("playOrder", playOrder)
            payload.put("sourceId", queue.sourceId)
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getQueue failed")
        }
    }

    @Command
    fun restoreLastQueue(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.restoreLastQueue(activity.applicationContext)
        }.onSuccess { restored ->
            if (restored) {
                invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
            } else {
                // Nothing saved: resolves with null.
                invoke.resolve()
            }
        }.onFailure {
            invoke.reject(it.message ?: "restoreLastQueue failed")
        }
    }

    @Command
    fun setOptions(invoke: Invoke) {
        val args = invoke.parseArgs(SetOptionsArgs::class.java)
        runCatching {
            NativeAudioRuntime.setOptions(
                activity.applicationContext,
                args.resumeLastQueue,
                args.repeatAddedTracks,
                args.pausedKeepAliveMinutes,
                args.keepAliveWhileCarConnected,
                args.trackProgress,
                args.keepQueueOnStop,
                args.volumeCurve,
                args.previousRestartsAfterSeconds,
                args.nextAtEnd,
            )
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "setOptions failed")
        }
    }

    @Command
    fun setControls(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.setControls(activity.applicationContext, invoke.getArgs())
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "setControls failed")
        }
    }

    @Command
    fun setControlActive(invoke: Invoke) {
        val args = invoke.parseArgs(SetControlActiveArgs::class.java)
        val buttonId = args.buttonId
        val itemIds = args.itemIds
        val active = args.active
        if (buttonId.isNullOrEmpty() || itemIds == null || active == null) {
            invoke.reject("buttonId, itemIds and active are required")
            return
        }

        runCatching {
            NativeAudioRuntime.setControlActive(activity.applicationContext, buttonId, itemIds.toList(), active)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "setControlActive failed")
        }
    }

    @Command
    fun getControlPresses(invoke: Invoke) {
        runCatching {
            PlaybackControls.pendingPresses(activity.applicationContext)
        }.onSuccess { presses ->
            val payload = JSObject()
            payload.put("presses", JSArray(presses.toString()))
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getControlPresses failed")
        }
    }

    @Command
    fun acknowledgeControlPresses(invoke: Invoke) {
        val ids = invoke.parseArgs(AcknowledgeControlPressesArgs::class.java).ids?.toSet().orEmpty()
        runCatching {
            PlaybackControls.acknowledge(activity.applicationContext, ids)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "acknowledgeControlPresses failed")
        }
    }

    @Command
    fun setTrackedLists(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.setTrackedLists(activity.applicationContext, invoke.getArgs())
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "setTrackedLists failed")
        }
    }

    @Command
    fun getTrackedList(invoke: Invoke) {
        val id = invoke.parseArgs(TrackedListArgs::class.java).id
        if (id.isNullOrEmpty()) {
            invoke.reject("id is required")
            return
        }
        runCatching {
            TrackedLists.entries(activity.applicationContext, id)
        }.onSuccess { entries ->
            val payload = JSObject()
            payload.put("entries", JSArray(TrackedLists.toJson(entries).toString()))
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getTrackedList failed")
        }
    }

    @Command
    fun setTrackedList(invoke: Invoke) {
        val args = invoke.getArgs()
        val id = args.optString("id")
        val entries = args.optJSONArray("entries")
        if (id.isEmpty() || entries == null) {
            invoke.reject("id and entries are required")
            return
        }
        runCatching {
            NativeAudioRuntime.setTrackedList(activity.applicationContext, id, entries, args.optBoolean("merge", false))
        }.onSuccess { result ->
            val payload = JSObject()
            payload.put("entries", JSArray(TrackedLists.toJson(result).toString()))
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "setTrackedList failed")
        }
    }

    @Command
    fun getTrackedListChanges(invoke: Invoke) {
        runCatching {
            TrackedLists.pendingChanges(activity.applicationContext)
        }.onSuccess { changes ->
            val payload = JSObject()
            payload.put("changes", JSArray(changes.toString()))
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getTrackedListChanges failed")
        }
    }

    @Command
    fun acknowledgeTrackedListChanges(invoke: Invoke) {
        val ids = invoke.parseArgs(AcknowledgeIdsArgs::class.java).ids?.toSet().orEmpty()
        runCatching {
            TrackedLists.acknowledge(activity.applicationContext, ids)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "acknowledgeTrackedListChanges failed")
        }
    }

    @Command
    fun getPlaybackEvents(invoke: Invoke) {
        runCatching {
            PlaybackEvents.pending(activity.applicationContext)
        }.onSuccess { events ->
            val payload = JSObject()
            payload.put("events", JSArray(events.toString()))
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getPlaybackEvents failed")
        }
    }

    @Command
    fun acknowledgePlaybackEvents(invoke: Invoke) {
        val ids = invoke.parseArgs(AcknowledgeIdsArgs::class.java).ids?.toSet().orEmpty()
        runCatching {
            PlaybackEvents.acknowledge(activity.applicationContext, ids)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "acknowledgePlaybackEvents failed")
        }
    }

    @Command
    fun setLibrary(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.setLibrary(activity.applicationContext, invoke.getArgs())
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "setLibrary failed")
        }
    }

    @Command
    fun setRepeatMode(invoke: Invoke) {
        val mode = when (invoke.parseArgs(SetRepeatModeArgs::class.java).mode) {
            "off" -> Player.REPEAT_MODE_OFF
            "all" -> Player.REPEAT_MODE_ALL
            "one" -> Player.REPEAT_MODE_ONE
            else -> {
                invoke.reject("mode must be off, all or one")
                return
            }
        }

        runCatching {
            NativeAudioRuntime.setRepeatMode(activity.applicationContext, mode)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setRepeatMode failed")
        }
    }

    @Command
    fun getItemProgress(invoke: Invoke) {
        val itemIds = invoke.parseArgs(ItemIdsArgs::class.java).itemIds?.toList()
        runCatching {
            ItemProgress.list(activity.applicationContext, itemIds)
        }.onSuccess { entries ->
            val payload = JSObject()
            payload.put("entries", JSArray().apply { entries.forEach { put(it.toJson()) } })
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "getItemProgress failed")
        }
    }

    @Command
    fun setItemProgress(invoke: Invoke) {
        val args = invoke.getArgs()
        val entries = args.optJSONArray("entries")
        if (entries == null) {
            invoke.reject("entries is required")
            return
        }
        runCatching {
            NativeAudioRuntime.setItemProgress(activity.applicationContext, ItemProgress.parse(entries), args.optBoolean("merge", false))
        }.onSuccess { result ->
            val payload = JSObject()
            payload.put("entries", JSArray().apply { result.forEach { put(it.toJson()) } })
            invoke.resolve(payload)
        }.onFailure {
            invoke.reject(it.message ?: "setItemProgress failed")
        }
    }

    @Command
    fun setSleepTimer(invoke: Invoke) {
        val args = invoke.parseArgs(SetSleepTimerArgs::class.java)
        val endOfTrack = args.endOfTrack ?: false
        val minutes = args.minutes
        if (!endOfTrack && (minutes == null || !minutes.isFinite() || minutes <= 0)) {
            invoke.reject("minutes must be > 0, or endOfTrack true")
            return
        }
        val fadeOutSeconds = args.fadeOutSeconds ?: DEFAULT_SLEEP_FADE_OUT_SECONDS
        if (!fadeOutSeconds.isFinite() || fadeOutSeconds < 0) {
            invoke.reject("fadeOutSeconds must be >= 0")
            return
        }

        runCatching {
            NativeAudioRuntime.setSleepTimer(activity.applicationContext, minutes, endOfTrack, fadeOutSeconds)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setSleepTimer failed")
        }
    }

    @Command
    fun cancelSleepTimer(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.cancelSleepTimer(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "cancelSleepTimer failed")
        }
    }

    @Command
    fun setSkipInterval(invoke: Invoke) {
        val seconds = invoke.parseArgs(SetSkipIntervalArgs::class.java).seconds
        if (seconds == null || !seconds.isFinite() || seconds < 0) {
            invoke.reject("seconds must be >= 0")
            return
        }

        runCatching {
            NativeAudioRuntime.setSkipInterval(activity.applicationContext, seconds)
        }.onSuccess {
            invoke.resolve(toJsObject(NativeAudioRuntime.getState(activity.applicationContext)))
        }.onFailure {
            invoke.reject(it.message ?: "setSkipInterval failed")
        }
    }

    @Command
    fun getState(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.getState(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(toJsObject(it))
        }.onFailure {
            invoke.reject(it.message ?: "getState failed")
        }
    }

    @Command
    fun getProgressCheckpoint(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.getProgressCheckpoint(activity.applicationContext)
        }.onSuccess {
            invoke.resolve(it?.let { checkpoint -> toJsObject(checkpoint) })
        }.onFailure {
            invoke.reject(it.message ?: "getProgressCheckpoint failed")
        }
    }

    @Command
    fun clearProgressCheckpoint(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.clearProgressCheckpoint(activity.applicationContext)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "clearProgressCheckpoint failed")
        }
    }

    @Command
    fun dispose(invoke: Invoke) {
        runCatching {
            NativeAudioRuntime.dispose(activity.applicationContext)
        }.onSuccess {
            invoke.resolve()
        }.onFailure {
            invoke.reject(it.message ?: "dispose failed")
        }
    }

    override fun onDestroy() {
        if (activeInstance === this) activeInstance = null
        super.onDestroy()
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            NOTIFICATION_PERMISSION_REQUEST_CODE,
        )
    }

    private fun emitControlPress(press: PlaybackControls.Press) {
        val payload = JSObject(press.toJson().toString())
        activity.runOnUiThread {
            trigger(EVENT_CONTROL, payload)
        }
    }

    private fun emitTrackedListChange(change: JSONObject) {
        val payload = JSObject(change.toString())
        activity.runOnUiThread {
            trigger(EVENT_TRACKED_LIST, payload)
        }
    }

    private fun emitPlaybackEvent(event: JSONObject) {
        val payload = JSObject(event.toString())
        activity.runOnUiThread {
            trigger(EVENT_PLAYBACK, payload)
        }
    }

    private fun emitState(state: NativeAudioState) {
        val payload = toJsObject(state)
        activity.runOnUiThread {
            trigger(EVENT_STATE, payload)
        }
    }

    private fun toJsObject(state: NativeAudioState): JSObject {
        val payload = JSObject()
        payload.put("status", state.status)
        payload.put("currentTime", state.currentTime)
        payload.put("duration", state.duration)
        payload.put("isPlaying", state.isPlaying)
        payload.put("buffering", state.buffering)
        payload.put("rate", state.rate)
        payload.put("queueIndex", state.queueIndex)
        payload.put("queueLength", state.queueLength)
        state.currentId?.let { payload.put("currentId", it) }
        payload.put("shuffle", state.shuffle)
        payload.put("repeatMode", state.repeatMode)
        payload.put("sleepTimerEndsAtMs", state.sleepTimerEndsAtMs ?: JSONObject.NULL)
        payload.put("sleepTimerEndOfTrack", state.sleepTimerEndOfTrack)
        if (!state.error.isNullOrBlank()) payload.put("error", state.error)
        payload.put("volume", state.volume)
        return payload
    }

    /** Same shape as a setQueue item. */
    private fun queueItemToJsObject(item: MediaItem): JSObject {
        val payload = JSObject()
        payload.put("src", item.localConfiguration?.uri?.toString())
        item.mediaId.toLongOrNull()?.let { payload.put("id", it) }
        item.mediaMetadata.title?.let { payload.put("title", it.toString()) }
        item.mediaMetadata.artist?.let { payload.put("artist", it.toString()) }
        NativeAudioRuntime.artworkUrlOf(item)?.let { payload.put("artworkUrl", it) }
        return payload
    }

    private fun toJsObject(checkpoint: NativeAudioProgressCheckpoint): JSObject {
        val payload = JSObject()
        payload.put("id", checkpoint.id)
        payload.put("currentTime", checkpoint.currentTime)
        payload.put("updatedAtMs", checkpoint.updatedAtMs)
        if (!checkpoint.status.isNullOrBlank()) payload.put("status", checkpoint.status)
        return payload
    }

    companion object {
        @Volatile
        private var activeInstance: NativeAudioPlugin? = null

        internal fun emitToActive(state: NativeAudioState) {
            activeInstance?.emitState(state)
        }

        /** Live delivery while the app runs; presses are logged either way (see PlaybackControls). */
        internal fun emitControlPressToActive(press: PlaybackControls.Press) {
            activeInstance?.emitControlPress(press)
        }

        internal fun emitPlaybackEventToActive(event: JSONObject) {
            activeInstance?.emitPlaybackEvent(event)
        }

        internal fun emitTrackedListChangeToActive(change: JSONObject) {
            activeInstance?.emitTrackedListChange(change)
        }
    }
}
