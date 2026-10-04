# Playback and the queue

[← README](../README.MD) · [Options](options.md) · [API](api.md)

- [The queue](#the-queue)
- [Editing the queue](#editing-the-queue)
- [Updating the queue from a changed playlist](#updating-the-queue-from-a-changed-playlist)
- [Shuffle and repeat](#shuffle-and-repeat)
- [Previous and next](#previous-and-next)
- [Sleep timer](#sleep-timer)
- [Volume](#volume)
- [Resuming the last queue](#resuming-the-last-queue)

Everything here works the same on Android and desktop, with the same rules; where a platform differs, it says so.

## The queue

`setQueue` hands the whole playlist to the native player, so tracks keep advancing while the screen is locked and
the WebView is suspended. The system's previous / next buttons (notification, lock screen, headset, Android Auto,
desktop media keys) move through this queue too.

```ts
import { setQueue, play, next, previous, skipTo } from "tauri-plugin-native-audio-api";

await setQueue({
  items: [
    { src: "/music/1.flac", id: 1, title: "Song 1", artist: "Artist" },
    { src: "/music/2.flac", id: 2, title: "Song 2", artist: "Artist" },
    { src: "/music/3.flac", id: 3, title: "Song 3", artist: "Artist" },
  ],
  startIndex: 0,
  sourceId: "playlist-7", // optional: the playlist / album this queue plays (for tracked lists)
});
await play();

await next();     // the next track (on the last one: see nextAtEnd)
await previous(); // restart the track if it's past 3 s, otherwise the previous track
await skipTo(2);  // a specific queue index
```

- The player moves to the next track by itself; `status` becomes `ended` only after the last one (unless repeat
  is on). Gapless: on desktop the next track's first sample follows the previous one's last; on Android ExoPlayer
  joins them.
- `next` / `previous` / `skipTo` keep the play / pause state.
- `setSource` is a queue with one item.
- Track changes show up in the state as `queueIndex`, `queueLength` and `currentId`.
- Give items an `id`: tracked lists, the playback log, progress and `updateQueue` all work by id.
- A track that can't be played (missing, broken file) is skipped and reported in the state's `error`; the status is
  `error` when nothing in the queue could be played. (Android stops at the broken track with status `error`.)

## Editing the queue

```ts
import { addToQueue, removeFromQueue, moveInQueue, getQueue } from "tauri-plugin-native-audio-api";

await addToQueue([{ src: "/music/4.flac", id: 4, title: "Song 4" }]);                      // at the end
await addToQueue([{ src: "/music/5.flac", id: 5, title: "Song 5" }], { playNext: true });  // after the current track
await removeFromQueue(2); // removing the playing track moves on to the next one
await moveInQueue(0, 3);

const { items, currentIndex, playOrder } = await getQueue();
```

- Indices are queue (list) indices everywhere: `queueIndex`, `skipTo`, `removeFromQueue`, `moveInQueue` and
  `getQueue().items`.
- `getQueue().playOrder` is the queue indices in the order they play: `0, 1, 2, …` without shuffle, the shuffled
  order with it, so an "up next" list can follow it.
- With shuffle on, added tracks get the matching spot in the shuffled order (next, or at the end of the pass) and
  the rest of the order stays. `moveInQueue` then only changes list positions; without shuffle it changes what
  plays next.
- `addToQueue` on an empty queue makes it the queue. Tracks added to a queue are marked as added: see
  `repeatAddedTracks` in [Options](options.md).

## Updating the queue from a changed playlist

When the playlist (or album) the queue plays changes, for example after a sync, `updateQueue` applies the change
without interrupting what's playing:

```ts
import { updateQueue } from "tauri-plugin-native-audio-api";

const { state, added, removed } = await updateQueue(playlistSongs, {
  removedCurrent: "finish", // the playing track left the playlist: let it finish ("skip": move on now)
  removedItems: "remove",   // tracks no longer in the playlist ("keep": leave them in the queue)
  newItems: "inPlace",      // new tracks: at their place in the playlist ("end", "next")
  order: "playlist",        // the list order: the playlist's new order ("queue": keep the queue's)
});
if (added || removed) showToast(`${added} new, ${removed} removed`);
```

- Tracks are matched by `id`; the same id twice is matched in order. Tracks without an id count as new.
- What's still there keeps its place in the play order, also when shuffled.
- Tracks added with `addToQueue` always stay, after the track they followed.
- The playing track's details (title, artwork) are updated without interrupting it.
- With an empty queue, the items become the queue. `sourceId` updates the queue's playlist when given.
- `added` / `removed` count what changed in the playlist (removed also counts tracks that were kept or are playing
  to the end).

What the options do together:

| | Without shuffle | With shuffle |
| --- | --- | --- |
| `newItems: "inPlace"` | at their place in the playlist | somewhere in the rest of this pass |
| `newItems: "end"` | at the end | at the end of the play order, shuffled among themselves |
| `newItems: "next"` | right after the playing track | right after the playing track |
| `order: "playlist"` | the existing tracks follow the playlist's new order | the list follows the playlist; what plays when doesn't change |
| `order: "queue"` | the existing tracks keep their order; new ones placed by `newItems` ("in place": after the track before them in the playlist) | the same, and what plays when doesn't change |
| `removedCurrent` | no effect with `removedItems: "keep"` (the playing track is kept too) | the same |

For example, with the queue `A B C D` and the playlist changed to `D A N C` (`N` new, `B` removed), without shuffle:
`newItems: "inPlace"` gives `D A N C`, `"end"` gives `D A C N`, and `order: "queue"` with `"end"` gives `A C D N`.

## Shuffle and repeat

```ts
import { setShuffle, setRepeatMode } from "tauri-plugin-native-audio-api";

await setShuffle(true);
await setRepeatMode("all"); // "off" (default) | "all" | "one"
```

Both run natively, can be set before or after `setQueue`, and stay for the session (also across `setQueue` and
`dispose()`); a restored queue brings its own. The state reports them as `shuffle` and `repeatMode`.

Shuffle:

- Every track plays once per pass, so a track you just heard doesn't come back until the rest has played.
- Turning it on makes a new order that starts with the current track; turning it off continues from the current
  track in list order.
- `setQueue` with shuffle on starts with `startIndex` and shuffles the rest (pass a random `startIndex` to start
  on a random track). `skipTo` plays that track and shuffles the rest after it.
- Where possible, two tracks by the same `artist` don't play back to back.
- `queueIndex` / `currentId` always refer to the position in `items`, not the shuffled position.

Repeat:

- `off`: playback stops after the last track (`status` becomes `ended`).
- `all`: after the last track the queue starts over. With shuffle, each pass gets a new order, and the tracks from
  the end of the previous pass (the last ~20%) are kept away from the start of the next one. Tracks added with
  `addToQueue` are dropped at this point unless `repeatAddedTracks` is on. `next` / `previous` wrap around.
- `one`: the track repeats when it ends (gaplessly); `next` / `previous` still change track.

## Previous and next

The app's `previous()` / `next()` and the system buttons follow the same rules, set with [options](options.md):

- `previousRestartsAfterSeconds` (default 3): past this point previous restarts the track; before it, it goes to
  the previous track. `0`: previous always goes to the previous track.
- `nextAtEnd` (default `"nothing"`): next on the last track with repeat off does nothing, or with `"first"` goes
  back to the first track of the play order, paused. The next button then stays enabled on the last track.

`setSkipInterval(seconds)` changes what the **system** buttons do (notification, lock screen, headset, Android
Auto, desktop media keys and media panel):

- `0` (the default): previous / next track, as above.
- Above `0`: seek back / forward by that many seconds, for podcasts and audiobooks. On desktop the media panel's
  fast-forward / rewind seek by it too (10 s otherwise).

```ts
await setSkipInterval(15); // buttons seek 15 s
await setSkipInterval(0);  // buttons change track again
```

It's saved on the device and only affects the system buttons: the app's `next()` / `previous()` always change
track. In Android Auto without `carSupport`, Auto's generic player draws its own ±10 s buttons; for seek buttons
with their own icons, use the `seekBack` / `seekForward` [extra buttons](android-auto.md#extra-buttons).

## Sleep timer

```ts
import { setSleepTimer, cancelSleepTimer } from "tauri-plugin-native-audio-api";

await setSleepTimer({ minutes: 30 });                     // pause in 30 minutes, fading out over the last 10 s
await setSleepTimer({ minutes: 45, fadeOutSeconds: 30 }); // a longer fade (0: none)
await setSleepTimer({ endOfTrack: true });                // pause when the current track ends
await cancelSleepTimer();
```

- Runs natively, also with the screen locked or while playing from Android Auto. A new timer replaces a running
  one. The state reports it as `sleepTimerEndsAtMs` or `sleepTimerEndOfTrack`.
- The fade lowers the volume over the last `fadeOutSeconds`, then pauses and puts the volume back. Changing the
  volume during the fade continues the fade from the new volume.
- `endOfTrack` pauses exactly where the track ends, with the next one ready at its start, and then clears itself.
- Pausing yourself doesn't cancel the timer. On Android, a `sleepTimer` [button](android-auto.md#extra-buttons) puts
  it on the player.

## Volume

```ts
import { setVolume } from "tauri-plugin-native-audio-api";

await setVolume(0.5); // 0 to 1: the player's own volume, on top of the system volume
```

- The value is a slider's position; what's heard follows the `volumeCurve` [option](options.md). Hearing is
  roughly logarithmic, so the default `quadratic` curve makes 50% sound about half as loud.
- Saved, so the next run starts at the same volume. The state reports it as `volume`.
- Desktop: the system media controls' volume (MPRIS on Linux) sets it too. For the output device, see
  [Desktop](desktop.md#output-devices).

## Resuming the last queue

The queue is saved as it plays and changes: the tracks, the current track and position (about once a second, and
right away on pause and track changes), the shuffle order and the repeat mode.

- `restoreLastQueue()` loads it back, paused. It resolves with the state, or `null` when nothing was saved. When a
  queue is already loaded (one playing from Android Auto while the app opens, or one auto-resumed), it's left
  alone and resolves with the current state, so it's safe to call at every start.
- The `resumeLastQueue` [option](options.md) does it automatically: `"paused"` (default) loads it, `"play"` also
  plays it, `"off"` leaves it to the app.
  - Android: when the audio service starts with nothing loaded, such as Android Auto connecting or a headset
    button with the app closed. Opening the app doesn't start the service, so `"play"` doesn't play from the
    phone's speaker when the app opens.
  - Desktop: at the app's first `initialize()`. Call it once the UI is ready, so a resumed queue doesn't start
    playing before the window shows anything.
  - Android Auto can start playback by itself when it connects (its own "auto-resume media" setting), whatever
    this is set to: `"paused"` can't hold it back.
- The shuffle order is restored exactly, so tracks already played this pass don't come back.
- To keep a restored queue in line with a playlist that changed since, call [`updateQueue`](#updating-the-queue-from-a-changed-playlist)
  after restoring.
