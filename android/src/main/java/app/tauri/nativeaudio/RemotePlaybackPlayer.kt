package app.tauri.nativeaudio

import android.net.Uri
import android.os.Looper
import android.os.SystemClock
import androidx.media3.common.C
import androidx.media3.common.DeviceInfo
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import org.json.JSONObject

/**
 * What plays on another device that this app controls (the app's own connection to it; see setRemotePlayback):
 * shown by the media session and the notification instead of this device's player.
 */
internal data class RemoteInfo(
    /** The other device's name ("Playing on …" in the notification). */
    val device: String,
    val title: String?,
    val artist: String?,
    val artworkUri: Uri?,
    val playing: Boolean,
    /** Where in the song, when this was given (it counts on while playing). */
    val positionMs: Long,
    val durationMs: Long,
    /** Whether there's a song after this one (previous always works: it starts the song again, or goes back). */
    val hasNext: Boolean,
    /** Its volume, in steps 0 to [maxVolume] (the volume buttons move it one step). */
    val volume: Int,
    val maxVolume: Int,
    /** Its shuffle and repeat (Player.REPEAT_MODE_*): the setControls buttons show them, and change them there. */
    val shuffle: Boolean = false,
    val repeatMode: Int = Player.REPEAT_MODE_OFF,
    /** The song's id in this app (when it has it too): the setControls custom buttons (like) are for it. */
    val itemId: Long? = null,
    /** When [positionMs] was taken (elapsed realtime). */
    val at: Long = SystemClock.elapsedRealtime(),
)

/**
 * The player the media session and the notification use while this device controls another one: its song, its
 * play / pause, previous, next and seeking, and its volume. Nothing plays here: each control goes to [send] (the
 * app passes it on to that device), and the other device's next state comes back through [update].
 *
 * As remote playback (DeviceInfo.PLAYBACK_TYPE_REMOTE), Android hands it the volume buttons: they move the other
 * device's volume, in its steps, and this device's own volume stays as it is.
 *
 * Its playlist is the song with a stand-in before it and (when there is one) after it, so previous and next are
 * always offered; they go to the other device as "previous" and "next", which decides what they do.
 */
internal class RemotePlaybackPlayer(looper: Looper, private val send: (JSONObject) -> Unit) : SimpleBasePlayer(looper) {
    var info: RemoteInfo? = null
        private set

    fun update(info: RemoteInfo) {
        this.info = info
        invalidateState()
    }

    /** A control for the other device from elsewhere (the setControls buttons: shuffle, repeat). */
    fun command(command: JSONObject) = send(command)

    override fun getState(): State {
        val info = info ?: return State.Builder().setPlaybackState(STATE_IDLE).build()
        val metadata = MediaMetadata.Builder()
            .setTitle(info.title)
            .setArtist(info.artist)
            .setSubtitle("Playing on ${info.device}")
            .apply { info.artworkUri?.let { setArtworkUri(it) } }
            .build()
        val current = MediaItemData.Builder(CURRENT)
            .setMediaItem(MediaItem.Builder().setMediaId(CURRENT).setMediaMetadata(metadata).build())
            .setMediaMetadata(metadata)
            .setDurationUs(if (info.durationMs > 0) info.durationMs * 1000 else C.TIME_UNSET)
            .setIsSeekable(info.durationMs > 0)
            .build()
        val playlist = buildList {
            add(MediaItemData.Builder(BEFORE).build())
            add(current)
            if (info.hasNext) add(MediaItemData.Builder(AFTER).build())
        }
        val position = PositionSupplier {
            val moved = if (info.playing) SystemClock.elapsedRealtime() - info.at else 0
            (info.positionMs + moved).coerceIn(0, if (info.durationMs > 0) info.durationMs else Long.MAX_VALUE)
        }
        return State.Builder()
            .setAvailableCommands(COMMANDS)
            .setPlayWhenReady(info.playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setPlaybackState(STATE_READY)
            .setPlaylist(playlist)
            .setCurrentMediaItemIndex(1)
            .setContentPositionMs(position)
            .setDeviceInfo(DeviceInfo.Builder(DeviceInfo.PLAYBACK_TYPE_REMOTE).setMinVolume(0).setMaxVolume(info.maxVolume).build())
            .setDeviceVolume(info.volume)
            .build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        send(JSONObject().put("cmd", if (playWhenReady) "play" else "pause"))
        info?.let { update(it.copy(playing = playWhenReady, positionMs = currentPosition, at = SystemClock.elapsedRealtime())) }
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        when (seekCommand) {
            COMMAND_SEEK_TO_NEXT, COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> send(JSONObject().put("cmd", "next"))
            COMMAND_SEEK_TO_PREVIOUS, COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> send(JSONObject().put("cmd", "previous"))
            else -> {
                val at = positionMs.coerceAtLeast(0)
                send(JSONObject().put("cmd", "seek").put("position", at / 1000.0))
                info?.let { update(it.copy(positionMs = at, at = SystemClock.elapsedRealtime())) }
            }
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleSetDeviceVolume(deviceVolume: Int, flags: Int): ListenableFuture<*> = volumeTo(deviceVolume)

    override fun handleIncreaseDeviceVolume(flags: Int): ListenableFuture<*> = volumeTo((info?.volume ?: 0) + 1)

    override fun handleDecreaseDeviceVolume(flags: Int): ListenableFuture<*> = volumeTo((info?.volume ?: 0) - 1)

    override fun handleSetDeviceMuted(muted: Boolean, flags: Int): ListenableFuture<*> =
        if (muted) volumeTo(0) else Futures.immediateVoidFuture()

    /** The other device's volume to `step` (of its steps), shown at once; it says its own next. */
    private fun volumeTo(step: Int): ListenableFuture<*> {
        val info = info ?: return Futures.immediateVoidFuture()
        val volume = step.coerceIn(0, info.maxVolume)
        send(JSONObject().put("cmd", "volume").put("step", volume).put("maxVolume", info.maxVolume))
        update(info.copy(volume = volume, positionMs = currentPosition, at = SystemClock.elapsedRealtime()))
        return Futures.immediateVoidFuture()
    }

    private companion object {
        const val BEFORE = "remote:before"
        const val CURRENT = "remote:current"
        const val AFTER = "remote:after"

        val COMMANDS: Player.Commands = Player.Commands.Builder()
            .addAll(
                COMMAND_PLAY_PAUSE,
                COMMAND_SEEK_TO_PREVIOUS,
                COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                COMMAND_SEEK_TO_NEXT,
                COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                COMMAND_GET_CURRENT_MEDIA_ITEM,
                COMMAND_GET_TIMELINE,
                COMMAND_GET_METADATA,
                COMMAND_GET_DEVICE_VOLUME,
                COMMAND_SET_DEVICE_VOLUME,
                COMMAND_SET_DEVICE_VOLUME_WITH_FLAGS,
                COMMAND_ADJUST_DEVICE_VOLUME,
                COMMAND_ADJUST_DEVICE_VOLUME_WITH_FLAGS,
            )
            .build()
    }
}
