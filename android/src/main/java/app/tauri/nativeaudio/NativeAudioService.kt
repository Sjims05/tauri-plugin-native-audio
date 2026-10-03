package app.tauri.nativeaudio

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.media3.session.MediaLibraryService
import androidx.media3.session.MediaSession
import androidx.media3.ui.PlayerNotificationManager

private const val NOTIFICATION_ID = 9501
private const val CHANNEL_ID_SUFFIX = ".native_audio"
private const val NOTIFICATION_ICON_NAME = "ic_notification"

/**
 * Keeps playback alive in the background and shows the media notification. It's a library service
 * so Android Auto can connect to it to browse and play; it's only reachable from outside the app
 * when carSupport is enabled (see build.rs).
 */
class NativeAudioService : MediaLibraryService() {
    private var notificationManager: PlayerNotificationManager? = null
    private var appLargeIcon: Bitmap? = null
    private val handler = Handler(Looper.getMainLooper())
    private var isForeground = false
    private var isPaused = false
    // After a pause the service stays in the foreground: with no time limit while Android Auto is
    // connected (keepAliveWhileCarConnected), otherwise for pausedKeepAliveMinutes. The controls stay
    // in the notification and in Android Auto, and resuming doesn't need a new foreground start,
    // which Android 12+ can refuse from the background.
    private var leaveForegroundScheduled = false
    private val leaveForeground = Runnable {
        leaveForegroundScheduled = false
        stopForegroundCompat(remove = false)
        isForeground = false
    }

