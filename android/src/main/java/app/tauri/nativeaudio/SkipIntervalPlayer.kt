package app.tauri.nativeaudio

import androidx.media3.common.C
import androidx.media3.common.FlagSet
import androidx.media3.common.ForwardingPlayer
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
 */
internal class SkipIntervalPlayer(
    private val player: Player,
    private val skipIntervalMs: () -> Long,
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

    override fun getAvailableCommands(): Player.Commands {
        val builder = super.getAvailableCommands().buildUpon().removeAll(*FIXED_SEEK_COMMANDS)
        if (skipIntervalMs() > 0L) builder.addAll(*SEEK_TO_TRACK_COMMANDS)
        return builder.build()
    }

    override fun isCommandAvailable(command: Int): Boolean {
        if (command in FIXED_SEEK_COMMANDS) return false
        if (skipIntervalMs() > 0L && command in SEEK_TO_TRACK_COMMANDS) return true
        return super.isCommandAvailable(command)
    }

    override fun seekToPrevious() {
        if (skipIntervalMs() > 0L) seekBy(-skipIntervalMs()) else super.seekToPrevious()
    }

    override fun seekToPreviousMediaItem() {
        if (skipIntervalMs() > 0L) seekBy(-skipIntervalMs()) else super.seekToPreviousMediaItem()
    }

    override fun seekToNext() {
        if (skipIntervalMs() > 0L) seekBy(skipIntervalMs()) else super.seekToNext()
    }

    override fun seekToNextMediaItem() {
        if (skipIntervalMs() > 0L) seekBy(skipIntervalMs()) else super.seekToNextMediaItem()
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
