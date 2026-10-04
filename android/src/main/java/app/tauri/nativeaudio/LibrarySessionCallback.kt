package app.tauri.nativeaudio

import android.content.Context
import android.net.Uri
import android.os.Bundle
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.session.LibraryResult
import androidx.media3.session.MediaConstants
import androidx.media3.session.MediaLibraryService.LibraryParams
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionCommand
import androidx.media3.session.SessionResult
import com.google.common.collect.ImmutableList
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import kotlin.math.min

private const val ROOT_ID = "root"
private const val FOLDER_PREFIX = "folder:"
private const val ITEM_PREFIX = "item:"
private const val EMPTY_ID = "empty"
private const val MAX_SEARCH_RESULTS = 50

/**
 * Serves the [MediaLibraryStore] tree to Android Auto (and other media browsers), and turns what
 * the user picks into a queue.
 *
 * Media ids: `root`, `folder:<folderId>`, and `item:<itemId>:<folderId>` for an item shown inside
 * a folder, so picking it can play the rest of that folder too. The resulting queue items use the
 * plain item id as media id, like `setQueue` does.
 */
internal class LibrarySessionCallback(private val context: Context) : MediaLibrarySession.Callback {
    /** Allows the setControls buttons for every controller (Android Auto, the media controls). */
    override fun onConnect(
        session: MediaSession,
        controller: MediaSession.ControllerInfo,
    ): MediaSession.ConnectionResult =
        MediaSession.ConnectionResult.AcceptedResultBuilder(session)
            .setAvailableSessionCommands(PlaybackControls.sessionCommands(session, context))
            .build()

    override fun onCustomCommand(
        session: MediaSession,
        controller: MediaSession.ControllerInfo,
        customCommand: SessionCommand,
        args: Bundle,
    ): ListenableFuture<SessionResult> {
        val handled = NativeAudioRuntime.onControlCommand(context, customCommand.customAction)
        return Futures.immediateFuture(
            SessionResult(if (handled) SessionResult.RESULT_SUCCESS else SessionResult.RESULT_ERROR_NOT_SUPPORTED),
        )
    }

    override fun onGetLibraryRoot(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<MediaItem>> {
        val root = MediaItem.Builder()
            .setMediaId(ROOT_ID)
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setIsBrowsable(true)
                    .setIsPlayable(false)
                    .setMediaType(MediaMetadata.MEDIA_TYPE_FOLDER_MIXED)
                    .build(),
            )
            .build()
        return Futures.immediateFuture(LibraryResult.ofItem(root, params))
    }

