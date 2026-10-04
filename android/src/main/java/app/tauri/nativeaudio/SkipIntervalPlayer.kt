package app.tauri.nativeaudio

import androidx.media3.common.C
import androidx.media3.common.FlagSet
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import java.util.concurrent.CopyOnWriteArraySet
import kotlin.math.max
import kotlin.math.min

private val SEEK_TO_TRACK_COMMANDS = intArrayOf(
    Player.COMMAND_SEEK_TO_PREVIOUS,
    Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
    Player.COMMAND_SEEK_TO_NEXT,
    Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
)

// ExoPlayer always offers these with a fixed 10 second step. The previous / next buttons do the
// seeking when the skip interval is above 0, so these are never advertised.
private val FIXED_SEEK_COMMANDS = intArrayOf(Player.COMMAND_SEEK_BACK, Player.COMMAND_SEEK_FORWARD)

/**
 * The player handed to the media session and notification. With a skip interval of 0 the
 * previous / next buttons move through the queue; above 0 they seek by that interval instead.
 *
 * Note: Android Auto's player for apps without Android Auto support draws its own ±10 second
 * buttons and ignores these commands.
 *
 * [onPlaylistReplaced] runs after a new queue is set through this player, i.e. by the media session
 * (Android Auto picking something to play), not by setQueue.
 */
internal class SkipIntervalPlayer(
    private val player: Player,
    private val skipIntervalMs: () -> Long,
    private val onPlaylistReplaced: () -> Unit,
    private val keepQueueOnStop: () -> Boolean,
    private val previousRestartAfterMs: () -> Long,
    private val nextAtEndFirst: () -> Boolean,
) : ForwardingPlayer(player) {
    private val listeners = CopyOnWriteArraySet<Player.Listener>()

    override fun addListener(listener: Player.Listener) {
        super.addListener(listener)
        listeners.add(listener)
    }

    override fun removeListener(listener: Player.Listener) {
        super.removeListener(listener)
        listeners.remove(listener)
    }

    /**
     * One cover for every view: when the app gave the playing item artwork, the picture embedded in the audio
     * file (which ExoPlayer adds as artwork data) isn't passed on. Otherwise Android Auto could take its small
     * player's colors from one picture and its big player's from the other.
     */
    override fun getMediaMetadata(): MediaMetadata {
        val metadata = super.getMediaMetadata()
        val appArtwork = player.currentMediaItem?.mediaMetadata?.artworkUri ?: return metadata
        if (metadata.artworkData == null) return metadata
        return metadata.buildUpon().setArtworkData(null, null).setArtworkUri(appArtwork).build()
    }

    override fun getAvailableCommands(): Player.Commands {
        val builder = super.getAvailableCommands().buildUpon().removeAll(*FIXED_SEEK_COMMANDS)
        if (skipIntervalMs() > 0L) builder.addAll(*SEEK_TO_TRACK_COMMANDS)
        // Next goes back to the first track at the end: keep the button on the last track too.
        if (nextAtEndFirst() && player.mediaItemCount > 0) builder.add(Player.COMMAND_SEEK_TO_NEXT)
        return builder.build()
    }

    override fun isCommandAvailable(command: Int): Boolean {
        if (command in FIXED_SEEK_COMMANDS) return false
        if (skipIntervalMs() > 0L && command in SEEK_TO_TRACK_COMMANDS) return true
        if (command == Player.COMMAND_SEEK_TO_NEXT && nextAtEndFirst() && player.mediaItemCount > 0) return true
        return super.isCommandAvailable(command)
    }

    /**
     * A stop from outside the app: Android's media panel sends one to players paused for about 10
     * minutes, Bluetooth devices and Android Auto can send one too. Stopping unloads the queue, which
     * empties Android Auto's player and the notification, so with keepQueueOnStop it pauses instead.
     */
    override fun stop() {
        if (keepQueueOnStop()) pause() else super.stop()
    }

    override fun seekToPrevious() {
        if (skipIntervalMs() > 0L) seekBy(-skipIntervalMs()) else TrackSkip.previous(player, previousRestartAfterMs())
    }

    override fun seekToPreviousMediaItem() {
        if (skipIntervalMs() > 0L) seekBy(-skipIntervalMs()) else super.seekToPreviousMediaItem()
    }

    override fun seekToNext() {
        if (skipIntervalMs() > 0L) seekBy(skipIntervalMs()) else TrackSkip.next(player, nextAtEndFirst())
    }

    override fun seekToNextMediaItem() {
        if (skipIntervalMs() > 0L) seekBy(skipIntervalMs()) else super.seekToNextMediaItem()
    }

    override fun setMediaItem(mediaItem: MediaItem) {
        super.setMediaItem(mediaItem)
        onPlaylistReplaced()
    }

    override fun setMediaItem(mediaItem: MediaItem, startPositionMs: Long) {
        super.setMediaItem(mediaItem, startPositionMs)
        onPlaylistReplaced()
    }

    override fun setMediaItem(mediaItem: MediaItem, resetPosition: Boolean) {
        super.setMediaItem(mediaItem, resetPosition)
        onPlaylistReplaced()
    }

    override fun setMediaItems(mediaItems: MutableList<MediaItem>) {
        super.setMediaItems(mediaItems)
        onPlaylistReplaced()
    }

    override fun setMediaItems(mediaItems: MutableList<MediaItem>, resetPosition: Boolean) {
        super.setMediaItems(mediaItems, resetPosition)
        onPlaylistReplaced()
    }

    override fun setMediaItems(mediaItems: MutableList<MediaItem>, startIndex: Int, startPositionMs: Long) {
        super.setMediaItems(mediaItems, startIndex, startPositionMs)
        onPlaylistReplaced()
    }

    /** Tells the session and notification to re-read the available commands. Main thread only. */
    fun notifyAvailableCommandsChanged() {
        val commands = availableCommands
        val events = Player.Events(FlagSet.Builder().add(Player.EVENT_AVAILABLE_COMMANDS_CHANGED).build())
        for (listener in listeners) {
            listener.onAvailableCommandsChanged(commands)
            listener.onEvents(this, events)
        }
    }

    private fun seekBy(deltaMs: Long) {
        var target = max(0L, player.currentPosition + deltaMs)
        val duration = player.duration
        if (duration != C.TIME_UNSET) target = min(target, duration)
        player.seekTo(target)
    }
}