    override fun onCreate() {
        super.onCreate()
        NativeAudioRuntime.ensure(applicationContext)
        // Registering the session connects media3's notification controller, which is what lets the
        // setControls buttons reach Android Auto and the Android 13+ media controls. Only with
        // carSupport: for other apps it makes Android Auto hide their now-playing card.
        if (NativeAudioRuntime.carSupportEnabled(this)) NativeAudioRuntime.mediaSession()?.let { addSession(it) }
        setupNotificationManager()
        NativeAudioRuntime.onKeepAliveRulesChanged = { applyPausedRules() }
        NativeAudioRuntime.startWatchingCarConnection(this)
        // Started with nothing loaded (e.g. Android Auto connecting while the app was closed):
        // load the last queue, paused, so it's ready to continue where it stopped.
        NativeAudioRuntime.restoreLastQueueIfIdle(applicationContext)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaLibrarySession? {
        return NativeAudioRuntime.mediaSession()
    }

    override fun onUpdateNotification(session: MediaSession, startInForegroundRequired: Boolean) {
        // PlayerNotificationManager is the single source for media controls in notification shade.
    }

    /**
     * While paused and in the foreground: stay as long as Android Auto is connected, otherwise for
     * pausedKeepAliveMinutes (counted from the pause, or from Android Auto disconnecting).
     */
    private fun applyPausedRules() {
        if (!isPaused || !isForeground) return
        val keepForCar = NativeAudioRuntime.carConnected && PluginSettings.keepAliveWhileCarConnected(this)
        val keepAliveMs = PluginSettings.pausedKeepAliveMs(this)
        when {
            keepForCar -> {
                handler.removeCallbacks(leaveForeground)
                leaveForegroundScheduled = false
            }
            keepAliveMs > 0 -> if (!leaveForegroundScheduled) {
                leaveForegroundScheduled = true
                handler.postDelayed(leaveForeground, keepAliveMs)
            }
            else -> {
                handler.removeCallbacks(leaveForeground)
                leaveForeground.run()
            }
        }
    }

    override fun onDestroy() {
        // The session outlives this service (it belongs to the runtime), so unregister it here: otherwise
        // the media3 controller this service connected to it stays, and every service restart (each
        // Android Auto connection, each app start) leaves one more behind.
        NativeAudioRuntime.mediaSession()?.let { if (it in sessions) removeSession(it) }
        NativeAudioRuntime.stopWatchingCarConnection()
        NativeAudioRuntime.onKeepAliveRulesChanged = null
        handler.removeCallbacks(leaveForeground)
        notificationManager?.setPlayer(null)
        notificationManager = null
        appLargeIcon?.recycle()
        appLargeIcon = null
        super.onDestroy()
    }

    private fun setupNotificationManager() {
        val player = NativeAudioRuntime.notificationPlayer() ?: return
        val mediaSession = NativeAudioRuntime.mediaSession() ?: return
        if (notificationManager != null) return

        ensureNotificationChannel()

        notificationManager = PlayerNotificationManager.Builder(this, NOTIFICATION_ID, channelId())
            .setMediaDescriptionAdapter(
                object : PlayerNotificationManager.MediaDescriptionAdapter {
                    override fun getCurrentContentTitle(player: androidx.media3.common.Player): CharSequence {
                        return player.mediaMetadata.title ?: appDisplayName()
                    }

                    override fun createCurrentContentIntent(player: androidx.media3.common.Player): PendingIntent? {
                        return mediaSession.sessionActivity
                    }

                    override fun getCurrentContentText(player: androidx.media3.common.Player): CharSequence? {
                        return player.mediaMetadata.artist
                    }

                    override fun getCurrentLargeIcon(
                        player: androidx.media3.common.Player,
                        callback: PlayerNotificationManager.BitmapCallback,
                    ): Bitmap? {
                        if (appLargeIcon == null) {
                            val iconResId = resolveAppIconResId()
                            if (iconResId != 0) appLargeIcon = BitmapFactory.decodeResource(resources, iconResId)
                        }
                        return appLargeIcon
                    }
                },
            )
            .setNotificationListener(
                object : PlayerNotificationManager.NotificationListener {
                    override fun onNotificationPosted(notificationId: Int, notification: Notification, ongoing: Boolean) {
                        isPaused = !ongoing
                        if (ongoing) {
                            handler.removeCallbacks(leaveForeground)
                            leaveForegroundScheduled = false
                            // Android 12+ can refuse this when playback resumes from the background
                            // (ForegroundServiceStartNotAllowedException); don't crash the app over it.
                            try {
                                startForeground(notificationId, notification)
                                isForeground = true
                            } catch (e: IllegalStateException) {
                                Log.w("plugin/native-audio", "startForeground not allowed", e)
                            }
                            return
                        }

                        // Paused or stopped.
                        if (isForeground) applyPausedRules() else stopForegroundCompat(remove = false)
                    }

                    override fun onNotificationCancelled(notificationId: Int, dismissedByUser: Boolean) {
                        handler.removeCallbacks(leaveForeground)
                        leaveForegroundScheduled = false
                        isForeground = false
                        stopForegroundCompat(remove = true)
                        stopSelf()
                    }
                },
            )
            .build()
            .apply {
                setMediaSessionToken(mediaSession.platformToken)
                setUsePlayPauseActions(true)
                setUsePreviousAction(true)
                setUseNextAction(true)
                setUseFastForwardAction(false)
                setUseRewindAction(false)
                setUsePreviousActionInCompactView(true)
                setUseNextActionInCompactView(true)
                setUseRewindActionInCompactView(false)
                setUseFastForwardActionInCompactView(false)
                setUseStopAction(false)
                setSmallIcon(resolveNotificationSmallIconResId())
                setPlayer(player)
            }
    }

    private fun channelId(): String {
        return "${packageName}${CHANNEL_ID_SUFFIX}"
    }

    private fun appDisplayName(): String {
        return applicationInfo.loadLabel(packageManager)?.toString().orEmpty().ifBlank { "Audio app" }
    }

    private fun resolveNotificationSmallIconResId(): Int {
        val notificationIcon = resources.getIdentifier(NOTIFICATION_ICON_NAME, "drawable", packageName)
        if (notificationIcon != 0) return notificationIcon
        return android.R.drawable.ic_media_play
    }

    private fun resolveAppIconResId(): Int {
        val appIcon = applicationInfo.icon
        return if (appIcon != 0) appIcon else android.R.drawable.sym_def_app_icon
    }

    private fun stopForegroundCompat(remove: Boolean) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(if (remove) Service.STOP_FOREGROUND_REMOVE else Service.STOP_FOREGROUND_DETACH)
            return
        }
        @Suppress("DEPRECATION")
        stopForeground(remove)
    }

    private fun ensureNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (manager.getNotificationChannel(channelId()) != null) return
        val channel = NotificationChannel(channelId(), appDisplayName(), NotificationManager.IMPORTANCE_LOW).apply {
            description = "Audio playback controls"
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }
}