    override fun onGetChildren(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        parentId: String,
        page: Int,
        pageSize: Int,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> {
        val library = MediaLibraryStore.get(context)
        val children = when {
            parentId == ROOT_ID && library.root.isEmpty() && library.emptyMessage != null ->
                listOf(emptyTab(library.emptyMessage))
            parentId == ROOT_ID -> library.root.map { browseItem(it) }
            parentId == EMPTY_ID -> emptyList()
            parentId.startsWith(FOLDER_PREFIX) -> {
                val folder = library.folders[parentId.removePrefix(FOLDER_PREFIX)]
                    ?: return Futures.immediateFuture(LibraryResult.ofError(LibraryResult.RESULT_ERROR_BAD_VALUE))
                childrenOf(library, folder).mapNotNull { child ->
                    when (child) {
                        is MediaLibraryStore.Node.FolderNode -> browseItem(child.folder)
                        is MediaLibraryStore.Node.ItemNode -> library.items[child.itemId]?.let { browseItem(it, folder) }
                    }
                }
            }
            else -> return Futures.immediateFuture(LibraryResult.ofError(LibraryResult.RESULT_ERROR_BAD_VALUE))
        }

        val from = page.coerceAtLeast(0) * pageSize.coerceAtLeast(1)
        val pageItems = if (from >= children.size) emptyList() else children.subList(from, min(children.size, from + pageSize))
        return Futures.immediateFuture(LibraryResult.ofItemList(pageItems, params))
    }

    override fun onSearch(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        query: String,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<Void>> {
        val library = MediaLibraryStore.get(context)
        if (!library.search) return Futures.immediateFuture(LibraryResult.ofError(LibraryResult.RESULT_ERROR_NOT_SUPPORTED))
        session.notifySearchResultChanged(browser, query, search(library, query).size, params)
        return Futures.immediateFuture(LibraryResult.ofVoid())
    }

    override fun onGetSearchResult(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        query: String,
        page: Int,
        pageSize: Int,
        params: LibraryParams?,
    ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> {
        val library = MediaLibraryStore.get(context)
        if (!library.search) return Futures.immediateFuture(LibraryResult.ofError(LibraryResult.RESULT_ERROR_NOT_SUPPORTED))
        val results = search(library, query)
        val from = page.coerceAtLeast(0) * pageSize.coerceAtLeast(1)
        val pageItems = if (from >= results.size) emptyList() else results.subList(from, min(results.size, from + pageSize))
        return Futures.immediateFuture(LibraryResult.ofItemList(pageItems, params))
    }

    override fun onGetItem(
        session: MediaLibrarySession,
        browser: MediaSession.ControllerInfo,
        mediaId: String,
    ): ListenableFuture<LibraryResult<MediaItem>> {
        val library = MediaLibraryStore.get(context)
        val item = when {
            mediaId.startsWith(FOLDER_PREFIX) -> library.folders[mediaId.removePrefix(FOLDER_PREFIX)]?.let { browseItem(it) }
            mediaId.startsWith(ITEM_PREFIX) -> parseItemId(mediaId)?.let { (itemId, folderId) ->
                library.items[itemId]?.let { browseItem(it, library.folders[folderId]) }
            }
            else -> null
        }
        return Futures.immediateFuture(
            if (item != null) LibraryResult.ofItem(item, null) else LibraryResult.ofError(LibraryResult.RESULT_ERROR_BAD_VALUE),
        )
    }

    override fun onSetMediaItems(
        mediaSession: MediaSession,
        controller: MediaSession.ControllerInfo,
        mediaItems: MutableList<MediaItem>,
        startIndex: Int,
        startPositionMs: Long,
    ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> {
        val library = MediaLibraryStore.get(context)
        NativeAudioRuntime.setPendingQueueSource(null)

        // "Play <something> on <app>" by voice: a request with a search query and no media id.
        val voiceQuery = mediaItems.singleOrNull()
            ?.takeIf { it.mediaId.isEmpty() && it.localConfiguration == null }
            ?.requestMetadata?.searchQuery
        if (voiceQuery != null && library.search) return playFromSearch(library, voiceQuery, startPositionMs)

        // One pick from a browse list: play its whole folder when that folder is playable.
        val single = mediaItems.singleOrNull()?.mediaId
        if (single != null) {
            val folderQueue = folderQueueFor(library, single)
            if (folderQueue != null) {
                val (queue, index, source) = folderQueue
                // A tracked-list folder isn't a playlist of its own, so it isn't recorded as one.
                NativeAudioRuntime.setPendingQueueSource(source.takeIf { it.trackedList == null }?.id)
                return Futures.immediateFuture(
                    withResume(MediaSession.MediaItemsWithStartPosition(queue.map(::playbackItem), index, startPositionMs)),
                )
            }
        }

        val resolved = mediaItems.flatMap { resolve(library, it) }
        return Futures.immediateFuture(
            withResume(
                MediaSession.MediaItemsWithStartPosition(resolved, startIndex.coerceIn(0, maxOf(0, resolved.size - 1)), startPositionMs),
            ),
        )
    }

    override fun onAddMediaItems(
        mediaSession: MediaSession,
        controller: MediaSession.ControllerInfo,
        mediaItems: MutableList<MediaItem>,
    ): ListenableFuture<MutableList<MediaItem>> {
        val library = MediaLibraryStore.get(context)
        return Futures.immediateFuture(mediaItems.flatMap { resolve(library, it) }.toMutableList())
    }

    /**
     * "Resume" with nothing loaded (media resumption, a play button while the app was closed): the
     * last saved queue. Its shuffle order and repeat mode are applied when the session sets it.
     */
    override fun onPlaybackResumption(
        mediaSession: MediaSession,
        controller: MediaSession.ControllerInfo,
    ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> {
        val snapshot = NativeAudioRuntime.prepareResumption(context)
            ?: return Futures.immediateFailedFuture(UnsupportedOperationException("no saved queue"))
        return Futures.immediateFuture(
            MediaSession.MediaItemsWithStartPosition(snapshot.items, snapshot.index, snapshot.positionMs),
        )
    }

    /**
     * Search over the library: playable folders by title, items by title, artist and album. Exact
     * matches first, then "starts with", then "contains"; titles before artists before albums.
     */
    private fun search(library: MediaLibraryStore.Library, query: String): List<MediaItem> {
        val needle = query.trim().lowercase()
        if (needle.isEmpty()) return emptyList()
        fun rank(text: String?, base: Int): Int? {
            val haystack = text?.lowercase() ?: return null
            return when {
                haystack == needle -> base
                haystack.startsWith(needle) -> base + 1
                haystack.contains(needle) -> base + 2
                else -> null
            }
        }
        val folders = library.folders.values
            .filter { it.playable && it.trackedList == null }
            .mapNotNull { folder -> rank(folder.title, 0)?.let { it to browseItem(folder) } }
        val items = library.items.values.mapNotNull { item ->
            listOfNotNull(rank(item.title, 0), rank(item.artist, 3), rank(item.album, 6)).minOrNull()
                ?.let { it to browseItem(item, null) }
        }
        // Stable sort: on equal rank, folders (playlists, albums) come before single items.
        return (folders + items).sortedBy { it.first }.take(MAX_SEARCH_RESULTS).map { it.second }
    }

    /**
     * Plays the best search match: a playlist / album as a whole, or a single item. An empty query
     * ("play <app>") continues the last queue, or plays the first playable folder.
     */
    private fun playFromSearch(
        library: MediaLibraryStore.Library,
        query: String,
        startPositionMs: Long,
    ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> {
        if (query.isBlank()) {
            NativeAudioRuntime.prepareResumption(context)?.let { snapshot ->
                return Futures.immediateFuture(
                    MediaSession.MediaItemsWithStartPosition(snapshot.items, snapshot.index, snapshot.positionMs),
                )
            }
            val firstFolder = library.folders.values.firstOrNull { it.playable && it.trackedList == null }
                ?: return Futures.immediateFailedFuture(UnsupportedOperationException("nothing to play"))
            return playFolderQueue(library, FOLDER_PREFIX + firstFolder.id, startPositionMs)
                ?: Futures.immediateFailedFuture(UnsupportedOperationException("nothing to play"))
        }

        val best = search(library, query).firstOrNull()
            ?: return Futures.immediateFailedFuture(UnsupportedOperationException("no match for $query"))
        playFolderQueue(library, best.mediaId, startPositionMs)?.let { return it }
        val items = resolve(library, best)
        return Futures.immediateFuture(withResume(MediaSession.MediaItemsWithStartPosition(items, 0, startPositionMs)))
    }

    private fun playFolderQueue(
        library: MediaLibraryStore.Library,
        mediaId: String,
        startPositionMs: Long,
    ): ListenableFuture<MediaSession.MediaItemsWithStartPosition>? {
        val (queue, index, source) = folderQueueFor(library, mediaId) ?: return null
        // A tracked-list folder isn't a playlist of its own, so it isn't recorded as one.
        NativeAudioRuntime.setPendingQueueSource(source.takeIf { it.trackedList == null }?.id)
        return Futures.immediateFuture(withResume(MediaSession.MediaItemsWithStartPosition(queue.map(::playbackItem), index, startPositionMs)))
    }

    /** Shown as the only tab when the library is empty, with the app's message as its title. */
    private fun emptyTab(message: String): MediaItem =
        MediaItem.Builder()
            .setMediaId(EMPTY_ID)
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(message)
                    .setIsBrowsable(true)
                    .setIsPlayable(false)
                    .setMediaType(MediaMetadata.MEDIA_TYPE_FOLDER_MIXED)
                    .build(),
            )
            .build()

    /** With trackProgress, the recorded progress (falling back to the library's); else the library's. */
    private fun progressOf(item: MediaLibraryStore.Item): Double? {
        if (!PluginSettings.trackProgress(context)) return item.progress
        return ItemProgress.get(context, item.id)?.progress ?: item.progress
    }

    /** With trackProgress, starts a partly played start item where it stopped. */
    private fun withResume(result: MediaSession.MediaItemsWithStartPosition): MediaSession.MediaItemsWithStartPosition {
        if (!PluginSettings.trackProgress(context)) return result
        val start = result.mediaItems.getOrNull(result.startIndex) ?: return result
        val position = start.mediaId.toLongOrNull()?.let { ItemProgress.get(context, it)?.resumePositionMs } ?: return result
        return MediaSession.MediaItemsWithStartPosition(result.mediaItems, result.startIndex, position)
    }

    private fun itemArtwork(item: MediaLibraryStore.Item): Uri? =
        ArtworkRegistry.artworkUri(context, item.artworkUrl, item.src, MediaLibraryStore.get(context).embeddedArtwork)

    /**
     * A folder's own artwork, or (by the library's folderArtwork) a collage of up to 4 covers of its
     * items from different albums, or its first cover.
     */
    private fun folderArtwork(folder: MediaLibraryStore.Folder): Uri? {
        if (!folder.artworkUrl.isNullOrBlank()) {
            return ArtworkRegistry.artworkUri(context, folder.artworkUrl, audioSrc = null, embedded = false)
        }
        val library = MediaLibraryStore.get(context)
        if (library.folderArtwork == "none") return null
        val covers = ArrayList<Uri>()
        val albums = HashSet<String>()
        for (item in itemsIn(library, folder)) {
            // One cover per album, so an album's own songs don't fill all four tiles.
            if (item.album != null && !albums.add(item.album)) continue
            itemArtwork(item)?.let { covers.add(it) }
            if (covers.size == 4 || library.folderArtwork == "first" && covers.isNotEmpty()) break
        }
        return if (library.folderArtwork == "first") covers.firstOrNull() else ArtworkRegistry.collageUri(context, covers)
    }

    /** Header and progress extras Android Auto reads from a browse item. */
    private fun displayExtras(group: String?, progress: Double?): Bundle {
        val extras = Bundle()
        if (!group.isNullOrBlank()) extras.putString(MediaConstants.EXTRAS_KEY_CONTENT_STYLE_GROUP_TITLE, group)
        if (progress != null) {
            val status = when {
                progress <= 0.0 -> MediaConstants.EXTRAS_VALUE_COMPLETION_STATUS_NOT_PLAYED
                progress >= 1.0 -> MediaConstants.EXTRAS_VALUE_COMPLETION_STATUS_FULLY_PLAYED
                else -> MediaConstants.EXTRAS_VALUE_COMPLETION_STATUS_PARTIALLY_PLAYED
            }
            extras.putInt(MediaConstants.EXTRAS_KEY_COMPLETION_STATUS, status)
            if (status == MediaConstants.EXTRAS_VALUE_COMPLETION_STATUS_PARTIALLY_PLAYED) {
                extras.putDouble(MediaConstants.EXTRAS_KEY_COMPLETION_PERCENTAGE, progress)
            }
        }
        return extras
    }

    /**
     * The queue, start index and folder for playing [mediaId]: a playable folder, or an item
     * inside one.
     */
    private fun folderQueueFor(
        library: MediaLibraryStore.Library,
        mediaId: String,
    ): Triple<List<MediaLibraryStore.Item>, Int, MediaLibraryStore.Folder>? {
        if (mediaId.startsWith(FOLDER_PREFIX)) {
            val folder = library.folders[mediaId.removePrefix(FOLDER_PREFIX)] ?: return null
            val items = itemsIn(library, folder)
            return if (items.isEmpty()) null else Triple(items, 0, folder)
        }
        val (itemId, folderId) = parseItemId(mediaId) ?: return null
        val folder = library.folders[folderId]?.takeIf { it.playable } ?: return null
        val items = itemsIn(library, folder)
        val index = items.indexOfFirst { it.id == itemId }
        return if (index < 0) null else Triple(items, index, folder)
    }

    /** A folder's children: its tracked list when it shows one (most recent first), else its own. */
    private fun childrenOf(library: MediaLibraryStore.Library, folder: MediaLibraryStore.Folder): List<MediaLibraryStore.Node> {
        val listId = folder.trackedList ?: return folder.children
        return TrackedLists.entries(context, listId).mapNotNull { entry ->
            when (val id = entry.id) {
                is Long -> id.takeIf { it in library.items }?.let { MediaLibraryStore.Node.ItemNode(it) }
                else -> library.folders[id.toString()]?.let { MediaLibraryStore.Node.FolderNode(it) }
            }
        }
    }

    /** Items directly inside [folder], in order; sub-folders are skipped. */
    private fun itemsIn(library: MediaLibraryStore.Library, folder: MediaLibraryStore.Folder): List<MediaLibraryStore.Item> =
        childrenOf(library, folder).mapNotNull { (it as? MediaLibraryStore.Node.ItemNode)?.let { node -> library.items[node.itemId] } }

    /** Turns a picked media id into playable queue items (a folder expands to its items). */
    private fun resolve(library: MediaLibraryStore.Library, mediaItem: MediaItem): List<MediaItem> {
        // Already playable (e.g. added by the app itself): keep as is.
        if (mediaItem.localConfiguration != null) return listOf(mediaItem)
        val mediaId = mediaItem.mediaId
        return when {
            mediaId.startsWith(FOLDER_PREFIX) ->
                library.folders[mediaId.removePrefix(FOLDER_PREFIX)]?.let { itemsIn(library, it) }.orEmpty().map(::playbackItem)
            mediaId.startsWith(ITEM_PREFIX) ->
                listOfNotNull(parseItemId(mediaId)?.let { library.items[it.first] }?.let(::playbackItem))
            else -> listOfNotNull(mediaId.toLongOrNull()?.let { library.items[it] }?.let(::playbackItem))
        }
    }

    private fun playbackItem(item: MediaLibraryStore.Item): MediaItem =
        NativeAudioRuntime.buildMediaItem(item.src, item.id, item.title, item.artist, item.artworkUrl)

    private fun browseItem(folder: MediaLibraryStore.Folder): MediaItem {
        val extras = displayExtras(folder.group, progress = null)
        val styleValue = when (folder.style) {
            "grid" -> MediaConstants.EXTRAS_VALUE_CONTENT_STYLE_GRID_ITEM
            "list" -> MediaConstants.EXTRAS_VALUE_CONTENT_STYLE_LIST_ITEM
            else -> null
        }
        if (styleValue != null) {
            // Applies to this folder's children.
            extras.putInt(MediaConstants.EXTRAS_KEY_CONTENT_STYLE_BROWSABLE, styleValue)
            extras.putInt(MediaConstants.EXTRAS_KEY_CONTENT_STYLE_PLAYABLE, styleValue)
        }
        return MediaItem.Builder()
            .setMediaId(FOLDER_PREFIX + folder.id)
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(folder.title)
                    .setSubtitle(folder.subtitle)
                    .setArtworkUri(folderArtwork(folder))
                    .setIsBrowsable(true)
                    // Android Auto plays a folder that's playable when it's tapped: openOnTap shows it as one to open.
                    .setIsPlayable(folder.playable && !folder.openOnTap)
                    .setMediaType(MediaMetadata.MEDIA_TYPE_FOLDER_MIXED)
                    .setExtras(extras)
                    .build(),
            )
            .build()
    }

    private fun browseItem(item: MediaLibraryStore.Item, parent: MediaLibraryStore.Folder?): MediaItem =
        MediaItem.Builder()
            .setMediaId("$ITEM_PREFIX${item.id}:${parent?.id.orEmpty()}")
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(item.title)
                    .setArtist(item.artist)
                    .setAlbumTitle(item.album)
                    .setArtworkUri(itemArtwork(item))
                    .setIsBrowsable(false)
                    .setIsPlayable(true)
                    .setMediaType(MediaMetadata.MEDIA_TYPE_MUSIC)
                    .setExtras(displayExtras(item.group, progressOf(item)))
                    .build(),
            )
            .build()

    /** `item:<itemId>:<folderId>` to (itemId, folderId); the folder id may itself contain ':'. */
    private fun parseItemId(mediaId: String): Pair<Long, String>? {
        val parts = mediaId.removePrefix(ITEM_PREFIX).split(":", limit = 2)
        val itemId = parts.firstOrNull()?.toLongOrNull() ?: return null
        return itemId to parts.getOrElse(1) { "" }
    }
}
