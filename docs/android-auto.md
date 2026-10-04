# Android Auto

[← README](../README.MD) · [Playback](playback.md) · [Syncing](syncing.md)

With `carSupport` enabled, the app shows up in Android Auto as a media app: Auto can browse a library you provide
and play from it, using the plugin's queue, controls and saved queue.

- [Setting up](#setting-up)
- [The library](#the-library)
- [Search](#search)
- [Artwork](#artwork)
- [Designing your Android Auto app](#designing-your-android-auto-app)
- [Extra buttons](#extra-buttons)
- [Good to know](#good-to-know)

## Setting up

```json
{ "plugins": { "native-audio": { "carSupport": true } } }
```

- `carSupport` (in `tauri.conf.json`, default `false`) is applied at build time, so changing it needs a rebuild. It
  needs the `mobile` Cargo feature (on by default).
- The plugin's build step adds what Auto needs to the app's `AndroidManifest.xml` (an exported media library
  service, the Auto media app entry and the Android for Cars library), and removes it again when `carSupport` is
  turned off.
- **Card icon:** the icon on Auto's player and media card is a one-color icon that Auto tints. By default Auto uses
  the app icon, which shows as a filled square when it has a solid background. To use your symbol, add a drawable
  named `native_audio_car_icon` to the Android project: white on transparent, 24 dp, for example
  `src-tauri/gen/android/app/src/main/res/drawable-xxxhdpi/native_audio_car_icon.png` (96 px) and the other
  densities. The build step points Auto to it.
- **Testing a sideloaded build:** enable Android Auto's developer settings and turn on **Unknown sources**.

## The library

```ts
import { setLibrary } from "tauri-plugin-native-audio-api";

await setLibrary({
  // Each playable item is defined once, then referenced by id from any number of folders.
  items: [
    { id: 1, src: "/…/1.mp3", title: "Song 1", artist: "Artist", album: "Album" },
    { id: 2, src: "/…/2.mp3", title: "Song 2", artist: "Artist", album: "Album" },
  ],
  // Top-level folders are Auto's tabs (it shows up to 4).
  root: [
    { id: "songs", title: "All songs", playable: true, style: "list", children: [1, 2] },
    {
      id: "playlists",
      title: "Playlists",
      children: [{ id: "pl-1", title: "Road trip", playable: true, children: [2, 1] }],
    },
  ],
});
```

- It's saved on the device, so Auto can browse and play it while the app isn't running. Call it again whenever the
  library changes; Auto reloads the top level.
- Folders have no meaning of their own: they can be albums, artists, playlists, podcasts, or a single flat list.
- `playable: true` folders can be played as a whole, and picking an item inside one plays the folder as the queue,
  starting at that item (and records the folder for `folder` [tracked lists](syncing.md#tracked-lists)). Items in
  other folders play on their own.
- `style` (`"list"` or `"grid"`) sets how Auto shows a folder's children.
- `trackedList: "<list id>"` makes a folder show a [tracked list](syncing.md#tracked-lists) instead of `children`,
  for example a "Recently played" tab. It updates on its own as things play.
- `group` on items and folders: a header above each run of entries with the same group, for example `"A"`, `"B"`, …
  in an alphabetical list.
- `progress` on items (0 to 1): a mark for not played (`0`), partly played (with a progress bar) or played (`1`),
  for podcast episodes or audiobook chapters. Leave it out for no mark.
- `emptyMessage`: shown while `root` is empty, for example `"Open the app to sync your music"`.
- `setLibrary` rejects duplicate ids, folders without `id` / `title`, items without `src`, and folders that refer to
  unknown item ids.

## Search

Android Auto's search button and voice ("play … on <app>") work with the library, natively, also with the app closed.

- Typed search lists playable folders matching by title and items matching by title, artist or album, best matches
  first (exact, then "starts with", then "contains").
- By voice, the best match plays: a playlist or album as a whole, or a single item. "Play <app>" on its own continues
  the last queue, or plays the first playable folder.
- `setLibrary({ …, search: false })` hides the search button and turns voice play off (default `true`).

## Artwork

Android Auto can't read files inside your app, so the plugin serves local artwork to it, and to the system media
controls, through a content provider. It only serves images the plugin registered itself, under opaque keys.

```ts
await setLibrary({
  items: [{ id: 1, src: "/…/1.mp3", title: "Song 1", artworkUrl: "/…/covers/album.jpg" }],
  root: [/* … */],
  embeddedArtwork: true,   // default true
  folderArtwork: "collage" // "collage" (default) | "first" | "none"
});
```

- `artworkUrl` on items and folders can be a local path or `file://` URL (an image file), or an `https://` or
  `content://` URL, passed on as is.
- `embeddedArtwork` (default `true`): items without `artworkUrl` show the cover embedded in their audio file.
- `folderArtwork`: folders without `artworkUrl` (like playlists made in the app) show a 2x2 `collage` of covers from
  their items, one per album so an album's songs don't fill all four tiles (with fewer than 4 albums, the first
  cover), or their `first` cover, or `none`.
- Embedded covers and collages are made the first time Auto asks for them, and cached.
- Queue items' local `artworkUrl` is served the same way, for the media controls. On the player screen, the cover
  embedded in the playing file is shown when there's no `artworkUrl`.

## Designing your Android Auto app

Android Auto draws every screen itself from fixed templates, so no app can change its layout, colors or fonts.
Within that, the content is up to you:

| What | How |
| --- | --- |
| App name and icon in Auto's launcher | Your app's own name and icon |
| Tabs (up to 4) and what's in them | Top-level folders in `setLibrary` |
| Lists, grids, nesting | `style` per folder, folders inside folders |
| Titles, subtitles | `title`, `subtitle` on folders and items |
| Artwork | `artworkUrl`, embedded covers, folder collages, see [Artwork](#artwork) |
| What tapping does | `playable` folders play as a whole; items in them play the folder from there |
| Section headers in a list | `group` on items and folders |
| Played / partly played marks | `progress` on items, or recorded with [`trackProgress`](syncing.md#listening-progress) |
| Search and voice ("play … on <app>") | On by default; `search: false` turns it off |
| Message while there's no library yet | `emptyMessage` |
| Extra buttons on the player | [`setControls`](#extra-buttons) |
| Title / artist on the player | The playing item's `title` and `artist` |
| Sections that follow listening (recently played) | A folder with `trackedList`, see [Tracked lists](syncing.md#tracked-lists) |

The same calls fit very different apps. Some examples:

**Music app**

```ts
await setLibrary({
  items: songs, // { id, src, title, artist, album, artworkUrl }
  root: [
    { id: "recent", title: "Recently played", style: "grid", trackedList: "recentPlaylists" },
    {
      id: "playlists",
      title: "Playlists",
      style: "list",
      children: playlists.map((pl) => ({
        id: `pl-${pl.id}`,
        title: pl.name,
        subtitle: `${pl.songIds.length} songs · ${Math.round(pl.totalSeconds / 60)} min`,
        artworkUrl: pl.coverUrl,
        playable: true, // tap to play the playlist, or tap a song to play from there
        style: "list",
        children: pl.songIds,
      })),
    },
    { id: "albums", title: "Albums", style: "grid", children: albumFolders },
    { id: "artists", title: "Artists", style: "list", children: artistFolders }, // artist > albums > songs
  ],
});

await setControls({
  buttons: [
    { action: "shuffle" },
    { action: "repeat" },
    { action: "custom", id: "like", label: "Like", icon: "heart", toggle: true },
  ],
});
```

**Podcast app**

```ts
await setOptions({ trackProgress: true }); // played / partly played marks, continue where you stopped

await setLibrary({
  items: episodes,
  root: [
    { id: "continue", title: "Continue listening", children: inProgress.map((e) => e.id) }, // not playable: episodes play on their own
    {
      id: "shows",
      title: "Shows",
      style: "grid",
      children: shows.map((show) => ({
        id: `show-${show.id}`,
        title: show.name,
        subtitle: `${show.newEpisodes} new`,
        artworkUrl: show.coverUrl,
        style: "list",
        children: show.episodeIds, // newest first
      })),
    },
    { id: "downloads", title: "Downloads", children: downloadedIds },
  ],
});

await setControls({
  buttons: [
    { action: "seekBack", seconds: 15 },
    { action: "seekForward", seconds: 30 },
    { action: "speed", rates: [1, 1.25, 1.5, 2] },
    { action: "sleepTimer" },
    { action: "custom", id: "bookmark", label: "Bookmark", icon: "bookmark" },
  ],
});
```

**Audiobook app**: a "Library" grid of books, each a `playable` folder of chapters, so tapping a book plays it from
the start and tapping a chapter plays from there. Buttons: `seekBack` 30, `seekForward` 30 and `speed`.

Keep top-level tabs to 4 and lists reasonably short: Auto may limit how much it shows while driving.

## Extra buttons

Android Auto and the Android 13+ media controls can show extra buttons next to play / pause / previous / next.
Android only, with `carSupport` enabled; elsewhere these calls are accepted and do nothing.

```ts
import { setControls, onControlPressed, setControlActive } from "tauri-plugin-native-audio-api";

await setControls({
  buttons: [
    { action: "seekBack", seconds: 15 },
    { action: "seekForward", seconds: 30 },
    { action: "shuffle" },
    { action: "repeat" },
    { action: "speed", rates: [1, 1.5, 2] },
    { action: "sleepTimer", options: [15, 30, 60, "endOfTrack"] },
    { action: "custom", id: "like", label: "Like", icon: "heart", toggle: true },
  ],
  debounceMs: 500,
});

// Presses of custom buttons, including those made while the app wasn't running.
await onControlPressed(async ({ buttonId, itemId, active, pressedAtMs }) => {
  if (buttonId === "like" && itemId !== null) await saveLike(itemId, active);
});

// Keep toggle icons in line with the app's own data.
await setControlActive("like", likedSongIds, true);
```

- Built-in buttons act natively, also when the app isn't running: `seekBack` / `seekForward` (numbered icons for 5,
  10, 15 and 30 seconds, a plain arrow otherwise), `shuffle` and `repeat` (their icons show the current mode),
  `speed` (cycles through `rates`) and `sleepTimer` (cycles off, then each option in minutes or `"endOfTrack"`, then
  off; default 15, 30, 60, end of track; `fadeOutSeconds` default 10).
- Custom buttons are for the app. Pick an `icon` (`heart`, `star`, `thumbUp`, `thumbDown`, `bookmark`, `flag`,
  `checkCircle`, `plusCircle`, `minusCircle`, `plus`, `minus`, `playlistAdd`, `playlistRemove`, `queueAdd`,
  `queueNext`, `queueRemove`, `block`, `share`, `radio`, `album`, `artist`, `feed`, `settings`, `sync`, `quality`,
  `signal`), or set `iconResource` to a drawable in your app's Android resources.
- `toggle: true` keeps an on / off state per track: the icon shows filled while on, and each press reports the new
  state as `active`.
- Every custom press is saved on the device with the time, the track playing, and for toggles the new state, also
  when the app wasn't running. `onControlPressed` hands them to the app like the other [synced logs](syncing.md):
  first the saved ones, then live ones, each removed once the handler has finished. Handle presses in a way that's
  safe to repeat.
- Repeat presses of the same button within `debounceMs` (default 500) are ignored, so a single tap that arrives
  twice doesn't flip a toggle back.
- `getControlPresses()` / `acknowledgeControlPresses(ids)` are the lower-level versions of `onControlPressed`.
- The buttons are saved on the device, so they're there when Android Auto starts the app in the background.

## Good to know

- **Playing on connect:** Android Auto can start playback by itself when it connects (its own "auto-resume media"
  setting), whatever `resumeLastQueue` is set to. With that setting off, `resumeLastQueue: "play"` decides.
- **The controls after a pause:** see `keepAliveWhileCarConnected`, `pausedKeepAliveMinutes` and `keepQueueOnStop` in
  [Options](options.md).
- **With the app closed,** Auto plays the saved library and queue natively; tracked lists, the playback log, progress
  and button presses are kept and reach the app when it opens. The app can then bring the queue up to date with
  [`updateQueue`](playback.md#updating-the-queue-from-a-changed-playlist).
- **Without `carSupport`,** Auto shows the app in its generic player, which draws its own ±10 second buttons.
