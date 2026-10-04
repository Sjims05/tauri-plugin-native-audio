package app.tauri.nativeaudio

import android.app.PendingIntent
import android.content.Context
import android.content.SharedPreferences
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.app.ActivityManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.DefaultAudioTrackBufferSizeProvider
import androidx.media3.exoplayer.source.ShuffleOrder.DefaultShuffleOrder
import androidx.car.app.connection.CarConnection
import androidx.lifecycle.LiveData
import androidx.lifecycle.Observer
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.max
import kotlin.random.Random

private const val TAG = "plugin/native-audio"

// The player itself (ExoPlayer, the media session, the queue, sleep timer, saved state), shared by the
// plugin's commands, the audio service and Android Auto. One per process.

/**
 * A larger audio output buffer than ExoPlayer's default (0.25 to 0.75 s). When the screen turns on
 * or off, some phones stall the app's audio for close to a second (measured: ~0.8 s of underrun
 * after screen on), which emptied the default buffer and was heard as a stutter. Pause, seek and
 * volume still act immediately; a speed change is heard once the buffered audio has played.
 */
private class LargeBufferRenderersFactory(context: Context) : DefaultRenderersFactory(context) {
    override fun buildAudioSink(
        context: Context,
        enableFloatOutput: Boolean,
        enableAudioTrackPlaybackParams: Boolean,
    ): AudioSink = DefaultAudioSink.Builder(context)
        .setEnableFloatOutput(enableFloatOutput)
        .setEnableAudioTrackPlaybackParams(enableAudioTrackPlaybackParams)
        .setAudioTrackBufferSizeProvider(
            DefaultAudioTrackBufferSizeProvider.Builder()
                .setMinPcmBufferDurationUs(AUDIO_BUFFER_MIN_US)
                .setMaxPcmBufferDurationUs(AUDIO_BUFFER_MAX_US)
                .build(),
        )
        .build()
}

private data class PendingSeekState(
    val shouldResume: Boolean,
    val startedAtMs: Long,
)

object NativeAudioRuntime {
    private val lock = Any()
    private val tickHandler = Handler(Looper.getMainLooper())
    private var tickScheduled = false

    private var player: ExoPlayer? = null
    private var appContext: Context? = null
    private var mediaSession: MediaLibrarySession? = null
    private var sessionPlayer: SkipIntervalPlayer? = null
    // Kept across dispose() so they only have to be set once.
    @Volatile
    private var skipIntervalMs = 0L
    private var shuffleEnabled = false
    /** setVolume (a slider's position), saved in PluginSettings; loaded in ensure. */
    private var volume = 1f
    /** The volume curve (setOptions volumeCurve) as a power: what's heard is volume^volumeExponent. */
    private var volumeExponent = 2
    /** A sleep timer is fading out: it sets the player volume meanwhile. */
    private var sleepFading = false

    private fun volumeExponentOf(curve: String) = when (curve) {
        "linear" -> 1
        "cubic" -> 3
        else -> 2
    }

    /** The player volume for the set volume. */
    private fun gainLocked(): Float = Math.pow(volume.toDouble(), volumeExponent.toDouble()).toFloat()
    private var repeatMode = Player.REPEAT_MODE_OFF
    // The shuffle order handed to ExoPlayer, kept to detect the wrap to a new pass.
    private var shuffleIndices = IntArray(0)
    /** The item that left the playlist (updateQueue) while it played: removed once playback moves on. */
    private var pendingRemoval: MediaItem? = null
    private var lastMediaItemIndex = C.INDEX_UNSET
    private var lastQueuePositionSavedAtMs = 0L
    // While a saved queue is being restored: don't save half-restored state, and don't let
    // "shuffle turned on" build a fresh order over the saved one.
    private var restoringQueue = false
    // A saved queue handed to the media session by onPlaybackResumption, applied once it's set.
    private var pendingResumption: QueueSnapshotStore.Snapshot? = null
    // Whether the current track's `start` playback event was logged (it's logged once it plays).
    private var currentStartLogged = false
    // The playable folder (playlist, album) the queue was started from, for "folder" tracked lists.
    private var queueSourceId: String? = null
    // Set by the library callback when Android Auto starts a folder; applied when the queue is set.
    private var pendingQueueSourceId: String? = null
    // Tracked lists the current play of the current track has already counted towards.
    private val countedLists = HashSet<String>()
    // Sleep timer: a time (with a fade-out before it) or the end of the current track.
    private var sleepTimerEndsAtMs: Long? = null
    private var sleepTimerMinutes: Double? = null
    private var sleepTimerEndOfTrack = false
    private var sleepFadeOutMs = 0L
    private val sleepTimerRunnable = Runnable { onSleepTimerReached() }
    // Item progress (trackProgress): last save while playing, and the last known track duration
    // (the player only knows the new track's once it moved on).
    private var lastItemProgressSavedAtMs = 0L
    private var knownDurationItemId: Long? = null
    private var knownDurationMs = 0L
    private var lastError: String? = null
    private var pendingSeekState: PendingSeekState? = null
    private var lastProgressPersistedAtMs = 0L
    private var lastProgressPersistedStoryId: Long? = null
    private var lastProgressPersistedTimeSec: Double? = null

    private val tickRunnable = object : Runnable {
        override fun run() {
            val shouldContinue = synchronized(lock) {
                val snapshot = snapshotLocked()
                appContext?.let { persistProgressCheckpointLocked(it, snapshot, force = false) }
                saveQueuePositionLocked(force = false)
                player?.let {
                    countPlayLocked(it.currentMediaItem, it.currentPosition, completed = false)
                    rememberDurationLocked(it)
                    val now = System.currentTimeMillis()
                    if (now - lastItemProgressSavedAtMs >= ITEM_PROGRESS_SAVE_INTERVAL_MS) {
                        lastItemProgressSavedAtMs = now
                        recordItemProgressLocked(it.currentMediaItem, it.currentPosition, it.duration, completed = false, notify = false)
                    }
                }
                NativeAudioPlugin.emitToActive(snapshot)
                val isPlaying = player?.isPlaying == true
                tickScheduled = isPlaying
                isPlaying
            }
            if (shouldContinue) {
                val delay = synchronized(lock) { nextProgressTickDelayLocked() }
                tickHandler.postDelayed(this, delay)
            }
        }
    }

    private val playerListener = object : Player.Listener {
        override fun onPlaybackStateChanged(playbackState: Int) {
            if (playbackState == Player.STATE_ENDED) {
                synchronized(lock) {
                    appContext?.let { persistProgressCheckpointLocked(it, snapshotLocked(), force = true) }
                    // The last track of the queue played to the end.
                    player?.let {
                        logPlaybackEventLocked("complete", it.currentMediaItem, it.duration)
                        countPlayLocked(it.currentMediaItem, it.duration, completed = true)
                        recordItemProgressLocked(it.currentMediaItem, it.duration, it.duration, completed = true, notify = true)
                    }
                }
            }
            syncTicking()
            emitState()
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            // Playback can also start without play() being called, e.g. from Android Auto while the
            // app is in the background. The service has to run for the notification and to keep
            // playing after Auto disconnects.
            if (isPlaying) appContext?.let { startService(it) }
            synchronized(lock) {
                if (isPlaying) {
                    logStartIfNeededLocked()
                } else {
                    saveQueuePositionLocked(force = true)
                    player?.let {
                        rememberDurationLocked(it)
                        recordItemProgressLocked(it.currentMediaItem, it.currentPosition, it.duration, completed = false, notify = true)
                    }
                }
            }
            syncTicking()
            emitState()
        }

        override fun onTimelineChanged(timeline: androidx.media3.common.Timeline, reason: Int) {
            // Items added, removed or moved, a new queue, or a new shuffle order.
            if (reason == Player.TIMELINE_CHANGE_REASON_PLAYLIST_CHANGED) synchronized(lock) { saveQueueLocked() }
        }

        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            synchronized(lock) {
                pendingSeekState = null
                player?.let { handleWrapLocked(it, reason) }
                player?.let { dropPendingRemovalLocked(it) }
                saveQueuePositionLocked(force = true)
            }
            refreshControls()
            syncTicking()
            emitState()
        }

