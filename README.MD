# tauri-plugin-native-audio

Native audio playback for Tauri mobile apps: one JavaScript API over Media3 ExoPlayer on Android and AVPlayer on
iOS, built for real music and podcast apps. Playback, the queue and everything around it run natively, so they keep
working with the screen locked, in the background and in Android Auto, also while the app's WebView isn't running.

This is a fork of [tauri-plugin-native-audio](https://github.com/uvarov-frontend/tauri-plugin-native-audio) by
uvarov-frontend.

| Feature | Android | Android Auto | iOS¹ |
| --- | :---: | :---: | :---: |
| Background playback, lock screen and notification controls | ✓ | ✓ | ✓ |
| Native queue, shuffle, repeat, queue editing | ✓ | ✓ | ✓ |
| Resume the last queue | ✓ | ✓ | ✓ |
| Sleep timer | ✓ | ✓ | ✓ |
| Browsable library, search and voice | N/A | ✓ | N/A |
| Artwork for local files (embedded covers, playlist collages) | ✓ | ✓ | |
| Extra player buttons (like, seek, speed, sleep timer, …) | ✓² | ✓ | |
| Tracked lists, playback log, listening progress | ✓ | ✓ | ✓ |

¹ The iOS code is written but hasn't been compiled or tested in this fork.

² In the Android 13+ media controls, with `carSupport` enabled.

Desktop (Windows, macOS, Linux) isn't supported.

## What this fork adds

- **[Native queue](#queue).** The original plays one source at a time, so moving to the next track relies on
  JavaScript, which Android 12+ blocks in the background. Here the queue lives in the native player, so tracks keep
  advancing with the screen locked. Tested on Android 16 with targetSdk 37.
- **[Shuffle and repeat](#shuffle-and-repeat)** that play every track once per pass and keep recently played tracks
  away from the start of the next pass, plus **[queue editing](#editing-the-queue)** that keeps the shuffled order.
- **[Previous / next buttons](#previous--next-buttons)** that skip tracks (the original always seeks 10 seconds), or
  seek by an interval you choose.
- **[Sleep timer](#sleep-timer)** with a fade-out, or at the end of the track.
- **[Resume where you left off](#resuming-the-last-queue)**: queue, position, shuffle order and repeat mode.
- **[Android Auto](#android-auto)** (optional): a browsable library you provide, search, voice, artwork and your own
  player buttons.
- **[Syncing with your app](#syncing-with-your-app)**: button presses, tracked lists ("last 5 playlists played"),
  listening progress and a playback log, kept natively and handed to the app when it runs, also for what happened
  in the car with the app closed.

The plugin leaves the decisions to the app: every behavior above can be configured or turned off.

## Contents

- [Install](#install) · [Setting up](#setting-up) · [Configuration](#configuration) · [Getting started](#getting-started)
- Playback: [Queue](#queue) · [Shuffle and repeat](#shuffle-and-repeat) · [Previous / next buttons](#previous--next-buttons) · [Sleep timer](#sleep-timer) · [Resuming the last queue](#resuming-the-last-queue) · [Options](#options)
- [Android Auto](#android-auto): [Library](#the-library) · [Search](#search) · [Artwork](#artwork) · [Designing your Android Auto app](#designing-your-android-auto-app) · [Custom buttons](#custom-buttons)
- [Syncing with your app](#syncing-with-your-app): [Tracked lists](#tracked-lists) · [Listening progress](#listening-progress) · [Playback log](#playback-log)
- Reference: [JavaScript API](#javascript-api) · [Permissions](#permissions) · [Native progress checkpoint](#native-progress-checkpoint) · [Source handling](#source-handling) · [Troubleshooting](#troubleshooting) · [iOS runtime](#ios-runtime)

## Install

_This plugin requires Rust **1.77.2** or newer._

This fork isn't published to crates.io or npm, so install it straight from GitHub. If you're switching from the original plugin, remove it first. The package names are the same, so your existing imports and permission identifiers keep working.

Add the Rust plugin in `src-tauri/Cargo.toml`:

```toml
[dependencies]
tauri-plugin-native-audio = { git = "https://github.com/Sjims05/tauri-plugin-native-audio", tag = "v1.2.1" }
```

Add the JavaScript bindings:

```sh
pnpm add github:Sjims05/tauri-plugin-native-audio#v1.2.1
# or
npm install github:Sjims05/tauri-plugin-native-audio#v1.2.1
# or
yarn add github:Sjims05/tauri-plugin-native-audio#v1.2.1
```

Use the same tag for both, so the Rust and JS sides always match.

### Local development

To work on the plugin itself, clone the repo next to your app and use path dependencies:

```toml
tauri-plugin-native-audio = { path = "../../tauri-plugin-native-audio" }
```

```sh
npm install ../tauri-plugin-native-audio
```

## Setting up

### 1. Register the plugin in Rust

`src-tauri/src/lib.rs`

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_native_audio::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 2. Enable permissions in capability

Add plugin permissions to your capability file (example: `src-tauri/capabilities/default.json`):

```json
{
  "identifier": "default",
  "description": "Default capability",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "native-audio:default"
  ]
}
```

`native-audio:default` allows every command of the plugin (see [Permissions](#permissions) for the list).

If you need stricter ACL, use explicit allow/deny permissions from `permissions/autogenerated/commands/*.toml`.

### 3. Platform notes

Android:

- Requires Android 8.0+ (API 26).
- `play()` starts a foreground service (`NativeAudioService`) for media playback notification.
- On Android 13+ notification permission is requested during `initialize()`.
- With `carSupport`, Android Auto support adds an exported media library service, the Auto media app entry and the
  Android for Cars library (to tell when Android Auto is connected).
- An artwork content provider serves local artwork to Android Auto and the system media controls. It only serves
  images the plugin registered itself.
- Plugin manifest declares:
  - `android.permission.INTERNET`
  - `android.permission.FOREGROUND_SERVICE`
  - `android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK`
  - `android.permission.POST_NOTIFICATIONS`
  - `android.permission.WAKE_LOCK`

iOS:

- Requires iOS 14.0+.
- Uses `AVAudioSession` category `.playback`.
- For true background audio, enable Background Modes -> Audio in your iOS target.
- Lock screen controls and now playing metadata are integrated through `MediaPlayer` framework APIs.
- If your app target minimum is iOS 14.x, ensure `LD_RUNPATH_SEARCH_PATHS` includes `/usr/lib/swift` (along with `@executable_path/Frameworks`) to avoid launch-time Swift runtime loading issues.
- For iOS 16+ targets, this extra runpath setting is typically not required.

## Configuration

Optional `tauri.conf.json` settings:

```json
{
  "plugins": {
    "native-audio": {
      "carSupport": true
    }
  }
}
```

- `carSupport` (default `false`): Android Auto support, see [Android Auto](#android-auto). It's applied at build
  time, so changing it needs a rebuild.

Everything else is configured at runtime from JavaScript: [`setOptions`](#options), [`setSkipInterval`](#previous--next-buttons),
[`setControls`](#custom-buttons), [`setLibrary`](#the-library) and [`setTrackedLists`](#tracked-lists).

## Getting started

After plugin registration, APIs are available in JavaScript:

```ts
import {
  initialize,
  setSource,
  play,
  pause,
  seekTo,
  setRate,
  getState,
  getProgressCheckpoint,
  dispose,
  addStateListener
} from "tauri-plugin-native-audio-api";

const unlisten = await addStateListener((state) => {
  console.log("native-audio state", state);
});

await initialize();
await setSource({
  src: "https://example.com/audio.mp3",
  id: 123,
  title: "Episode 1",
  artist: "My Podcast",
  artworkUrl: "https://example.com/cover.jpg"
});
await setRate(1.25);
await play();

// later
await seekTo(42);
await pause();
console.log(await getState());
console.log(await getProgressCheckpoint());

unlisten();
await dispose();
```

## Queue

`setQueue` hands the whole playlist to the native player, so tracks keep advancing while the screen is locked
and the WebView is suspended. By default the previous / next buttons in the Android notification, the iOS lock
screen and Control Center, and headset / Bluetooth controls all move through this queue
(see [Previous / next buttons](#previous--next-buttons) to make them seek instead).

```ts
import { setQueue, play, next, previous, skipTo } from "tauri-plugin-native-audio-api";

await setQueue({
  items: [
    { src: "https://example.com/1.mp3", id: 1, title: "Song 1", artist: "Artist" },
    { src: "https://example.com/2.mp3", id: 2, title: "Song 2", artist: "Artist" },
    { src: "https://example.com/3.mp3", id: 3, title: "Song 3", artist: "Artist" }
  ],
  startIndex: 0
});
await play();

await next();      // go to the next track (no-op on the last track)
await previous();  // restart the current track if more than 3s in, otherwise go to the previous track
await skipTo(2);   // jump to a specific index
```

- The player advances to the next item automatically when a track ends; `status` becomes `ended` only after the last item
  (unless repeat is on, see below).
- `next` / `previous` / `skipTo` keep the current play/pause state.
- `setSource` is equivalent to a queue with a single item.
- Track changes are reported through `queueIndex`, `queueLength` and `currentId` in `NativeAudioState`.

### Editing the queue

```ts
import { addToQueue, removeFromQueue, moveInQueue, getQueue } from "tauri-plugin-native-audio-api";

await addToQueue([{ src: "…/4.mp3", id: 4, title: "Song 4" }]);                     // at the end
await addToQueue([{ src: "…/5.mp3", id: 5, title: "Song 5" }], { playNext: true }); // right after the current track
await removeFromQueue(2);  // removing the current track moves on to the next one
await moveInQueue(0, 3);

const { items, currentIndex, playOrder } = await getQueue();
```

- Indices are queue (list) indices everywhere: `queueIndex`, `skipTo`, `removeFromQueue`, `moveInQueue` and
  `getQueue().items`.
- `getQueue().playOrder` lists the queue indices in the order they play. Without shuffle it's `0, 1, 2, …`; with
  shuffle it's the shuffled order, so an "up next" list can follow it.
- With shuffle on, `addToQueue` puts the new tracks at the matching spot in the shuffled order (next, or at the
  end of the pass), and the rest of the order stays as it was. `moveInQueue` then only changes list positions,
  not the play order.
- `addToQueue` on an empty queue sets it as the queue.

## Shuffle and repeat

```ts
import { setShuffle, setRepeatMode } from "tauri-plugin-native-audio-api";

await setShuffle(true);
await setRepeatMode("all"); // "off" (default) | "all" | "one"
```

Both are handled natively, so they keep working while the screen is locked. Both are off by default, can be
set before or after `setQueue`, and are kept for the rest of the app session (also across `setQueue` and
`dispose()`). The current values are reported as `shuffle` and `repeatMode` in `NativeAudioState`.

Shuffle:

- Plays the queue in a shuffled order. Every track plays once per pass, so a track you just heard can't come
  back until the rest of the queue has played.
- Turning shuffle on makes a new order that starts with the current track, so the whole rest of the queue follows.
  Turning it off continues from the current track in queue order.
- `setQueue` with shuffle on starts with `startIndex` and shuffles the rest after it. Pass a random `startIndex`
  to also start on a random track.
- `skipTo(index)` with shuffle on plays that track and shuffles the rest of the queue after it.
- Where possible, two tracks by the same `artist` don't play back to back.
- `queueIndex` / `currentId` always refer to the position in `items`, not the shuffled position.

Repeat:

- `off`: playback stops after the last track (`status` becomes `ended`).
- `all`: after the last track, the queue starts over. With shuffle on, every pass gets a new order, and the tracks
  from the end of the previous pass (the last ~20% of the queue) are kept out of the start of the next one.
  `next()` / `previous()` also wrap around between the last and first track.
- `one`: the current track repeats when it ends. `next()` / `previous()` and the next / previous buttons still
  change track.

## Previous / next buttons

`setSkipInterval(seconds)` sets what the previous / next buttons in the Android notification and lock screen,
the iOS lock screen and Control Center, and headset / Bluetooth controls do:

- `0` (the default): skip whole tracks in the queue, like `previous()` / `next()`. You don't need to call
  `setSkipInterval` for this.
- Above `0`: seek back / forward by that many seconds within the current track. On Android the buttons keep
  their previous / next icons. On iOS the lock screen shows skip-interval buttons instead (for example "15").

Android Auto: with `carSupport`, Auto's player uses these buttons too. Without it, Auto shows the app in its generic
player, which always draws its own ±10 second buttons. For seek buttons with their own icons, use the `seekBack` /
`seekForward` [custom buttons](#custom-buttons).

```ts
import { setSkipInterval } from "tauri-plugin-native-audio-api";

await setSkipInterval(15); // buttons seek 15s back / forward (podcasts, audiobooks)
await setSkipInterval(0);  // buttons skip whole tracks again
```

- The value can be changed at any time, and is saved on the device.
- It only affects the system buttons. The JS `next()` / `previous()` commands always change track.

## Sleep timer

```ts
import { setSleepTimer, cancelSleepTimer } from "tauri-plugin-native-audio-api";

await setSleepTimer({ minutes: 30 });                     // pause in 30 minutes, fading out over the last 10 s
await setSleepTimer({ minutes: 45, fadeOutSeconds: 30 }); // a longer fade
await setSleepTimer({ endOfTrack: true });                // pause when the current track ends
await cancelSleepTimer();
```

- Runs natively, so it also works with the screen locked or while playing from Android Auto.
- A new timer replaces a running one. The state reports it as `sleepTimerEndsAtMs` (when it pauses) or
  `sleepTimerEndOfTrack`.
- The fade-out lowers the volume over the last `fadeOutSeconds` (default 10, `0` for none), then pauses and puts the
  volume back.
- `endOfTrack` pauses when the current track ends, with the next track ready, so play continues the queue.
- Pausing yourself doesn't cancel the timer; it just has nothing left to do when it runs out.
- A `sleepTimer` [button](#custom-buttons) puts it on the player, cycling through options you choose.

## Resuming the last queue

The playing queue is saved on the device as it plays and changes: the tracks, the current track and position
(about once a second, and right away on pause and track changes), the shuffle order and the repeat mode.

- `restoreLastQueue()` loads it back, paused, so the app can continue where it stopped, for example at startup.
  It resolves with the new state, or `null` when nothing was saved. It's up to the app whether to call it.
  When a queue is already loaded, for example one playing from Android Auto while the app opens, it's left
  alone (it resolves with the current state), so it's safe to call every time the app starts.
- Android: when the audio service starts with nothing loaded, for example Android Auto connecting while the app
  was closed, or a headset play button, the last queue is loaded automatically so it's ready to continue.
- The shuffle order is restored exactly, so already played tracks of the current pass don't come back.
- `setOptions({ resumeLastQueue: false })` turns the automatic loading off (default on). The queue is still saved,
  and `restoreLastQueue()` still works.

## Options

```ts
import { setOptions } from "tauri-plugin-native-audio-api";

await setOptions({
  resumeLastQueue: true,      // Android: load the last queue automatically, see above (default true)
  repeatAddedTracks: true,    // default false
  pausedKeepAliveMinutes: 30,       // Android, default 30
  keepAliveWhileCarConnected: true, // Android with carSupport, default true
  trackProgress: false              // default false
});
```

- Options are saved on the device, so they also apply when Android Auto starts the app in the background.
  Only the options passed are changed.
- `repeatAddedTracks`: whether tracks added with `addToQueue` stay in the queue when it repeats (repeat mode
  `all`). By default (`false`) they play once and are dropped when the queue starts over.
- `pausedKeepAliveMinutes` (Android): how long the playback controls stay after a pause, in the notification, on
  the lock screen and on Android Auto's player, so it's easy to pick up again (for example after a conversation in
  the car). Meanwhile the notification can't be swiped away. Afterwards the app's audio service leaves the
  foreground and Android may remove the controls. `0` lets them go right away.
- `keepAliveWhileCarConnected` (Android, with `carSupport`): while the phone is connected to Android Auto, the
  controls stay after a pause with no time limit, so Android Auto's player stays too. When Android Auto
  disconnects, `pausedKeepAliveMinutes` starts counting. The connection is read from Android Auto itself
  (Android for Cars library), since media sessions can't tell a paused car connection from a closed one.
- `trackProgress`: remember how far each item played, see [Listening progress](#listening-progress).
- The skip interval (`setSkipInterval`) is saved on the device the same way.

## Android Auto

With `carSupport` enabled, the app shows up in Android Auto as a media app: Auto can browse a library you
provide and play from it, and uses the plugin's queue, controls and saved queue.

- The plugin's build step adds what Auto needs to the app's `AndroidManifest.xml` (an exported media
  library service and the Auto media app entry), and removes it again when `carSupport` is turned off.
- `setLibrary(library)` sets what Auto can browse. It's saved on the device, so Auto can browse and play it
  while the app isn't running; call it again whenever the library changes.
- To test a sideloaded build, enable Android Auto's developer settings and turn on **Unknown sources**.
- iOS: `setLibrary` is accepted but not used yet (CarPlay isn't supported).

### The library

```ts
import { setLibrary } from "tauri-plugin-native-audio-api";

await setLibrary({
  // Each playable item is defined once, then referenced by id from any number of folders.
  items: [
    { id: 1, src: "/…/1.mp3", title: "Song 1", artist: "Artist", album: "Album" },
    { id: 2, src: "/…/2.mp3", title: "Song 2", artist: "Artist", album: "Album" }
  ],
  // Top-level folders are Auto's tabs (it shows up to 4).
  root: [
    { id: "songs", title: "All songs", playable: true, style: "list", children: [1, 2] },
    {
      id: "playlists",
      title: "Playlists",
      children: [{ id: "pl-1", title: "Road trip", playable: true, children: [2, 1] }]
    }
  ]
});
```

- The plugin gives folders no meaning of their own: they can be albums, artists, playlists, podcasts, or a
  single flat list.
- `playable: true` folders can be played as a whole, and picking an item inside one plays the folder as the
  queue, starting at that item. Items in other folders play on their own.
- `style` (`"list"` or `"grid"`) sets how Auto shows a folder's children.
- `trackedList: "<list id>"` makes a folder show a [tracked list](#tracked-lists) instead of `children`, for
  example a "Recently played" tab. It updates on its own as things play.
- `group` on items and folders: Auto shows it as a header above each run of entries with the same group, for
  example `"A"`, `"B"`, … in an alphabetical list.
- `progress` on items (0 to 1): Auto shows a mark for not played (`0`), partly played (with a progress bar) or
  played (`1`), e.g. for podcast episodes or audiobook chapters. Leave it out for no mark.
- `emptyMessage`: shown in Auto while `root` is empty, for example `"Open the app to sync your music"`.
- `artworkUrl` can be a local file; see [Artwork](#artwork) for that, embedded covers and folder collages.
- `setLibrary` rejects duplicate ids, folders without `id` / `title`, items without `src`, and folders that
  refer to unknown item ids.

### Search

Android Auto's search button and voice ("play … on <app>") work with the library, natively, also with the app closed.

- Typed search lists playable folders matching by title and items matching by title, artist or album, best matches
  first (exact, then "starts with", then "contains").
- By voice, the best match plays: a playlist or album as a whole, or a single item. "Play <app>" on its own
  continues the last queue, or plays the first playable folder.
- `setLibrary({ …, search: false })` hides the search button and turns voice play off (default `true`).

### Artwork

Android Auto can't read files inside your app, so the plugin serves local artwork to it, and to the system media
controls, through a content provider. It only serves images the plugin registered itself, under opaque keys.

```ts
await setLibrary({
  items: [{ id: 1, src: "/…/1.mp3", title: "Song 1", artworkUrl: "/…/covers/album.jpg" }],
  root: [/* … */],
  embeddedArtwork: true,     // default true
  folderArtwork: "collage"   // "collage" (default) | "first" | "none"
});
```

- `artworkUrl` on items and folders can be a local path or `file://` URL (an image file), or an `https://` or
  `content://` URL, which is passed on as is.
- `embeddedArtwork` (default `true`): items without `artworkUrl` show the cover embedded in their audio file.
- `folderArtwork`: folders without `artworkUrl` (like playlists made in the app) show a 2x2 `collage` of covers
  from their items, one per album so an album's songs don't fill all four tiles (with fewer than 4 albums, the
  first cover), or their `first` cover, or `none`.
- Embedded covers and collages are made the first time Android Auto asks for them, and cached.
- Queue items' local `artworkUrl` is served the same way, for the media controls. On the player screen, the cover
  embedded in the playing file is shown when there's no `artworkUrl`.

### Designing your Android Auto app

Android Auto draws every screen itself from fixed templates, so no app can change its layout, colors or fonts.
Within that, the plugin leaves the content up to you:

| What | How |
| --- | --- |
| App name and icon in Auto's launcher | Your app's own name and icon |
| Tabs (up to 4) and what's in them | Top-level folders in `setLibrary` |
| Lists, grids, nesting | `style` per folder, folders inside folders |
| Titles, subtitles | `title`, `subtitle` on folders and items |
| Artwork | `artworkUrl`, embedded covers, folder collages, see [Artwork](#artwork) |
| What tapping does | `playable` folders play as a whole; items in them play the folder from there |
| Section headers in a list | `group` on items and folders |
| Played / partly played marks | `progress` on items, or recorded with [`trackProgress`](#listening-progress) |
| Search and voice ("play … on <app>") | On by default; `search: false` turns it off |
| Message while there's no library yet | `emptyMessage` |
| Extra buttons on the player | `setControls` |
| Title / artist on the player | The playing item's `title` and `artist` |
| Sections that follow listening (recently played) | A folder with `trackedList`, see [Tracked lists](#tracked-lists) |

The plugin gives folders no meaning, so the same calls fit very different apps. Some examples:

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
        playable: true,          // tap to play the playlist, or tap a song to play from there
        style: "list",
        children: pl.songIds
      }))
    },
    { id: "albums", title: "Albums", style: "grid", children: albumFolders },
    { id: "artists", title: "Artists", style: "list", children: artistFolders } // artist > albums > songs
  ]
});

await setControls({
  buttons: [
    { action: "shuffle" },
    { action: "repeat" },
    { action: "custom", id: "like", label: "Like", icon: "heart", toggle: true }
  ]
});
```

**Podcast app**

```ts
await setOptions({ trackProgress: true }); // played / partly played marks, continue where you stopped

await setLibrary({
  items: episodes,
  root: [
    {
      id: "continue",
      title: "Continue listening",
      children: inProgress.map((e) => e.id) // episodes play on their own: the folder isn't playable
    },
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
        children: show.episodeIds // newest first
      }))
    },
    { id: "downloads", title: "Downloads", children: downloadedIds }
  ]
});

await setControls({
  buttons: [
    { action: "seekBack", seconds: 15 },
    { action: "seekForward", seconds: 30 },
    { action: "speed", rates: [1, 1.25, 1.5, 2] },
    { action: "sleepTimer" },
    { action: "custom", id: "bookmark", label: "Bookmark", icon: "bookmark" }
  ]
});
```

**Audiobook app**: a "Library" grid of books, each a `playable` folder of chapters, so tapping a book plays it from
the start and tapping a chapter plays from there. Buttons: `seekBack` 30, `seekForward` 30 and `speed`.

Notes:

- Keep top-level tabs to 4 and lists reasonably short: Auto may limit how much it shows while driving.
- Call `setLibrary` again whenever the content changes; Auto reloads the top level.

## Custom buttons

Android Auto and the Android 13+ media controls can show extra buttons next to play / pause / previous / next.

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
    { action: "custom", id: "like", label: "Like", icon: "heart", toggle: true }
  ],
  debounceMs: 500
});

// Presses of custom buttons, including those made while the app wasn't running.
await onControlPressed(async ({ buttonId, itemId, active, pressedAtMs }) => {
  if (buttonId === "like" && itemId !== null) await saveLike(itemId, active);
});

// Keep toggle icons in line with the app's own data.
await setControlActive("like", likedSongIds, true);
```

- Android only, with `carSupport` enabled. On iOS these calls are accepted and do nothing.
- Built-in buttons act natively, also when the app isn't running: `seekBack` / `seekForward` (media3 has numbered
  icons for 5, 10, 15 and 30 seconds, other values get a plain arrow), `shuffle` and `repeat` (their icons show the
  current mode), `speed` (cycles through `rates`) and `sleepTimer` (cycles off, then each option in minutes or
  `"endOfTrack"`, then off; default 15, 30, 60, end of track; `fadeOutSeconds` default 10).
- Custom buttons are for the app. Pick an `icon` from the built-in set (`heart`, `star`, `thumbUp`, `thumbDown`,
  `bookmark`, `flag`, `checkCircle`, `plusCircle`, `minusCircle`, `plus`, `minus`, `playlistAdd`,
  `playlistRemove`, `queueAdd`, `queueNext`, `queueRemove`, `block`, `share`, `radio`, `album`, `artist`,
  `feed`, `settings`, `sync`, `quality`, `signal`), or set `iconResource` to a drawable in your app's Android
  resources.
- `toggle: true` keeps an on / off state per track: the icon shows filled while on, and each press reports the
  new state as `active`.
- Every custom press is saved on the device with the time it was pressed, the track playing, and for toggles the
  new state, even when the app wasn't running. `onControlPressed` hands them to the app in order: first the saved
  ones, then live ones, and removes each from the log once the handler has finished. If the app closes before
  that, the press is delivered again next time, so handle presses in a way that's safe to repeat.
- Repeat presses of the same button within `debounceMs` (default 500) are ignored, so a single tap that arrives
  twice doesn't flip a toggle back. `pressedAtMs` is there for any extra checks.
- `getControlPresses()` and `acknowledgeControlPresses(ids)` are the lower-level versions of `onControlPressed`.
- The buttons are saved on the device, so they're there when Android Auto starts the app in the background.

## Syncing with your app

The plugin keeps several things natively, so they're right even when they happen while the app isn't running, for
example in Android Auto with the phone in a pocket. They all reach the app the same way:

- Saved on the device until the app has handled them.
- `on…` helpers hand them to the app in order: first what was saved while it wasn't running, then live changes.
- Each one is removed only after the app's handler has finished. If the app closes before that, it's delivered
  again next time, so handle them in a way that's safe to repeat.

[Custom button presses](#custom-buttons) work this way, and so do these:

### Tracked lists

Lists you declare, kept up to date by the plugin itself: "the last 5 playlists played", "the last 20 songs".
They keep updating while the app isn't running (for example in Android Auto) and sync to the app when it opens,
the same way as custom button presses.

```ts
import { setTrackedLists, onTrackedListChanged, setQueue } from "tauri-plugin-native-audio-api";

await setTrackedLists([
  { id: "recentPlaylists", track: "folder", limit: 5 },
  { id: "recentSongs", track: "item", limit: 20, countAfterSeconds: 30 }
]);

// Changes, including those made while the app was closed.
await onTrackedListChanged(async ({ listId, entries }) => {
  await db.saveRecent(listId, entries); // the whole list, newest first: [{ id, playedAtMs }]
});

// For "folder" lists, say which playlist a queue plays when the app starts it.
await setQueue({ items: playlistSongs, sourceId: "pl-7" });
```

And to show one in Android Auto, give a library folder `trackedList` instead of `children`:

```ts
root: [{ id: "recent", title: "Recently played", style: "grid", trackedList: "recentPlaylists" }, …]
```

- `track: "item"` remembers tracks (by `id`). `track: "folder"` remembers the playable folder a queue was started
  from: a playlist or album picked in Android Auto, or the `sourceId` the app passes to `setQueue`. Queues without
  one aren't recorded in folder lists.
- Most recent first, no duplicates (playing something again moves it to the front), at most `limit` entries
  (default 10).
- `countAfterSeconds` (default 0): how long something has to play before it counts, so a quick skip doesn't.
  A track that finishes always counts.
- The lists, and what each list counts, are saved on the device. Calling `setTrackedLists` again changes the
  definitions; lists that are no longer declared are dropped.
- Only the latest change per list waits for the app, since the app needs the current list rather than every step.
  Changes are delivered like button presses: removed once the handler has finished, delivered again next time if
  the app closes before that.
- Each entry is `{ id, playedAtMs }`: what was played and when it was last played. Changes also carry `ids` as a
  shortcut.
- `getTrackedList(id)` reads a list. `setTrackedList(id, entries)` replaces one, for example to clear it.
  `setTrackedList(id, entries, { merge: true })` combines a version from elsewhere with the current one: one entry
  per id with the newest `playedAtMs`, newest first, capped at the limit. Both resolve with the resulting list and
  aren't reported back as a change.
- `getTrackedListChanges()` and `acknowledgeTrackedListChanges(ids)` are the lower-level versions of
  `onTrackedListChanged`.
- In a `trackedList` folder, folder ids show as those folders (they need to exist in the library) and item ids as
  playable items. A `trackedList` folder can be `playable` too, to play its items as a queue.

#### Syncing a tracked list between devices

Because entries carry their play time, versions of a list kept in different places merge without losing plays,
whichever order they arrive in. For example with a shared copy on a server or network share:

```ts
// When the app opens or syncs: fold in the shared copy, and write the merged result back.
const shared = await readSharedCopy("recentPlaylists");          // [{ id, playedAtMs }]
const merged = await setTrackedList("recentPlaylists", shared, { merge: true });
await writeSharedCopy("recentPlaylists", merged);

// And whenever the list changes on this device (also from plays while the app was closed):
await onTrackedListChanged(async ({ listId, entries }) => {
  const shared = await readSharedCopy(listId);
  const merged = await setTrackedList(listId, shared, { merge: true });
  await writeSharedCopy(listId, merged);
});
```

### Listening progress

Remembers how far each item played, for podcasts and audiobooks. Off by default.

```ts
import { setOptions, getItemProgress, setItemProgress } from "tauri-plugin-native-audio-api";

await setOptions({ trackProgress: true });

const progress = await getItemProgress([12, 13]); // [{ itemId, progress, positionMs, updatedAtMs }]

// "Mark as played" from the app, or fold in a synced copy (the newer version of each item wins):
await setItemProgress([{ itemId: 12, progress: 1, positionMs: 0, updatedAtMs: Date.now() }]);
const merged = await setItemProgress(sharedCopy, { merge: true });
```

- Recorded natively on pause, skip and finish, and every 15 seconds while playing. Only items with an `id`.
- `progress` is 0 to 1; from 98% on an item counts as played (`1`), like skipped credits at the end of an episode.
- Android Auto shows played / partly played marks from it (falling back to the library's own `progress`), and an item
  picked there that was partly played continues where it stopped.
- `getItemProgress()` without ids returns everything recorded. `setItemProgress` replaces the given items, or with
  `merge: true` keeps whichever version of each is newer (`updatedAtMs`), so it syncs between devices like
  [tracked lists](#syncing-a-tracked-list-between-devices). Both resolve with the result.
- There's no separate change event: the [playback log](#playback-log) tells the app when something was paused,
  skipped or finished.

### Playback log

A lower-level log of what played, for things tracked lists don't cover: play counts, listening history,
scrobbling. It's kept on the device, so playback while the app wasn't running (for example from Android Auto)
reaches the app the next time it opens.

```ts
import { onPlaybackEvent } from "tauri-plugin-native-audio-api";

await onPlaybackEvent(async (event) => {
  // event: { id, type: "start" | "complete" | "skip", itemId, positionMs, atMs }
  if (event.type === "complete") await db.addPlayCount(event.itemId, event.id);
});
```

- `start`: a track started playing (once per time it's played, not on every resume after a pause).
- `complete`: it played to the end. `skip`: it was left before the end, with `positionMs` where.
- Only tracks with an `id` are logged.
- `onPlaybackEvent` works like `onControlPressed`: first the saved events, in order, then live ones, each removed
  from the log once the handler has finished. If the app closes before that, the event is delivered again next
  time, so handle events in a way that's safe to repeat (for example keyed on `event.id`).
- The newest 500 events are kept until the app picks them up.
- `getPlaybackEvents()` and `acknowledgePlaybackEvents(ids)` are the lower-level versions of `onPlaybackEvent`.

## JavaScript API

Types:

```ts
export type NativeAudioStatus = "idle" | "loading" | "playing" | "ended" | "error";

export type NativeAudioState = {
  status: NativeAudioStatus;
  currentTime: number;
  duration: number;
  isPlaying: boolean;
  buffering: boolean;
  rate: number;
  queueIndex: number; // -1 when nothing is loaded
  queueLength: number;
  currentId?: number; // `id` of the current item, when one was given
  shuffle: boolean;
  repeatMode: NativeAudioRepeatMode;
  sleepTimerEndsAtMs: number | null; // when a running sleep timer pauses (epoch ms)
  sleepTimerEndOfTrack: boolean;     // a sleep timer that pauses at the end of the track
  error?: string;
};

export type NativeAudioRepeatMode = "off" | "all" | "one";

export type NativeAudioSetSourcePayload = {
  src: string;
  id?: number;
  title?: string;
  artist?: string;
  artworkUrl?: string;
};

export type NativeAudioQueueItem = NativeAudioSetSourcePayload;

export type NativeAudioSetQueuePayload = {
  items: NativeAudioQueueItem[];
  sourceId?: string;      // the playlist / album this queue plays, for "folder" tracked lists
  startIndex?: number;    // default 0
  startPosition?: number; // seconds into the start item, default 0
};

export type NativeAudioQueue = {
  items: NativeAudioQueueItem[]; // queue (list) order
  currentIndex: number;          // -1 when the queue is empty
  playOrder: number[];           // queue indices in the order they play
};

export type NativeAudioLibraryItem = {
  id: number;
  src: string;
  title?: string;
  artist?: string;
  album?: string;
  artworkUrl?: string;
  group?: string;                // header in Android Auto lists
  progress?: number;             // 0 to 1: not played / partly played / played
};

export type NativeAudioLibraryFolder = {
  id: string;                    // unique across all folders
  title: string;
  subtitle?: string;
  artworkUrl?: string;
  playable?: boolean;
  style?: "list" | "grid";
  children?: Array<NativeAudioLibraryFolder | number>; // sub-folders and item ids
  trackedList?: string;          // show a tracked list instead of children
  group?: string;                // header in Android Auto lists
};

export type NativeAudioTrackedListConfig = {
  id: string;
  track: "folder" | "item";
  limit?: number;                // default 10
  countAfterSeconds?: number;    // default 0
};

export type NativeAudioTrackedListEntry = {
  id: string | number;           // folder id or item id
  playedAtMs: number;
};

export type NativeAudioTrackedListChange = {
  id: string;
  listId: string;
  entries: NativeAudioTrackedListEntry[]; // the whole list, newest first
  ids: Array<string | number>;   // the entries' ids
  changedAtMs: number;
};

export type NativeAudioLibrary = {
  items: NativeAudioLibraryItem[];
  root: NativeAudioLibraryFolder[];
  search?: boolean;              // default true
  emptyMessage?: string;         // shown while root is empty
  embeddedArtwork?: boolean;     // default true
  folderArtwork?: "collage" | "first" | "none"; // default "collage"
};

export type NativeAudioOptions = {
  resumeLastQueue?: boolean;            // default true
  repeatAddedTracks?: boolean;          // default false
  pausedKeepAliveMinutes?: number;      // default 30
  keepAliveWhileCarConnected?: boolean; // default true
  trackProgress?: boolean;              // default false
};

export type NativeAudioSleepTimer =
  | { minutes: number; fadeOutSeconds?: number } // fadeOutSeconds default 10
  | { endOfTrack: true };

export type NativeAudioItemProgress = {
  itemId: number;
  progress: number;              // 0 to 1
  positionMs: number;
  updatedAtMs: number;
};

export type NativeAudioControls = {
  buttons: NativeAudioControlButton[];
  debounceMs?: number;           // default 500
};

export type NativeAudioControlButton =
  | { action: "seekBack" | "seekForward"; seconds: number }
  | { action: "shuffle" }
  | { action: "repeat" }
  | { action: "speed"; rates?: number[] }
  | { action: "sleepTimer"; options?: Array<number | "endOfTrack">; fadeOutSeconds?: number }
  | { action: "custom"; id: string; label: string; icon?: NativeAudioControlIcon; iconResource?: string; toggle?: boolean };

export type NativeAudioControlPress = {
  id: string;
  buttonId: string;
  itemId: number | null;         // the track playing when it was pressed
  active: boolean | null;        // new state of a toggle button
  pressedAtMs: number;
};

export type NativeAudioPlaybackEvent = {
  id: string;
  type: "start" | "complete" | "skip";
  itemId: number;
  positionMs: number;
  atMs: number;
};

export type NativeAudioProgressCheckpoint = {
  id: number;
  currentTime: number;
  updatedAtMs: number;
  status?: NativeAudioStatus;
};
```

Commands:

- `initialize(): Promise<NativeAudioState>`
- `setSource(payload: NativeAudioSetSourcePayload): Promise<NativeAudioState>`
- `setQueue(payload: NativeAudioSetQueuePayload): Promise<NativeAudioState>`
- `next(): Promise<NativeAudioState>`
- `previous(): Promise<NativeAudioState>`
- `skipTo(index: number): Promise<NativeAudioState>`
- `addToQueue(items: NativeAudioQueueItem[], options?: { playNext?: boolean }): Promise<NativeAudioState>`
- `removeFromQueue(index: number): Promise<NativeAudioState>`
- `moveInQueue(from: number, to: number): Promise<NativeAudioState>`
- `getQueue(): Promise<NativeAudioQueue>`
- `restoreLastQueue(): Promise<NativeAudioState | null>`
- `setLibrary(library: NativeAudioLibrary): Promise<void>`
- `setOptions(options: NativeAudioOptions): Promise<void>`
- `setControls(controls: NativeAudioControls): Promise<void>`
- `setControlActive(buttonId: string, itemIds: number[], active: boolean): Promise<void>`
- `onControlPressed(handler: (press: NativeAudioControlPress) => void | Promise<void>): Promise<() => void>`
- `getControlPresses(): Promise<NativeAudioControlPress[]>`
- `acknowledgeControlPresses(ids: string[]): Promise<void>`
- `onPlaybackEvent(handler: (event: NativeAudioPlaybackEvent) => void | Promise<void>): Promise<() => void>`
- `getPlaybackEvents(): Promise<NativeAudioPlaybackEvent[]>`
- `acknowledgePlaybackEvents(ids: string[]): Promise<void>`
- `setTrackedLists(lists: NativeAudioTrackedListConfig[]): Promise<void>`
- `onTrackedListChanged(handler: (change: NativeAudioTrackedListChange) => void | Promise<void>): Promise<() => void>`
- `getTrackedList(id: string): Promise<NativeAudioTrackedListEntry[]>`
- `setTrackedList(id: string, entries: NativeAudioTrackedListEntry[], options?: { merge?: boolean }): Promise<NativeAudioTrackedListEntry[]>`
- `getTrackedListChanges(): Promise<NativeAudioTrackedListChange[]>`
- `acknowledgeTrackedListChanges(ids: string[]): Promise<void>`
- `play(): Promise<NativeAudioState>`
- `pause(): Promise<NativeAudioState>`
- `seekTo(position: number): Promise<NativeAudioState>`
- `setRate(rate: number): Promise<NativeAudioState>`
- `setSkipInterval(seconds: number): Promise<NativeAudioState>`
- `setSleepTimer(timer: NativeAudioSleepTimer): Promise<NativeAudioState>`
- `cancelSleepTimer(): Promise<NativeAudioState>`
- `getItemProgress(itemIds?: number[]): Promise<NativeAudioItemProgress[]>`
- `setItemProgress(entries: NativeAudioItemProgress[], options?: { merge?: boolean }): Promise<NativeAudioItemProgress[]>`
- `setShuffle(enabled: boolean): Promise<NativeAudioState>`
- `setRepeatMode(mode: NativeAudioRepeatMode): Promise<NativeAudioState>`
- `getState(): Promise<NativeAudioState>`
- `getProgressCheckpoint(): Promise<NativeAudioProgressCheckpoint | null>`
- `dispose(): Promise<void>`
- `addStateListener(handler): Promise<() => void>`

Validation and errors:

- `setSource` rejects if `src` is empty.
- `setQueue` rejects if `items` is empty, an item has an empty `src`, or `startIndex` is out of range.
- `skipTo` rejects if `index` is out of range.
- `addToQueue` rejects if `items` is empty or an item has an empty `src`.
- `removeFromQueue` / `moveInQueue` reject if an index is out of range.
- `seekTo` rejects if `position` is missing/non-finite.
- `setRate` rejects if `rate <= 0` or non-finite.
- `setSkipInterval` rejects if `seconds < 0` or non-finite.
- `setShuffle` rejects if `enabled` is missing.
- `setRepeatMode` rejects if `mode` is not `off`, `all` or `one`.
- `setSleepTimer` rejects without `minutes > 0` or `endOfTrack: true`, or with `fadeOutSeconds < 0`.
- `setLibrary`, `setControls`, `setTrackedLists` and `setItemProgress` reject invalid input with a message saying what's wrong.
- Native failures can also be reflected in `NativeAudioState.error` with status `error`.

State semantics:

- `idle`: player idle or paused
- `loading`: buffering
- `playing`: active playback
- `ended`: reached the end of the last track (never with repeat `all` / `one`)
- `error`: runtime/player error

## Permissions

`native-audio:default` allows all of these. For a stricter setup, list the ones you use, or use explicit allow /
deny permissions from `permissions/autogenerated/commands/*.toml`.

- `allow-initialize`
- `allow-register-listener`
- `allow-remove-listener`
- `allow-set-source`
- `allow-set-queue`
- `allow-next`
- `allow-previous`
- `allow-skip-to`
- `allow-add-to-queue`
- `allow-remove-from-queue`
- `allow-move-in-queue`
- `allow-get-queue`
- `allow-restore-last-queue`
- `allow-play`
- `allow-pause`
- `allow-seek-to`
- `allow-set-rate`
- `allow-set-skip-interval`
- `allow-set-shuffle`
- `allow-set-repeat-mode`
- `allow-set-library`
- `allow-set-options`
- `allow-set-controls`
- `allow-set-control-active`
- `allow-get-control-presses`
- `allow-acknowledge-control-presses`
- `allow-get-playback-events`
- `allow-acknowledge-playback-events`
- `allow-set-tracked-lists`
- `allow-get-tracked-list`
- `allow-set-tracked-list`
- `allow-get-tracked-list-changes`
- `allow-acknowledge-tracked-list-changes`
- `allow-get-state`
- `allow-get-progress-checkpoint`
- `allow-dispose`
- `allow-set-sleep-timer`
- `allow-cancel-sleep-timer`
- `allow-get-item-progress`
- `allow-set-item-progress`

## Native progress checkpoint

The plugin can persist a lightweight playback checkpoint natively (Android/iOS) for background playback scenarios where the WebView/JS runtime may be suspended.

Checkpoint shape:

- `id`
- `currentTime`
- `updatedAtMs`
- `status` (optional)

How it works:

- Pass `id` in `setSource(...)` (or on each `setQueue` item) to associate playback with an app-level item.
- During playback, the plugin stores a single checkpoint (v1) in native storage:
  - Android: `SharedPreferences`
  - iOS: `UserDefaults`
- Checkpoint updates are deterministic: on pause, successful seek commit, end, dispose, and throttled play ticks.
- Near-start micro progress (<= 0.25s) is ignored to avoid noisy checkpoints.

Read the last checkpoint:

```ts
const checkpoint = await getProgressCheckpoint();
if (checkpoint) {
  console.log(checkpoint.id, checkpoint.currentTime, checkpoint.updatedAtMs);
}
```

Notes:

- The plugin stores only audio progress checkpoint data, not your app's story cache.
- v1 stores only the latest active story checkpoint (single record).

## Source handling

Supported source forms:

- remote URL (`https://...`, and platform-dependent `http://...`)
- local file URL (`file://...`)
- local file path

iOS extras:

- `asset://localhost/...` and `http://asset.localhost/...` resolve to local file paths.
- Local files with missing/`.bin` extension are inspected by header and remapped via temporary alias (`mp3`, `wav`, `m4a`, `ogg`) for better AVPlayer compatibility.

## Troubleshooting

`unknown command plugin:native-audio|...`

- Ensure Rust registration exists: `.plugin(tauri_plugin_native_audio::init())`
- Ensure capability includes `native-audio:default` (or explicit allow permissions)

Android notification controls not shown:

- Call `initialize()` before playback
- Start playback using `play()`
- Grant notifications permission on Android 13+

iOS background playback not working:

- Enable Background Modes -> Audio for the iOS app target
- Verify stream URL is reachable and valid

iOS HTTP URL fails:

- ATS can block plain HTTP
- Prefer HTTPS or configure ATS exceptions in app settings

iOS app closes right after launch on iOS 14:

- Ensure app target `LD_RUNPATH_SEARCH_PATHS` includes both `@executable_path/Frameworks` and `/usr/lib/swift`
- Clean/reinstall the app after changing build settings

## iOS runtime

iOS playback runs through a single actor-owned runtime.

- Playback commands (queue, playback, shuffle / repeat, sleep timer and the rest) are serialized in one state owner.
- `setSource` and `seekTo` use revision fencing (`sourceRevision`, `seekRevision`) to ignore stale async callbacks.
- Seek is processed as a deterministic two-phase flow (`pending -> committed`).
- Native state is emitted through `native_audio_state` with the `NativeAudioState` payload.

## Notice

This plugin was fully developed with the help of AI.

## License

MIT or Apache-2.0, where applicable.
