package app.tauri.nativeaudio

import androidx.media3.common.Player

/**
 * What previous and next do with the queue: the same for the app's previous() / next() and for the
 * notification, lock screen, headset and Android Auto buttons (SkipIntervalPlayer).
 */
internal object TrackSkip {
    /**
     * Previous: restarts the current track when it's past [restartAfterMs] (0: never), otherwise
     * goes to the previous track (the start of the current one when there's none).
     */
    fun previous(player: Player, restartAfterMs: Long) {
        when {
            restartAfterMs > 0L && player.currentPosition > restartAfterMs -> player.seekTo(0L)
            player.hasPreviousMediaItem() -> player.seekToPreviousMediaItem()
            else -> player.seekTo(0L)
        }
    }

    /**
     * Next: the next track. At the end of the queue (repeat off) nothing happens, or with
     * [firstAtEnd] it goes back to the first track of the play order, paused. Returns whether it moved.
     */
    fun next(player: Player, firstAtEnd: Boolean): Boolean {
        if (player.hasNextMediaItem()) {
            player.seekToNextMediaItem()
            return true
        }
        if (!firstAtEnd || player.mediaItemCount == 0) return false
        val first = player.currentTimeline.getFirstWindowIndex(player.shuffleModeEnabled)
        player.pause()
        player.seekTo(first, 0L)
        return true
    }
}