        override fun onShuffleModeEnabledChanged(shuffleModeEnabled: Boolean) {
            synchronized(lock) {
                shuffleEnabled = shuffleModeEnabled
                // Every time shuffle is turned on: a fresh order that starts with the current track.
                val exoPlayer = player
                if (shuffleModeEnabled && exoPlayer != null && !restoringQueue) {
                    applyShuffleOrderLocked(exoPlayer, exoPlayer.currentMediaItemIndex)
                }
                saveQueueLocked()
            }
            refreshControls()
            emitState()
        }

        override fun onRepeatModeChanged(repeatMode: Int) {
            synchronized(lock) {
                this@NativeAudioRuntime.repeatMode = repeatMode
                saveQueueLocked()
            }
            refreshControls()
            emitState()
        }

        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            // The end-of-track sleep timer did its job: playback continues normally next time.
            if (reason == Player.PLAY_WHEN_READY_CHANGE_REASON_END_OF_MEDIA_ITEM) {
                synchronized(lock) {
                    if (sleepTimerEndOfTrack) clearSleepTimerLocked()
                }
                refreshControls()
                emitState()
            }
        }

        override fun onPlaybackParametersChanged(playbackParameters: androidx.media3.common.PlaybackParameters) {
            refreshControls()
            emitState()
        }

        override fun onPositionDiscontinuity(
            oldPosition: Player.PositionInfo,
            newPosition: Player.PositionInfo,
            reason: Int,
        ) {
            synchronized(lock) { logTrackLeftLocked(oldPosition, newPosition, reason) }
            if (reason == Player.DISCONTINUITY_REASON_SEEK || reason == Player.DISCONTINUITY_REASON_SEEK_ADJUSTMENT) {
                synchronized(lock) {
                    val exoPlayer = player ?: return@synchronized
                    val pendingSeek = pendingSeekState
                    val shouldResume = pendingSeek?.shouldResume ?: exoPlayer.playWhenReady
                    if (!shouldResume && exoPlayer.playWhenReady) exoPlayer.pause()
                    val shouldRecoverPlayback =
                        shouldResume &&
                            !exoPlayer.isPlaying &&
                            exoPlayer.playbackState == Player.STATE_READY &&
                            lastError == null
                    if (shouldRecoverPlayback) exoPlayer.play()
                    appContext?.let { persistProgressCheckpointLocked(it, snapshotLocked(), force = true) }
                    saveQueuePositionLocked(force = true)
                }
            }
            syncTicking()
            emitState()
        }

        override fun onPlayerError(error: PlaybackException) {
            Log.e(TAG, "onPlayerError code=${error.errorCodeName} message=${error.message}", error)
            synchronized(lock) {
                lastError = error.message ?: "unknown"
                pendingSeekState = null
            }
            syncTicking()
            emitState()
        }
    }

    fun ensure(context: Context) {
        synchronized(lock) {
            if (player != null && mediaSession != null) return

            val ctx = context.applicationContext
            appContext = ctx
            skipIntervalMs = PluginSettings.skipIntervalMs(ctx)
            volumeExponent = volumeExponentOf(PluginSettings.volumeCurve(ctx))
            volume = PluginSettings.volume(ctx)

            val audioAttributes = AudioAttributes.Builder()
                .setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                .build()

            val exoPlayer = ExoPlayer.Builder(ctx, LargeBufferRenderersFactory(ctx)).build()
            exoPlayer.setAudioAttributes(audioAttributes, true)
            exoPlayer.setHandleAudioBecomingNoisy(true)
            exoPlayer.setWakeMode(C.WAKE_MODE_LOCAL)
            exoPlayer.repeatMode = repeatMode
            exoPlayer.shuffleModeEnabled = shuffleEnabled
            exoPlayer.volume = gainLocked()
            exoPlayer.addListener(playerListener)
            player = exoPlayer
            val launchIntent = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)
            val pendingIntent = launchIntent?.let {
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                    (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) PendingIntent.FLAG_IMMUTABLE else 0)
                PendingIntent.getActivity(ctx, 0, it, flags)
            }

            // Notification / headset / lock screen previous & next buttons go through this player,
            // which either moves through the queue or seeks by the skip interval.
            val skipPlayer = SkipIntervalPlayer(
                exoPlayer,
                skipIntervalMs = { skipIntervalMs },
                onPlaylistReplaced = ::onSessionPlaylistReplaced,
                keepQueueOnStop = { PluginSettings.keepQueueOnStop(ctx) },
                previousRestartAfterMs = { PluginSettings.previousRestartAfterMs(ctx) },
                nextAtEndFirst = { PluginSettings.nextAtEndFirst(ctx) },
            )
            sessionPlayer = skipPlayer
            // A library session, so Android Auto can browse the setLibrary tree (when the app enables
            // carSupport, which exports the service). Without carSupport it acts as a plain session.
            mediaSession = MediaLibrarySession.Builder(ctx, skipPlayer, LibrarySessionCallback(ctx))
                .setCustomLayout(PlaybackControls.layout(ctx, playerViewLocked(exoPlayer)))
                // Media3 otherwise re-sends the playback state every 3 seconds while playing, and Android
                // Auto redraws its queue each time, scrolling it back to the playing track. Controllers
                // move the progress bar themselves (the state has the position, speed and its time).
                .setPeriodicPositionUpdateEnabled(false)
                .apply {
                    if (pendingIntent != null) setSessionActivity(pendingIntent)
                }
                .build()

            lastError = null
            syncTickingLocked()
        }
    }

    fun initialize(context: Context) {
        ensure(context)
        emitState()
    }

    fun startService(context: Context) {
        val serviceIntent = Intent(context.applicationContext, NativeAudioService::class.java)
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.applicationContext.startForegroundService(serviceIntent)
            } else {
                context.applicationContext.startService(serviceIntent)
            }
        }.onFailure { error ->
            Log.w(TAG, "startService failed", error)
        }
    }

    fun stopService(context: Context) {
        val serviceIntent = Intent(context.applicationContext, NativeAudioService::class.java)
        context.applicationContext.stopService(serviceIntent)
    }

    fun setSource(context: Context, src: String, storyId: Long?, title: String?, artist: String?, artworkUrl: String?) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return

            val mediaItem = buildMediaItem(src, storyId, title, artist, artworkUrl)

            pendingSeekState = null
            queueSourceId = null
            lastMediaItemIndex = 0
            exoPlayer.setMediaItem(mediaItem)
            if (exoPlayer.shuffleModeEnabled) applyShuffleOrderLocked(exoPlayer, 0)
            exoPlayer.prepare()
            lastError = null
            syncTickingLocked()
        }
        emitState()
    }

    /** [sourceId]: the playable folder (playlist, album) this queue plays, for "folder" tracked lists. */
    fun setQueue(context: Context, items: List<SetSourceArgs>, startIndex: Int, startPositionSec: Double, sourceId: String? = null) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return

            val mediaItems = items.map { buildMediaItem(it.src!!.trim(), it.id, it.title, it.artist, it.artworkUrl) }
            val safeIndex = startIndex.coerceIn(0, mediaItems.size - 1)
            val safeStartMs = if (startPositionSec.isFinite()) max(0L, (startPositionSec * 1000.0).toLong()) else 0L

            pendingSeekState = null
            queueSourceId = sourceId?.takeIf { it.isNotEmpty() }
            lastMediaItemIndex = safeIndex
            exoPlayer.setMediaItems(mediaItems, safeIndex, safeStartMs)
            // ExoPlayer gives a new playlist a random order; start ours with the start item instead.
            if (exoPlayer.shuffleModeEnabled) applyShuffleOrderLocked(exoPlayer, safeIndex)
            exoPlayer.prepare()
            lastError = null
            syncTickingLocked()
        }
        emitState()
    }

    fun next(context: Context) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return
            pendingSeekState = null
            if (!TrackSkip.next(exoPlayer, PluginSettings.nextAtEndFirst(context))) return@synchronized
            lastError = null
            exoPlayer.prepare()
        }
        emitState()
    }

    fun previous(context: Context) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return
            pendingSeekState = null
            lastError = null
            // Same rule as the notification button: restart the current track when it's past
            // previousRestartsAfterSeconds (default 3), otherwise go to the previous one.
            TrackSkip.previous(exoPlayer, PluginSettings.previousRestartAfterMs(context))
            exoPlayer.prepare()
        }
        emitState()
    }

    /** Returns false when [index] is outside the queue. */
    fun skipTo(context: Context, index: Int): Boolean {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return false
            if (index < 0 || index >= exoPlayer.mediaItemCount) return false
            pendingSeekState = null
            lastError = null
            // Picking a track while shuffled plays it, then shuffles the rest of the queue after it.
            if (exoPlayer.shuffleModeEnabled) applyShuffleOrderLocked(exoPlayer, index)
            lastMediaItemIndex = index
            exoPlayer.seekToDefaultPosition(index)
            exoPlayer.prepare()
        }
        emitState()
        return true
    }

    fun play(context: Context) {
        startService(context)
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return
            if (exoPlayer.playbackState == Player.STATE_ENDED) {
                exoPlayer.seekTo(0L)
            }
            pendingSeekState = null
            exoPlayer.playWhenReady = true
            exoPlayer.play()
            lastError = null
            syncTickingLocked()
        }
        emitState()
    }

    fun pause(context: Context) {
        synchronized(lock) {
            ensure(context)
            pendingSeekState = null
            player?.pause()
            syncTickingLocked()
            persistProgressCheckpointLocked(context.applicationContext, snapshotLocked(), force = true)
            saveQueuePositionLocked(force = true)
        }
        emitState()
    }

    fun seekTo(context: Context, positionSec: Double) {
        if (!positionSec.isFinite()) return
        synchronized(lock) {
            ensure(context)
            val safeMs = max(0L, (positionSec * 1000.0).toLong())
            val exoPlayer = player ?: return@synchronized
            val shouldResume = exoPlayer.playWhenReady || exoPlayer.isPlaying
            pendingSeekState = PendingSeekState(shouldResume = shouldResume, startedAtMs = System.currentTimeMillis())
            if (!shouldResume && exoPlayer.playWhenReady) exoPlayer.pause()
            exoPlayer.seekTo(safeMs)
        }
        emitState()
    }

    fun setVolume(context: Context, value: Double) {
        synchronized(lock) {
            ensure(context)
            volume = value.coerceIn(0.0, 1.0).toFloat()
            PluginSettings.setVolume(context, volume)
            // During a sleep timer's fade-out the fade sets it.
            if (!sleepFading) player?.volume = gainLocked()
        }
        emitState()
    }

    fun setRate(context: Context, rate: Double) {
        if (!rate.isFinite() || rate <= 0.0) return
        synchronized(lock) {
            ensure(context)
            player?.setPlaybackSpeed(rate.toFloat())
        }
        emitState()
    }

    /**
     * Adds [items] to the queue: right after the current track when [playNext], otherwise at the end.
     * With shuffle on they're placed at the same spot in the play order; the rest of the order stays.
     */
    fun addToQueue(context: Context, items: List<SetSourceArgs>, playNext: Boolean) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return
            val count = exoPlayer.mediaItemCount
            // Marked so they can be dropped when the queue repeats (repeatAddedTracks = false).
            // Adding to an empty queue just makes it the queue.
            val mediaItems = items.map {
                buildMediaItem(it.src!!.trim(), it.id, it.title, it.artist, it.artworkUrl, addedToQueue = count > 0)
            }
            if (count == 0) {
                lastMediaItemIndex = 0
                exoPlayer.setMediaItems(mediaItems)
                if (exoPlayer.shuffleModeEnabled) applyShuffleOrderLocked(exoPlayer, 0)
                exoPlayer.prepare()
                return@synchronized
            }

            val current = exoPlayer.currentMediaItemIndex
            val at = if (playNext) current + 1 else count
            val oldOrder = playOrderLocked(exoPlayer)
            exoPlayer.addMediaItems(at, mediaItems)
            if (exoPlayer.shuffleModeEnabled) {
                val playPosition = if (playNext) oldOrder.indexOf(current) + 1 else oldOrder.size
                setShuffleIndicesLocked(exoPlayer, QueueOrder.insert(oldOrder, at, mediaItems.size, playPosition))
            }
            lastMediaItemIndex = exoPlayer.currentMediaItemIndex
        }
        emitState()
    }

    /**
     * The playlist the queue plays changed: [items] is its new content (see QueueUpdate). The current
     * track keeps playing untouched: everything around it is replaced. With an empty queue, [items]
     * becomes the queue.
     */
    fun updateQueue(
        context: Context,
        items: List<SetSourceArgs>,
        sourceId: String?,
        skipRemovedCurrent: Boolean = false,
        keepRemoved: Boolean = false,
        newItems: String = "inPlace",
        keepQueueOrder: Boolean = false,
    ): Pair<Int, Int> {
        synchronized(lock) {
            ensure(context)
            if (sourceId != null) queueSourceId = sourceId
        }
        val empty = synchronized(lock) { player?.mediaItemCount == 0 }
        if (empty) {
            addToQueue(context, items, playNext = false)
            return items.size to 0
        }
        val changes = synchronized(lock) {
            val exoPlayer = player ?: return 0 to 0
            val count = exoPlayer.mediaItemCount
            val current = exoPlayer.currentMediaItemIndex
            val oldItems = (0 until count).map { exoPlayer.getMediaItemAt(it) }
            val plan = QueueUpdate.plan(
                oldIds = oldItems.map { it.mediaId.toLongOrNull() },
                oldAdded = oldItems.map { isAddedToQueue(it) },
                oldOrder = playOrderLocked(exoPlayer),
                current = current,
                newIds = items.map { it.id },
                shuffle = exoPlayer.shuffleModeEnabled,
                keepRemoved = keepRemoved,
                newItems = newItems,
                keepQueueOrder = keepQueueOrder,
            )
            fun build(item: Int) = items[item].let { buildMediaItem(it.src!!.trim(), it.id, it.title, it.artist, it.artworkUrl, addedToQueue = false) }
            val currentAt = plan.list.indexOfFirst {
                (it is QueueUpdate.Slot.Matched && it.old == current) || (it is QueueUpdate.Slot.Kept && it.old == current)
            }
            val mediaItems = plan.list.map {
                when (it) {
                    // The playing item itself stays, so playback isn't interrupted.
                    is QueueUpdate.Slot.Matched -> if (it.old == current) oldItems[current] else build(it.item)
                    is QueueUpdate.Slot.New -> build(it.item)
                    is QueueUpdate.Slot.Kept -> oldItems[it.old]
                }
            }
            restoringQueue = true
            try {
                exoPlayer.removeMediaItems(current + 1, count)
                exoPlayer.removeMediaItems(0, current)
                exoPlayer.addMediaItems(0, mediaItems.subList(0, currentAt))
                exoPlayer.addMediaItems(mediaItems.subList(currentAt + 1, mediaItems.size))
                if (exoPlayer.shuffleModeEnabled) setShuffleIndicesLocked(exoPlayer, plan.order)
                // The playing item's details (title, cover) may have changed: same file, so this
                // doesn't interrupt it.
                (plan.list[currentAt] as? QueueUpdate.Slot.Matched)?.let { matched ->
                    val updated = build(matched.item)
                    if (updated.localConfiguration?.uri == oldItems[current].localConfiguration?.uri &&
                        updated.mediaMetadata != oldItems[current].mediaMetadata
                    ) {
                        exoPlayer.replaceMediaItem(exoPlayer.currentMediaItemIndex, updated)
                    }
                }
            } finally {
                restoringQueue = false
            }
            pendingRemoval = if (plan.currentLeft) oldItems[current] else null
            lastMediaItemIndex = exoPlayer.currentMediaItemIndex
            if (plan.currentLeft && skipRemovedCurrent) {
                // Moving on drops it (dropPendingRemovalLocked); with nothing after it, it goes now.
                if (exoPlayer.hasNextMediaItem()) {
                    exoPlayer.seekToNextMediaItem()
                } else {
                    pendingRemoval = null
                    val oldOrder = playOrderLocked(exoPlayer)
                    val index = exoPlayer.currentMediaItemIndex
                    exoPlayer.removeMediaItem(index)
                    if (exoPlayer.shuffleModeEnabled && exoPlayer.mediaItemCount > 0) {
                        setShuffleIndicesLocked(exoPlayer, QueueOrder.remove(oldOrder, index))
                    }
                    lastMediaItemIndex = exoPlayer.currentMediaItemIndex
                }
            }
            saveQueueLocked()
            plan.added to plan.removed
        }
        emitState()
        return changes
    }

    /** Playback moved on from an item that left the playlist (updateQueue): remove it now. */
    private fun dropPendingRemovalLocked(exoPlayer: ExoPlayer) {
        val item = pendingRemoval ?: return
        val index = (0 until exoPlayer.mediaItemCount).firstOrNull { exoPlayer.getMediaItemAt(it) === item }
        if (index == null) {
            pendingRemoval = null
            return
        }
        if (index == exoPlayer.currentMediaItemIndex) return // repeat one: still playing
        pendingRemoval = null
        val oldOrder = playOrderLocked(exoPlayer)
        exoPlayer.removeMediaItem(index)
        if (exoPlayer.shuffleModeEnabled && exoPlayer.mediaItemCount > 0) {
            setShuffleIndicesLocked(exoPlayer, QueueOrder.remove(oldOrder, index))
        }
        lastMediaItemIndex = exoPlayer.currentMediaItemIndex
    }

    /** Returns false when [index] is outside the queue. Removing the current track moves on to the next. */
    fun removeFromQueue(context: Context, index: Int): Boolean {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return false
            if (index < 0 || index >= exoPlayer.mediaItemCount) return false
            val oldOrder = playOrderLocked(exoPlayer)
            exoPlayer.removeMediaItem(index)
            if (exoPlayer.shuffleModeEnabled && exoPlayer.mediaItemCount > 0) {
                setShuffleIndicesLocked(exoPlayer, QueueOrder.remove(oldOrder, index))
            }
            lastMediaItemIndex = exoPlayer.currentMediaItemIndex
        }
        emitState()
        return true
    }

    /**
     * Moves the queue entry at [from] to [to] (queue list indices). With shuffle on, the play order
     * stays the same; without shuffle the list order is the play order.
     */
    fun moveInQueue(context: Context, from: Int, to: Int): Boolean {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return false
            val count = exoPlayer.mediaItemCount
            if (from !in 0 until count || to !in 0 until count) return false
            if (from == to) return true
            val oldOrder = playOrderLocked(exoPlayer)
            exoPlayer.moveMediaItem(from, to)
            if (exoPlayer.shuffleModeEnabled) setShuffleIndicesLocked(exoPlayer, QueueOrder.move(oldOrder, from, to))
            lastMediaItemIndex = exoPlayer.currentMediaItemIndex
        }
        emitState()
        return true
    }

    fun getQueue(context: Context): NativeAudioQueue {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return NativeAudioQueue(emptyList(), -1, IntArray(0))
            val count = exoPlayer.mediaItemCount
            return NativeAudioQueue(
                items = (0 until count).map { exoPlayer.getMediaItemAt(it) },
                currentIndex = if (count > 0) exoPlayer.currentMediaItemIndex else -1,
                playOrder = playOrderLocked(exoPlayer),
                sourceId = queueSourceId,
            )
        }
    }

    /**
     * Loads the last saved queue (items, track, position, shuffle order, repeat mode), paused.
     * Returns false when nothing was saved. Leaves an already loaded queue alone (returns true):
     * reloading it would interrupt playback, for example from Android Auto while the app's page reloads.
     */
    fun restoreLastQueue(context: Context): Boolean {
        val snapshot = QueueSnapshotStore.load(context.applicationContext) ?: return false
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return false
            if (exoPlayer.mediaItemCount > 0) return true
            pendingSeekState = null
            lastError = null
            restoringQueue = true
            try {
                queueSourceId = snapshot.sourceId
                exoPlayer.setMediaItems(snapshot.items, snapshot.index, snapshot.positionMs)
                applySnapshotModesLocked(exoPlayer, snapshot)
            } finally {
                restoringQueue = false
            }
            saveQueueLocked()
            exoPlayer.prepare()
            syncTickingLocked()
        }
        emitState()
        return true
    }

    /**
     * When the audio service starts with nothing loaded (Android Auto connecting, a headset play
     * button with the app closed), load the last queue so it's ready to continue, and with
     * resumeLastQueue "play" also play it.
     */
    fun restoreLastQueueIfIdle(context: Context) {
        val mode = PluginSettings.resumeMode(context)
        if (mode == "off") return
        val idle = synchronized(lock) {
            ensure(context)
            player?.mediaItemCount == 0
        }
        if (idle && restoreLastQueue(context) && mode == "play") play(context)
    }

    /** For onPlaybackResumption: the saved queue, whose modes are applied once the session sets it. */
    internal fun prepareResumption(context: Context): QueueSnapshotStore.Snapshot? {
        if (!PluginSettings.resumeLastQueue(context)) return null
        val snapshot = QueueSnapshotStore.load(context.applicationContext) ?: return null
        synchronized(lock) {
            pendingResumption = snapshot
        }
        return snapshot
    }

    fun setShuffle(context: Context, enabled: Boolean) {
        synchronized(lock) {
            ensure(context)
            shuffleEnabled = enabled
            // The order itself is built in onShuffleModeEnabledChanged.
            player?.shuffleModeEnabled = enabled
        }
        emitState()
    }

    fun setRepeatMode(context: Context, mode: Int) {
        synchronized(lock) {
            ensure(context)
            repeatMode = mode
            player?.repeatMode = mode
        }
        emitState()
    }

    fun setSkipInterval(context: Context, seconds: Double) {
        if (!seconds.isFinite() || seconds < 0.0) return
        val intervalMs = (seconds * 1000.0).toLong()
        PluginSettings.setSkipIntervalMs(context, intervalMs)
        val skipPlayer = synchronized(lock) {
            skipIntervalMs = intervalMs
            sessionPlayer
        } ?: return
        tickHandler.post { skipPlayer.notifyAvailableCommandsChanged() }
    }

    /**
     * Pauses playback after [minutes] (fading out over the last [fadeOutSeconds]), or at the end of
     * the current track with [endOfTrack]. Replaces a running timer.
     */
    fun setSleepTimer(context: Context, minutes: Double?, endOfTrack: Boolean, fadeOutSeconds: Double) {
        synchronized(lock) {
            ensure(context)
            val exoPlayer = player ?: return
            clearSleepTimerLocked()
            if (endOfTrack) {
                sleepTimerEndOfTrack = true
                exoPlayer.pauseAtEndOfMediaItems = true
            } else if (minutes != null) {
                val durationMs = (minutes * 60_000.0).toLong().coerceAtLeast(0L)
                sleepFadeOutMs = (fadeOutSeconds * 1000.0).toLong().coerceIn(0L, durationMs)
                sleepTimerMinutes = minutes
                sleepTimerEndsAtMs = System.currentTimeMillis() + durationMs
                tickHandler.postDelayed(sleepTimerRunnable, durationMs - sleepFadeOutMs)
            }
        }
        refreshControls()
        emitState()
    }

    fun cancelSleepTimer(context: Context) {
        synchronized(lock) {
            ensure(context)
            clearSleepTimerLocked()
        }
        refreshControls()
        emitState()
    }

    /** Stops a running timer or fade, and puts the volume back. */
    private fun clearSleepTimerLocked() {
        tickHandler.removeCallbacks(sleepTimerRunnable)
        player?.let {
            it.pauseAtEndOfMediaItems = false
            it.volume = gainLocked()
        }
        sleepFading = false
        sleepTimerEndsAtMs = null
        sleepTimerMinutes = null
        sleepTimerEndOfTrack = false
    }

    /** Fades out over sleepFadeOutMs, then pauses and puts the volume back. Main thread. */
    private fun onSleepTimerReached() {
        val steps = (sleepFadeOutMs / SLEEP_FADE_STEP_MS).toInt()
        fun fade(step: Int) {
            synchronized(lock) {
                val exoPlayer = player ?: return
                if (sleepTimerEndsAtMs == null) return // cancelled meanwhile
                if (step < steps) {
                    sleepFading = true
                    exoPlayer.volume = gainLocked() * (1f - (step + 1).toFloat() / (steps + 1))
                    tickHandler.postDelayed({ fade(step + 1) }, SLEEP_FADE_STEP_MS)
                    return
                }
                exoPlayer.pause()
                clearSleepTimerLocked()
            }
            refreshControls()
            emitState()
        }
        fade(0)
    }

    fun setOptions(
        context: Context,
        resumeLastQueue: Any?,
        repeatAddedTracks: Boolean?,
        pausedKeepAliveMinutes: Double?,
        keepAliveWhileCarConnected: Boolean?,
        trackProgress: Boolean?,
        keepQueueOnStop: Boolean?,
        volumeCurve: String?,
        previousRestartsAfterSeconds: Double?,
        nextAtEnd: String?,
    ) {
        require(pausedKeepAliveMinutes == null || (pausedKeepAliveMinutes.isFinite() && pausedKeepAliveMinutes >= 0)) {
            "pausedKeepAliveMinutes must be >= 0"
        }
        PluginSettings.setOptions(
            context, resumeLastQueue, repeatAddedTracks, pausedKeepAliveMinutes, keepAliveWhileCarConnected, trackProgress,
            keepQueueOnStop, volumeCurve, previousRestartsAfterSeconds, nextAtEnd,
        )
        // The next button may be available on the last track now (nextAtEnd).
        if (nextAtEnd != null) synchronized(lock) { sessionPlayer }?.let { tickHandler.post { it.notifyAvailableCommandsChanged() } }
        if (volumeCurve != null) {
            synchronized(lock) {
                volumeExponent = volumeExponentOf(volumeCurve)
                if (!sleepFading) player?.volume = gainLocked()
            }
        }
        // The service re-applies its pause rules with the new values.
        tickHandler.post { onKeepAliveRulesChanged?.invoke() }
    }

    /** Whether the phone is projecting to Android Auto (watched only with carSupport). */
    @Volatile
    var carConnected = false
        private set
    private var carConnectionType: LiveData<Int>? = null
    private val carConnectionObserver = Observer<Int> { type ->
        carConnected = type == CarConnection.CONNECTION_TYPE_PROJECTION
        onKeepAliveRulesChanged?.invoke()
    }

    /** Set by the service: re-applies what happens to the controls while paused. Main thread. */
    internal var onKeepAliveRulesChanged: (() -> Unit)? = null

    /**
     * Starts following whether Android Auto is connected. Uses Android Auto's own connection state,
     * because media sessions can't tell a paused car connection from a gone one. Main thread.
     */
    internal fun startWatchingCarConnection(context: Context) {
        if (carConnectionType != null || !carSupportEnabled(context)) return
        val type = CarConnection(context.applicationContext).type
        carConnectionType = type
        type.observeForever(carConnectionObserver)
    }

    internal fun stopWatchingCarConnection() {
        carConnectionType?.removeObserver(carConnectionObserver)
        carConnectionType = null
        carConnected = false
    }

    fun setControls(context: Context, json: JSONObject) {
        PlaybackControls.set(context, json)
        ensure(context)
        refreshControls(updateCommands = true)
    }

    fun setControlActive(context: Context, buttonId: String, itemIds: List<Long?>, active: Boolean) {
        PlaybackControls.setActive(context, buttonId, itemIds, active)
        refreshControls()
    }

    /** A press of a setControls button, from Android Auto or the media controls. */
    fun onControlCommand(context: Context, action: String): Boolean {
        val button = PlaybackControls.find(context, action) ?: return false
        when (button) {
            is PlaybackControls.Button.SeekBack -> seekByInterval(-(button.seconds * 1000.0).toLong())
            is PlaybackControls.Button.SeekForward -> seekByInterval((button.seconds * 1000.0).toLong())
            PlaybackControls.Button.Shuffle -> setShuffle(context, !(synchronized(lock) { player?.shuffleModeEnabled } ?: false))
            PlaybackControls.Button.Repeat -> {
                val next = when (synchronized(lock) { player?.repeatMode }) {
                    Player.REPEAT_MODE_OFF -> Player.REPEAT_MODE_ALL
                    Player.REPEAT_MODE_ALL -> Player.REPEAT_MODE_ONE
                    else -> Player.REPEAT_MODE_OFF
                }
                setRepeatMode(context, next)
            }
            is PlaybackControls.Button.Speed -> {
                val current = synchronized(lock) { player?.playbackParameters?.speed?.toDouble() } ?: 1.0
                val closest = button.rates.indices.minByOrNull { kotlin.math.abs(button.rates[it] - current) } ?: 0
                setRate(context, button.rates[(closest + 1) % button.rates.size])
            }
            is PlaybackControls.Button.SleepTimer -> {
                // Cycle: off -> each option in order -> off.
                val current = synchronized(lock) {
                    when {
                        sleepTimerEndOfTrack -> button.options.indexOfFirst { it == null }
                        sleepTimerMinutes != null -> button.options.indexOfFirst { it == sleepTimerMinutes }
                        else -> -1
                    }
                }
                val next = if (current + 1 < button.options.size && !(current == -1 && sleepTimerActive())) current + 1 else -1
                if (next == -1) {
                    cancelSleepTimer(context)
                } else {
                    val option = button.options[next]
                    setSleepTimer(context, option, endOfTrack = option == null, fadeOutSeconds = button.fadeOutSeconds)
                }
            }
            is PlaybackControls.Button.Custom -> {
                val itemId = synchronized(lock) { player?.currentMediaItem?.mediaId?.toLongOrNull() }
                val press = PlaybackControls.recordPress(context, button, itemId) ?: return true
                NativeAudioPlugin.emitControlPressToActive(press)
                refreshControls()
            }
        }
        return true
    }

    /** Whether the app enabled carSupport (build.rs adds this meta-data to its manifest). */
    fun carSupportEnabled(context: Context): Boolean = runCatching {
        @Suppress("DEPRECATION")
        val info = context.packageManager.getApplicationInfo(context.packageName, PackageManager.GET_META_DATA)
        info.metaData?.containsKey(CAR_APP_META_DATA) == true
    }.getOrDefault(false)

    fun getState(context: Context): NativeAudioState {
        synchronized(lock) {
            ensure(context)
            return snapshotLocked()
        }
    }

    fun getProgressCheckpoint(context: Context): NativeAudioProgressCheckpoint? {
        val prefs = progressPrefs(context.applicationContext)
        val storyId = prefs.getLong(PROGRESS_KEY_STORY_ID, 0L)
        if (storyId <= 0L) return null
        val currentTime = prefs.getFloat(PROGRESS_KEY_CURRENT_TIME, 0f).toDouble()
        val updatedAtMs = prefs.getLong(PROGRESS_KEY_UPDATED_AT_MS, 0L)
        if (!currentTime.isFinite() || currentTime <= 0.0 || updatedAtMs <= 0L) return null
        val status = prefs.getString(PROGRESS_KEY_STATUS, null)
        return NativeAudioProgressCheckpoint(
            id = storyId,
            currentTime = currentTime,
            updatedAtMs = updatedAtMs,
            status = status,
        )
    }

    fun clearProgressCheckpoint(context: Context) {
        synchronized(lock) {
            progressPrefs(context.applicationContext).edit()
                .remove(PROGRESS_KEY_STORY_ID)
                .remove(PROGRESS_KEY_CURRENT_TIME)
                .remove(PROGRESS_KEY_UPDATED_AT_MS)
                .remove(PROGRESS_KEY_STATUS)
                .apply()
            lastProgressPersistedAtMs = 0L
            lastProgressPersistedStoryId = null
            lastProgressPersistedTimeSec = null
        }
    }

    fun dispose(context: Context) {
        synchronized(lock) {
            persistProgressCheckpointLocked(context.applicationContext, snapshotLocked(), force = true)
            saveQueuePositionLocked(force = true)
            tickHandler.removeCallbacks(tickRunnable)
            tickScheduled = false

            player?.removeListener(playerListener)
            player?.release()
            player = null

            mediaSession?.release()
            mediaSession = null
            sessionPlayer = null

            lastError = null
            pendingSeekState = null
            appContext = null
        }
        stopService(context)
        emitState()
    }

    fun setLibrary(context: Context, library: JSONObject) {
        val before = MediaLibraryStore.get(context).folders.keys
        MediaLibraryStore.set(context, library)
        // Search may have been turned on or off: update what connected controllers may do.
        refreshControls(updateCommands = true)
        // Let connected browsers (Android Auto) reload the top level and every folder, old and new: Auto keeps
        // folders it has opened, and would go on showing their old contents.
        val session = synchronized(lock) { mediaSession } ?: return
        val folders = before + MediaLibraryStore.get(context).folders.keys
        tickHandler.post {
            session.notifyChildrenChanged("root", Int.MAX_VALUE, null)
            folders.forEach { session.notifyChildrenChanged("folder:$it", Int.MAX_VALUE, null) }
        }
    }

    fun mediaSession(): MediaLibrarySession? {
        synchronized(lock) {
            return mediaSession
        }
    }

    /** The player the notification (PlayerNotificationManager) shows controls for. */
    fun notificationPlayer(): Player? {
        synchronized(lock) {
            return sessionPlayer ?: player
        }
    }

    private fun syncTicking() {
        synchronized(lock) {
            syncTickingLocked()
        }
    }

    private fun syncTickingLocked() {
        val isPlaying = player?.isPlaying == true
        if (isPlaying && !tickScheduled) {
            tickScheduled = true
            tickHandler.removeCallbacks(tickRunnable)
            tickHandler.post(tickRunnable)
            return
        }
        if (!isPlaying && tickScheduled) {
            tickScheduled = false
            tickHandler.removeCallbacks(tickRunnable)
        }
    }

    private fun nextProgressTickDelayLocked(): Long {
        val context = appContext ?: return BACKGROUND_PROGRESS_TICK_MS
        val isForeground = isAppInForeground()
        val isInteractive = isDeviceInteractive(context)
        return if (isForeground && isInteractive) FOREGROUND_PROGRESS_TICK_MS else BACKGROUND_PROGRESS_TICK_MS
    }

    private fun isAppInForeground(): Boolean {
        val processInfo = ActivityManager.RunningAppProcessInfo()
        ActivityManager.getMyMemoryState(processInfo)
        return processInfo.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND ||
            processInfo.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_VISIBLE
    }

    private fun isDeviceInteractive(context: Context): Boolean {
        val powerManager = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
        return powerManager?.isInteractive ?: true
    }

    private fun emitState() {
        val snapshot = synchronized(lock) { snapshotLocked() }
        NativeAudioPlugin.emitToActive(snapshot)
    }

    private fun progressPrefs(context: Context): SharedPreferences =
        context.getSharedPreferences(PROGRESS_PREFS_NAME, Context.MODE_PRIVATE)

    private fun applyShuffleOrderLocked(exoPlayer: ExoPlayer, first: Int?, recent: Set<Int> = emptySet()) {
        val count = exoPlayer.mediaItemCount
        val order = ShuffleOrderBuilder.build(
            count = count,
            first = first?.takeIf { it in 0 until count },
            recent = recent,
            artistOf = { exoPlayer.getMediaItemAt(it).mediaMetadata.artist?.toString() },
        )
        setShuffleIndicesLocked(exoPlayer, order)
    }

    private fun setShuffleIndicesLocked(exoPlayer: ExoPlayer, order: IntArray) {
        shuffleIndices = order
        exoPlayer.setShuffleOrder(DefaultShuffleOrder(order, Random.nextLong()))
    }

    /** Queue indices in play order: our shuffle order when shuffle is on, list order otherwise. */
    private fun playOrderLocked(exoPlayer: ExoPlayer): IntArray {
        val count = exoPlayer.mediaItemCount
        if (exoPlayer.shuffleModeEnabled && shuffleIndices.size == count) return shuffleIndices.copyOf()
        return IntArray(count) { it }
    }

    /** Repeat mode, shuffle mode and the exact saved shuffle order of a restored queue. */
    private fun applySnapshotModesLocked(exoPlayer: ExoPlayer, snapshot: QueueSnapshotStore.Snapshot) {
        queueSourceId = snapshot.sourceId
        repeatMode = snapshot.repeatMode
        exoPlayer.repeatMode = snapshot.repeatMode
        shuffleEnabled = snapshot.shuffle
        exoPlayer.shuffleModeEnabled = snapshot.shuffle
        lastMediaItemIndex = exoPlayer.currentMediaItemIndex
        if (snapshot.shuffle) {
            val order = snapshot.shuffleOrder
            if (order != null && order.size == exoPlayer.mediaItemCount) {
                setShuffleIndicesLocked(exoPlayer, order)
            } else {
                applyShuffleOrderLocked(exoPlayer, exoPlayer.currentMediaItemIndex)
            }
        }
    }

    private fun saveQueueLocked() {
        if (restoringQueue) return
        val context = appContext ?: return
        val exoPlayer = player ?: return
        val count = exoPlayer.mediaItemCount
        QueueSnapshotStore.saveQueue(
            context,
            items = (0 until count).map { exoPlayer.getMediaItemAt(it) },
            shuffle = exoPlayer.shuffleModeEnabled,
            shuffleOrder = if (shuffleIndices.size == count) shuffleIndices else IntArray(0),
            repeatMode = exoPlayer.repeatMode,
            sourceId = queueSourceId,
        )
        saveQueuePositionLocked(force = true)
    }

    private fun saveQueuePositionLocked(force: Boolean) {
        if (restoringQueue) return
        val context = appContext ?: return
        val exoPlayer = player ?: return
        if (exoPlayer.mediaItemCount == 0) return
        val now = System.currentTimeMillis()
        if (!force && now - lastQueuePositionSavedAtMs < QUEUE_POSITION_SAVE_THROTTLE_MS) return
        lastQueuePositionSavedAtMs = now
        QueueSnapshotStore.savePosition(context, exoPlayer.currentMediaItemIndex, max(0L, exoPlayer.currentPosition))
    }

    /**
     * Playback wrapped from the last track of the play order to the first (repeat all). Drops the
     * tracks added with addToQueue when repeatAddedTracks is off, and with shuffle on builds a new
     * order for the next pass, since ExoPlayer would replay the same order every time.
     */
    private fun handleWrapLocked(exoPlayer: ExoPlayer, reason: Int) {
        val previousIndex = lastMediaItemIndex
        var currentIndex = exoPlayer.currentMediaItemIndex
        lastMediaItemIndex = currentIndex

        if (reason == Player.MEDIA_ITEM_TRANSITION_REASON_REPEAT ||
            reason == Player.MEDIA_ITEM_TRANSITION_REASON_PLAYLIST_CHANGED
        ) return
        if (exoPlayer.repeatMode != Player.REPEAT_MODE_ALL) return
        var order = playOrderLocked(exoPlayer)
        if (order.size < 2 || previousIndex != order.last() || currentIndex != order.first()) return

        val context = appContext
        if (context != null && !PluginSettings.repeatAddedTracks(context)) {
            val added = (0 until exoPlayer.mediaItemCount)
                .filter { it != currentIndex && isAddedToQueue(exoPlayer.getMediaItemAt(it)) }
            // Highest index first, so the lower indices stay valid.
            for (index in added.sortedDescending()) {
                exoPlayer.removeMediaItem(index)
                order = QueueOrder.remove(order, index)
            }
            currentIndex = exoPlayer.currentMediaItemIndex
            lastMediaItemIndex = currentIndex
            if (added.isNotEmpty() && exoPlayer.shuffleModeEnabled) setShuffleIndicesLocked(exoPlayer, order)
        }

        if (exoPlayer.shuffleModeEnabled && order.size >= 2) {
            // The current track opened the old pass, so it's the least recently played one.
            applyShuffleOrderLocked(exoPlayer, currentIndex, ShuffleOrderBuilder.recentTail(order))
        }
    }

    /** Logs `start` for the current track the first time it plays. */
    private fun logStartIfNeededLocked() {
        if (currentStartLogged) return
        val exoPlayer = player ?: return
        currentStartLogged = true
        logPlaybackEventLocked("start", exoPlayer.currentMediaItem, max(0L, exoPlayer.currentPosition))
        countPlayLocked(exoPlayer.currentMediaItem, exoPlayer.currentPosition, completed = false)
    }

    /** The player moved off a track: it either played to the end, or was skipped or removed. */
    private fun logTrackLeftLocked(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) {
        val leftTrack = reason == Player.DISCONTINUITY_REASON_AUTO_TRANSITION ||
            reason == Player.DISCONTINUITY_REASON_REMOVE ||
            oldPosition.mediaItemIndex != newPosition.mediaItemIndex
        if (!leftTrack) return

        val oldId = oldPosition.mediaItem?.mediaId?.toLongOrNull()
        val oldDurationMs = if (oldId != null && oldId == knownDurationItemId) knownDurationMs else 0L
        if (reason == Player.DISCONTINUITY_REASON_AUTO_TRANSITION) {
            logPlaybackEventLocked("complete", oldPosition.mediaItem, oldPosition.positionMs)
            countPlayLocked(oldPosition.mediaItem, oldPosition.positionMs, completed = true)
            recordItemProgressLocked(oldPosition.mediaItem, oldPosition.positionMs, oldDurationMs, completed = true, notify = true)
        } else if (currentStartLogged) {
            recordItemProgressLocked(oldPosition.mediaItem, oldPosition.positionMs, oldDurationMs, completed = false, notify = true)
            logPlaybackEventLocked("skip", oldPosition.mediaItem, oldPosition.positionMs)
        }
        // A new track (or the same one again, with repeat one): its start is logged once it plays.
        currentStartLogged = false
        countedLists.clear()
        if (player?.isPlaying == true) logStartIfNeededLocked()
    }

    /**
     * Counts the current play of [item] towards the tracked lists whose countAfterSeconds it has
     * reached ([completed]: it played to the end, which always counts). Each list once per play.
     */
    private fun countPlayLocked(item: MediaItem?, positionMs: Long, completed: Boolean) {
        if (item == null || !currentStartLogged && !completed) return
        val context = appContext ?: return
        for (list in TrackedLists.configs(context)) {
            if (list.id in countedLists) continue
            if (!completed && positionMs < list.countAfterMs) continue
            val value: Any = when (list.track) {
                "item" -> item.mediaId.toLongOrNull() ?: continue
                else -> queueSourceId ?: continue
            }
            countedLists.add(list.id)
            val change = TrackedLists.record(context, list, value) ?: continue
            NativeAudioPlugin.emitTrackedListChangeToActive(change)
            notifyTrackedListFolders(context, list.id)
        }
    }

    private fun rememberDurationLocked(exoPlayer: Player) {
        val duration = exoPlayer.duration
        val id = exoPlayer.currentMediaItem?.mediaId?.toLongOrNull() ?: return
        if (duration != C.TIME_UNSET && duration > 0) {
            knownDurationItemId = id
            knownDurationMs = duration
        }
    }

    /** With trackProgress: remembers how far [item] played; [notify] refreshes it in Android Auto. */
    private fun recordItemProgressLocked(item: MediaItem?, positionMs: Long, durationMs: Long, completed: Boolean, notify: Boolean) {
        val context = appContext ?: return
        if (!PluginSettings.trackProgress(context)) return
        val itemId = item?.mediaId?.toLongOrNull() ?: return
        val duration = if (durationMs == C.TIME_UNSET) 0L else durationMs
        ItemProgress.record(context, itemId, positionMs.coerceAtLeast(0L), duration, completed)
        if (notify) notifyItemFolders(context, setOf(itemId))
    }

    /** Lets Android Auto reload the library folders that contain any of [itemIds]. */
    private fun notifyItemFolders(context: Context, itemIds: Set<Long>) {
        val session = mediaSession ?: return
        val folderIds = MediaLibraryStore.get(context).folders.values
            .filter { folder -> folder.children.any { it is MediaLibraryStore.Node.ItemNode && it.itemId in itemIds } }
            .map { it.id }
        if (folderIds.isEmpty()) return
        tickHandler.post {
            folderIds.forEach { session.notifyChildrenChanged("folder:$it", Int.MAX_VALUE, null) }
        }
    }

    internal fun setItemProgress(context: Context, entries: List<ItemProgress.Entry>, merge: Boolean): List<ItemProgress.Entry> {
        val result = ItemProgress.set(context, entries, merge)
        synchronized(lock) { notifyItemFolders(context, entries.map { it.itemId }.toSet()) }
        return result
    }

    /** Lets Android Auto reload the library folders that show [listId]. */
    private fun notifyTrackedListFolders(context: Context, listId: String) {
        val session = mediaSession ?: return
        val folderIds = MediaLibraryStore.get(context).folders.values.filter { it.trackedList == listId }.map { it.id }
        if (folderIds.isEmpty()) return
        tickHandler.post {
            folderIds.forEach { session.notifyChildrenChanged("folder:$it", Int.MAX_VALUE, null) }
        }
    }

    /** Called by the library callback before Android Auto sets a queue started from a folder. */
    internal fun setPendingQueueSource(sourceId: String?) {
        synchronized(lock) {
            pendingQueueSourceId = sourceId
        }
    }

    fun setTrackedLists(context: Context, json: JSONObject) {
        TrackedLists.set(context, json)
        synchronized(lock) { countedLists.clear() }
    }

    internal fun setTrackedList(context: Context, listId: String, entries: JSONArray, merge: Boolean): List<TrackedLists.Entry> {
        val result = TrackedLists.set(context, listId, TrackedLists.parseEntries(entries), merge)
        synchronized(lock) { notifyTrackedListFolders(context, listId) }
        return result
    }

    private fun logPlaybackEventLocked(type: String, item: MediaItem?, positionMs: Long) {
        val context = appContext ?: return
        val itemId = item?.mediaId?.toLongOrNull() ?: return
        val event = PlaybackEvents.record(context, type, itemId, positionMs.coerceAtLeast(0L))
        NativeAudioPlugin.emitPlaybackEventToActive(event)
    }

    /** The artwork URL the app gave for [item] (not the provider URI it may have been swapped for). */
    internal fun artworkUrlOf(item: MediaItem): String? =
        item.mediaMetadata.extras?.getString(ARTWORK_URL_EXTRA) ?: item.mediaMetadata.artworkUri?.toString()

    internal fun isAddedToQueue(item: MediaItem): Boolean =
        item.mediaMetadata.extras?.getBoolean(ADDED_TO_QUEUE_EXTRA, false) == true

    private fun playerViewLocked(exoPlayer: Player) = PlaybackControls.PlayerView(
        shuffle = exoPlayer.shuffleModeEnabled,
        repeatMode = exoPlayer.repeatMode,
        speed = exoPlayer.playbackParameters.speed.toDouble(),
        itemId = exoPlayer.currentMediaItem?.mediaId?.toLongOrNull(),
        sleepTimerMinutes = sleepTimerMinutes,
        sleepTimerEndOfTrack = sleepTimerEndOfTrack,
    )

    private fun sleepTimerActive(): Boolean = synchronized(lock) { sleepTimerEndsAtMs != null || sleepTimerEndOfTrack }

    /**
     * Rebuilds the setControls buttons (their icons follow shuffle, repeat, speed and each track's
     * toggle state). With [updateCommands], also allows the (new) buttons for connected controllers.
     */
    private fun refreshControls(updateCommands: Boolean = false) {
        val (session, layout) = synchronized(lock) {
            val context = appContext ?: return
            val exoPlayer = player ?: return
            val session = mediaSession ?: return
            session to PlaybackControls.layout(context, playerViewLocked(exoPlayer))
        }
        val commands = if (updateCommands) PlaybackControls.sessionCommands(session, appContext ?: return) else null
        tickHandler.post {
            if (commands != null) {
                for (controller in session.connectedControllers) {
                    session.setAvailableCommands(controller, commands, MediaSession.ConnectionResult.DEFAULT_PLAYER_COMMANDS)
                }
            }
            session.setCustomLayout(layout)
        }
    }

    private fun seekByInterval(deltaMs: Long) {
        synchronized(lock) {
            val exoPlayer = player ?: return
            var target = max(0L, exoPlayer.currentPosition + deltaMs)
            val duration = exoPlayer.duration
            if (duration != C.TIME_UNSET) target = kotlin.math.min(target, duration)
            exoPlayer.seekTo(target)
        }
    }

    private fun repeatModeName(mode: Int): String = when (mode) {
        Player.REPEAT_MODE_ONE -> "one"
        Player.REPEAT_MODE_ALL -> "all"
        else -> "off"
    }

    private fun currentStoryIdLocked(): Long? =
        player?.currentMediaItem?.mediaId?.toLongOrNull()?.takeIf { it > 0 }

    private fun persistProgressCheckpointLocked(context: Context, snapshot: NativeAudioState, force: Boolean) {
        val storyId = currentStoryIdLocked() ?: return
        if (storyId <= 0L) return
        if (!snapshot.currentTime.isFinite() || snapshot.currentTime <= PROGRESS_NEAR_START_EPSILON_SEC) return

        val now = System.currentTimeMillis()
        if (!force && now - lastProgressPersistedAtMs < PROGRESS_PERSIST_THROTTLE_MS) return

        val prevStoryId = lastProgressPersistedStoryId
        val prevTime = lastProgressPersistedTimeSec
        if (!force && prevStoryId == storyId && prevTime != null && kotlin.math.abs(prevTime - snapshot.currentTime) <= PROGRESS_PERSIST_EPSILON_SEC) {
            return
        }

        progressPrefs(context).edit()
            .putLong(PROGRESS_KEY_STORY_ID, storyId)
            .putFloat(PROGRESS_KEY_CURRENT_TIME, snapshot.currentTime.toFloat())
            .putLong(PROGRESS_KEY_UPDATED_AT_MS, now)
            .putString(PROGRESS_KEY_STATUS, snapshot.status)
            .apply()

        lastProgressPersistedAtMs = now
        lastProgressPersistedStoryId = storyId
        lastProgressPersistedTimeSec = snapshot.currentTime
    }

    /**
     * A queue set through the media session (e.g. picked in Android Auto) instead of setQueue: apply
     * the same shuffle order rules. Runs on the main thread, inside the player call.
     */
    private fun onSessionPlaylistReplaced() {
        synchronized(lock) {
            val exoPlayer = player ?: return
            pendingSeekState = null
            lastError = null
            val resumption = pendingResumption
            pendingResumption = null
            queueSourceId = if (resumption != null) resumption.sourceId else pendingQueueSourceId
            pendingQueueSourceId = null
            if (resumption != null && resumption.items.size == exoPlayer.mediaItemCount) {
                restoringQueue = true
                try {
                    applySnapshotModesLocked(exoPlayer, resumption)
                } finally {
                    restoringQueue = false
                }
                saveQueueLocked()
                return
            }
            lastMediaItemIndex = exoPlayer.currentMediaItemIndex
            if (exoPlayer.shuffleModeEnabled) applyShuffleOrderLocked(exoPlayer, exoPlayer.currentMediaItemIndex)
        }
    }

    internal fun buildMediaItem(
        src: String,
        id: Long?,
        title: String?,
        artist: String?,
        artworkUrl: String?,
        addedToQueue: Boolean = false,
    ): MediaItem {
        val metadataBuilder = MediaMetadata.Builder()
        val extras = android.os.Bundle()
        if (addedToQueue) extras.putBoolean(ADDED_TO_QUEUE_EXTRA, true)
        // The artwork URL as the app gave it (local files are swapped for a provider URI below).
        if (!artworkUrl.isNullOrBlank()) extras.putString(ARTWORK_URL_EXTRA, artworkUrl)
        if (!extras.isEmpty) metadataBuilder.setExtras(extras)
        if (!title.isNullOrBlank()) metadataBuilder.setTitle(title)
        if (!artist.isNullOrBlank()) metadataBuilder.setArtist(artist)
        if (!artworkUrl.isNullOrBlank()) {
            // Local image files go through ArtworkProvider so the system media controls and Android
            // Auto can read them. (Covers embedded in the audio file are read by the player itself.)
            val context = appContext
            val uri = if (context != null) {
                ArtworkRegistry.artworkUri(context, artworkUrl, audioSrc = null, embedded = false)
            } else {
                runCatching { Uri.parse(artworkUrl) }.getOrNull()
            }
            uri?.let { metadataBuilder.setArtworkUri(it) }
        }
        return MediaItem.Builder()
            .apply { if (id != null) setMediaId(id.toString()) }
            .setUri(src)
            .setMediaMetadata(metadataBuilder.build())
            .build()
    }

    private fun snapshotLocked(): NativeAudioState {
        val exoPlayer = player
            ?: return NativeAudioState(
                status = "idle",
                currentTime = 0.0,
                duration = 0.0,
                isPlaying = false,
                buffering = false,
                rate = 1.0,
                queueIndex = -1,
                queueLength = 0,
                currentId = null,
                shuffle = shuffleEnabled,
                repeatMode = repeatModeName(repeatMode),
                sleepTimerEndsAtMs = sleepTimerEndsAtMs,
                sleepTimerEndOfTrack = sleepTimerEndOfTrack,
                error = null,
                volume = volume.toDouble(),
            )

        val rawDurationMs = exoPlayer.duration
        val durationMs = if (rawDurationMs > 0) rawDurationMs else 0L
        val currentMs = max(0L, exoPlayer.currentPosition)
        val buffering = exoPlayer.playbackState == Player.STATE_BUFFERING

        val seekState = activeSeekStateLocked()
        if (seekState?.shouldResume == true && exoPlayer.isPlaying) pendingSeekState = null

        val hasTerminalState = lastError != null || exoPlayer.playbackState == Player.STATE_ENDED
        if (hasTerminalState) pendingSeekState = null
        val effectiveIsPlaying = if (hasTerminalState) false else (seekState?.shouldResume ?: exoPlayer.isPlaying)
        val effectiveBuffering = if (hasTerminalState || seekState?.shouldResume == false) false else buffering

        val status = when {
            lastError != null -> "error"
            exoPlayer.playbackState == Player.STATE_ENDED -> "ended"
            seekState?.shouldResume == true -> "playing"
            effectiveBuffering -> "loading"
            effectiveIsPlaying -> "playing"
            else -> "idle"
        }

        return NativeAudioState(
            status = status,
            currentTime = currentMs / 1000.0,
            duration = durationMs / 1000.0,
            isPlaying = effectiveIsPlaying,
            buffering = effectiveBuffering,
            rate = exoPlayer.playbackParameters.speed.toDouble(),
            queueIndex = if (exoPlayer.mediaItemCount > 0) exoPlayer.currentMediaItemIndex else -1,
            queueLength = exoPlayer.mediaItemCount,
            currentId = exoPlayer.currentMediaItem?.mediaId?.toLongOrNull(),
            shuffle = exoPlayer.shuffleModeEnabled,
            repeatMode = repeatModeName(exoPlayer.repeatMode),
            sleepTimerEndsAtMs = sleepTimerEndsAtMs,
            sleepTimerEndOfTrack = sleepTimerEndOfTrack,
            error = lastError,
            volume = volume.toDouble(),
        )
    }

    private fun activeSeekStateLocked(): PendingSeekState? {
        val seekState = pendingSeekState ?: return null
        val now = System.currentTimeMillis()
        if (now - seekState.startedAtMs > SEEK_STATE_STALE_MS) {
            pendingSeekState = null
            return null
        }
        return seekState
    }
}
