# Syncing with your app

[← README](../README.MD) · [Playback](playback.md) · [API](api.md)

The plugin keeps a few things itself, so they stay right even when they happen while the app isn't running (on
Android, for example in Android Auto with the phone in a pocket). On desktop they work the same and are saved
between runs.

- [How they reach the app](#how-they-reach-the-app)
- [Tracked lists](#tracked-lists)
- [Listening progress](#listening-progress)
- [Playback log](#playback-log)
- [Progress checkpoint](#progress-checkpoint)

## How they reach the app

Tracked list changes, the playback log and [button presses](android-auto.md#extra-buttons) all work the same way:

- They're saved on the device until the app has handled them.
- The `on…` helpers hand them over in order: first what was saved while the app wasn't running, then live changes.
- Each one is removed only after the app's handler has finished. If the app closes before that, it's delivered
  again next time, so handle them in a way that's safe to repeat (for example keyed on the entry's `id`).

## Tracked lists

Lists you declare, kept up to date by the plugin: "the last 5 playlists played", "the last 20 songs".

```ts
import { setTrackedLists, onTrackedListChanged, setQueue } from "tauri-plugin-native-audio-api";

await setTrackedLists([
  { id: "recentPlaylists", track: "folder", limit: 5 },
  { id: "recentSongs", track: "item", limit: 20, countAfterSeconds: 30 },
]);

// Changes, including those from while the app was closed.
await onTrackedListChanged(async ({ listId, entries }) => {
  await db.saveRecent(listId, entries); // the whole list, newest first: [{ id, playedAtMs }]
});

// For "folder" lists, say which playlist a queue plays.
await setQueue({ items: playlistSongs, sourceId: "pl-7" });
```

- `track: "item"` remembers tracks (by `id`). `track: "folder"` remembers the playlist / album a queue was started
  from: one picked in Android Auto, or the `sourceId` given to `setQueue` (or `updateQueue`). Queues without one
  aren't recorded in folder lists.
- Most recent first, no duplicates (playing something again moves it to the front), at most `limit` (default 10).
- `countAfterSeconds` (default 0): how long something has to play before it counts, so a quick skip doesn't. A track
  that finishes always counts. Each list counts a play once.
- The definitions and the lists are saved. Calling `setTrackedLists` again changes the definitions; lists no longer
  declared are dropped, and shorter limits apply right away.
- Only the latest change per list waits for the app, since the app needs the current list rather than every step.
  Changes carry `entries` and, as a shortcut, `ids`.
- `getTrackedList(id)` reads a list. `setTrackedList(id, entries)` replaces it (for example to clear it), and with
  `{ merge: true }` combines it with the current one. Both resolve with the result and aren't reported back as a
  change.
- `getTrackedListChanges()` / `acknowledgeTrackedListChanges(ids)` are the lower-level versions of
  `onTrackedListChanged`.
- To show a list in Android Auto, give a library folder `trackedList` instead of `children`: folder ids show as
  those folders (they need to exist in the library), item ids as playable items.

### Syncing a list between devices

Entries carry when they were last played, so versions of a list kept in different places merge without losing
plays, whichever order they arrive in: one entry per id with the newest `playedAtMs`, newest first, capped at the
limit. For example with a shared copy on a server or network share:

```ts
// When the app opens or syncs: fold in the shared copy, and write the merged result back.
const shared = await readSharedCopy("recentPlaylists"); // [{ id, playedAtMs }]
const merged = await setTrackedList("recentPlaylists", shared, { merge: true });
await writeSharedCopy("recentPlaylists", merged);

// And whenever the list changes on this device (also from plays while the app was closed):
await onTrackedListChanged(async ({ listId }) => {
  const merged = await setTrackedList(listId, await readSharedCopy(listId), { merge: true });
  await writeSharedCopy(listId, merged);
});
```

## Listening progress

Remembers how far each item played, for podcasts and audiobooks. Off by default.

```ts
import { setOptions, getItemProgress, setItemProgress } from "tauri-plugin-native-audio-api";

await setOptions({ trackProgress: true });

const progress = await getItemProgress([12, 13]); // [{ itemId, progress, positionMs, updatedAtMs }]

// "Mark as played" from the app, or fold in a synced copy (the newer version of each item wins):
await setItemProgress([{ itemId: 12, progress: 1, positionMs: 0, updatedAtMs: Date.now() }]);
const merged = await setItemProgress(sharedCopy, { merge: true });
```

- Recorded on pause, skip and finish, and every 15 seconds while playing. Only items with an `id`.
- `progress` is 0 to 1; from 98% on an item counts as played (`1`), like skipped credits at the end of an episode.
- Android Auto shows played / partly played marks from it, and an item picked there that was partly played continues
  where it stopped.
- `getItemProgress()` without ids returns everything. `setItemProgress` replaces the given items, or with
  `merge: true` keeps whichever version of each is newer (`updatedAtMs`), so it syncs between devices like tracked
  lists. Both resolve with the result.
- There's no separate change event: the [playback log](#playback-log) tells the app when something was paused,
  skipped or finished.

## Playback log

A lower-level log of what played, for things tracked lists don't cover: play counts, listening history, scrobbling.

```ts
import { onPlaybackEvent } from "tauri-plugin-native-audio-api";

await onPlaybackEvent(async (event) => {
  // { id, type: "start" | "complete" | "skip", itemId, positionMs, atMs }
  if (event.type === "complete") await db.addPlayCount(event.itemId, event.id);
});
```

- `start`: a track started playing (once per play, not on every resume after a pause).
- `complete`: it played to the end (repeat `one` counts each time round). `skip`: it was left before the end, with
  `positionMs` where.
- Only tracks with an `id` are logged. The newest 500 events are kept until the app picks them up.
- `getPlaybackEvents()` / `acknowledgePlaybackEvents(ids)` are the lower-level versions of `onPlaybackEvent`.

## Progress checkpoint

A lightweight record of the current item and position, saved natively while playing, for apps that keep their own
"continue where you left off" (it predates [resuming the queue](playback.md#resuming-the-last-queue), which covers
most of what it's for).

```ts
import { getProgressCheckpoint, clearProgressCheckpoint } from "tauri-plugin-native-audio-api";

const checkpoint = await getProgressCheckpoint(); // { id, currentTime, updatedAtMs, status } or null
await clearProgressCheckpoint();
```

- Needs an `id` on the playing item. Only the latest item is kept (one record).
- Saved on pause, seek, end and about once a second while playing; positions in the first 0.25 s are ignored.
- Stored in `SharedPreferences` on Android, `progress-checkpoint.json` on desktop and `UserDefaults` on iOS.
