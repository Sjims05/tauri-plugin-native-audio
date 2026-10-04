package app.tauri.nativeaudio

import androidx.media3.common.MediaItem
import app.tauri.annotation.InvokeArg

// Shared by the runtime and the plugin: constants, the state and queue shapes, and the commands' arguments.

internal const val EVENT_STATE = "native_audio_state"
internal const val EVENT_CONTROL = "native_audio_control"
internal const val EVENT_PLAYBACK = "native_audio_playback_event"
internal const val EVENT_TRACKED_LIST = "native_audio_tracked_list"
internal const val ADDED_TO_QUEUE_EXTRA = "app.tauri.nativeaudio.ADDED_TO_QUEUE"
internal const val ARTWORK_URL_EXTRA = "app.tauri.nativeaudio.ARTWORK_URL"
internal const val CAR_APP_META_DATA = "com.google.android.gms.car.application"
internal const val NOTIFICATION_PERMISSION_REQUEST_CODE = 9512
internal const val FOREGROUND_PROGRESS_TICK_MS = 100L
internal const val BACKGROUND_PROGRESS_TICK_MS = 250L
internal const val SEEK_STATE_STALE_MS = 1_500L
internal const val PROGRESS_PERSIST_THROTTLE_MS = 1_000L
internal const val QUEUE_POSITION_SAVE_THROTTLE_MS = 1_000L
internal const val AUDIO_BUFFER_MIN_US = 1_500_000
internal const val AUDIO_BUFFER_MAX_US = 2_000_000
internal const val DEFAULT_SLEEP_FADE_OUT_SECONDS = 10.0
internal const val SLEEP_FADE_STEP_MS = 100L
internal const val ITEM_PROGRESS_SAVE_INTERVAL_MS = 15_000L
internal const val PROGRESS_NEAR_START_EPSILON_SEC = 0.25
internal const val PROGRESS_PERSIST_EPSILON_SEC = 0.05
internal const val PROGRESS_PREFS_NAME = "tauri_native_audio_progress"
internal const val PROGRESS_KEY_STORY_ID = "story_id"
internal const val PROGRESS_KEY_CURRENT_TIME = "current_time"
internal const val PROGRESS_KEY_UPDATED_AT_MS = "updated_at_ms"
internal const val PROGRESS_KEY_STATUS = "status"

data class NativeAudioState(
    val status: String,
    val currentTime: Double,
    val duration: Double,
    val isPlaying: Boolean,
    val buffering: Boolean,
    val rate: Double,
    val queueIndex: Int,
    val queueLength: Int,
    val currentId: Long? = null,
    val shuffle: Boolean,
    val repeatMode: String,
    /** When a running sleep timer pauses playback (epoch ms), or null. */
    val sleepTimerEndsAtMs: Long? = null,
    /** A sleep timer that pauses at the end of the current track. */
    val sleepTimerEndOfTrack: Boolean = false,
    val error: String? = null,
    /** The player's own volume, 0 to 1 (on top of the system volume). */
    val volume: Double = 1.0,
)

data class NativeAudioProgressCheckpoint(
    val id: Long,
    val currentTime: Double,
    val updatedAtMs: Long,
    val status: String? = null,
)

@InvokeArg
class SetSourceArgs {
    var src: String? = null
    var id: Long? = null
    var title: String? = null
    var artist: String? = null
    var artworkUrl: String? = null
}

@InvokeArg
class SetQueueArgs {
    var items: Array<SetSourceArgs>? = null
    var startIndex: Int? = null
    var startPosition: Double? = null
    var sourceId: String? = null
}

@InvokeArg
class SkipToArgs {
    var index: Int? = null
}

@InvokeArg
class AddToQueueArgs {
    var items: Array<SetSourceArgs>? = null
    var playNext: Boolean? = null
}

@InvokeArg
class UpdateQueueArgs {
    var items: Array<SetSourceArgs>? = null
    var sourceId: String? = null
    /** "finish" (default) or "skip". */
    var removedCurrent: String? = null
    /** "remove" (default) or "keep". */
    var removedItems: String? = null
    /** "inPlace" (default), "end" or "next". */
    var newItems: String? = null
    /** "playlist" (default) or "queue". */
    var order: String? = null
}

@InvokeArg
class SetOptionsArgs {
    /** true / false, or "off" / "paused" / "play". */
    var resumeLastQueue: Any? = null
    var repeatAddedTracks: Boolean? = null
    var pausedKeepAliveMinutes: Double? = null
    var keepAliveWhileCarConnected: Boolean? = null
    var trackProgress: Boolean? = null
    var keepQueueOnStop: Boolean? = null
    var volumeCurve: String? = null
    var previousRestartsAfterSeconds: Double? = null
    var nextAtEnd: String? = null
}

@InvokeArg
class SetControlActiveArgs {
    var buttonId: String? = null
    var itemIds: Array<Long>? = null
    var active: Boolean? = null
}

@InvokeArg
class AcknowledgeControlPressesArgs {
    var ids: Array<String>? = null
}

@InvokeArg
class AcknowledgeIdsArgs {
    var ids: Array<String>? = null
}

@InvokeArg
class TrackedListArgs {
    var id: String? = null
}

@InvokeArg
class MoveInQueueArgs {
    var from: Int? = null
    var to: Int? = null
}

/** The queue as getQueue reports it. */
class NativeAudioQueue(
    val items: List<MediaItem>,
    val currentIndex: Int,
    /** Queue indices in the order they play (the shuffle order when shuffle is on). */
    val playOrder: IntArray,
)

@InvokeArg
class SetShuffleArgs {
    var enabled: Boolean? = null
}

@InvokeArg
class SetRepeatModeArgs {
    var mode: String? = null
}

@InvokeArg
class ItemIdsArgs {
    var itemIds: Array<Long>? = null
}

@InvokeArg
class SetSleepTimerArgs {
    var minutes: Double? = null
    var endOfTrack: Boolean? = null
    var fadeOutSeconds: Double? = null
}

@InvokeArg
class SetSkipIntervalArgs {
    var seconds: Double? = null
}

@InvokeArg
class SeekToArgs {
    var position: Double? = null
}

@InvokeArg
class SetRateArgs {
    var rate: Double? = null
}

@InvokeArg
class SetVolumeArgs {
    var volume: Double? = null
}
