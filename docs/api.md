# JavaScript API

[← README](../README.MD) · [Playback](playback.md) · [Options](options.md)

Everything is imported from `tauri-plugin-native-audio-api`. The full types, with comments, are in
[`guest-js/dist-js/index.d.ts`](../guest-js/dist-js/index.d.ts).

- [Commands](#commands)
- [State](#state)
- [Types](#types)
- [Validation and errors](#validation-and-errors)
- [Permissions](#permissions)

## Commands

Commands that change playback resolve with the new `NativeAudioState`. "A / D" marks what's Android-only (A) or
desktop-only (D); on the other platform those are accepted and do nothing (or return empty results), except
`setRate`, which rejects on desktop.

**Setup and state**

| Command | |
| --- | --- |
| `initialize(): Promise<NativeAudioState>` | Call once, when the UI is ready (desktop resumes the last queue here) |
| `getState(): Promise<NativeAudioState>` | |
| `addStateListener(handler): Promise<() => void>` | Called on every change, and regularly while playing (the position) |
| `setOptions(options: NativeAudioOptions): Promise<void>` | See [Options](options.md) |
| `dispose(): Promise<void>` | Stops playback and empties the queue (the saved queue stays) |

**Queue** ([guide](playback.md))

| Command | |
| --- | --- |
| `setSource(payload: NativeAudioSetSourcePayload)` | A queue of one |
| `setQueue(payload: NativeAudioSetQueuePayload)` | Replaces the queue |
| `addToQueue(items, options?: { playNext?: boolean })` | |
| `updateQueue(items, options?: NativeAudioUpdateQueueOptions): Promise<NativeAudioUpdateQueueResult>` | A changed playlist, see [the guide](playback.md#updating-the-queue-from-a-changed-playlist) |
| `removeFromQueue(index: number)` | |
| `moveInQueue(from: number, to: number)` | |
| `getQueue(): Promise<NativeAudioQueue>` | Items, current index and play order |
| `restoreLastQueue(): Promise<NativeAudioState \| null>` | |

**Playback**

| Command | |
| --- | --- |
| `play()`, `pause()` | |
| `seekTo(position: number)` | Seconds |
| `next()`, `previous()`, `skipTo(index: number)` | |
| `setShuffle(enabled: boolean)`, `setRepeatMode(mode: NativeAudioRepeatMode)` | |
| `setSkipInterval(seconds: number)` | What the system's previous / next buttons do |
| `setSleepTimer(timer: NativeAudioSleepTimer)`, `cancelSleepTimer()` | |
| `setVolume(volume: number)` | 0 to 1 |
| `setRate(rate: number)` | Playback speed (A; rejects on desktop) |
| `getOutputDevices(): Promise<NativeAudioOutputDevices>` | D |
| `setOutputDevice(id: string \| null): Promise<NativeAudioOutputDevices>` | D |

**Syncing** ([guide](syncing.md))

| Command | |
| --- | --- |
| `setTrackedLists(lists: NativeAudioTrackedListConfig[]): Promise<void>` | |
| `getTrackedList(id): Promise<NativeAudioTrackedListEntry[]>` | |
| `setTrackedList(id, entries, options?: { merge?: boolean }): Promise<NativeAudioTrackedListEntry[]>` | |
| `onTrackedListChanged(handler): Promise<() => void>` | |
| `getTrackedListChanges()`, `acknowledgeTrackedListChanges(ids)` | Lower level |
| `onPlaybackEvent(handler): Promise<() => void>` | |
| `getPlaybackEvents()`, `acknowledgePlaybackEvents(ids)` | Lower level |
| `getItemProgress(itemIds?: number[]): Promise<NativeAudioItemProgress[]>` | |
| `setItemProgress(entries, options?: { merge?: boolean }): Promise<NativeAudioItemProgress[]>` | |
| `getProgressCheckpoint(): Promise<NativeAudioProgressCheckpoint \| null>`, `clearProgressCheckpoint()` | |

**Android Auto** ([guide](android-auto.md)), all A

| Command | |
| --- | --- |
| `setLibrary(library: NativeAudioLibrary): Promise<void>` | |
| `setControls(controls: NativeAudioControls): Promise<void>` | |
| `setControlActive(buttonId, itemIds, active): Promise<void>` | |
| `onControlPressed(handler): Promise<() => void>` | |
| `getControlPresses()`, `acknowledgeControlPresses(ids)` | Lower level |

## State

```ts
type NativeAudioState = {
  status: "idle" | "loading" | "playing" | "ended" | "error";
  currentTime: number;             // seconds
  duration: number;                // seconds
  isPlaying: boolean;
  buffering: boolean;
  rate: number;
  queueIndex: number;              // the current item's index in the queue, -1 when nothing is loaded
  queueLength: number;
  currentId?: number;              // the current item's id, when it has one
  shuffle: boolean;
  repeatMode: "off" | "all" | "one";
  sleepTimerEndsAtMs: number | null; // when a running sleep timer pauses (epoch ms)
  sleepTimerEndOfTrack: boolean;
  volume: number;                  // setVolume, 0 to 1
  error?: string;
};
```

- `idle`: nothing playing, or paused.
- `loading`: buffering (Android) or opening a track (desktop).
- `playing`: playing.
- `ended`: the last track finished (never with repeat `all` / `one`).
- `error`: playback failed (desktop: nothing in the queue could be played). `error` holds the message; it clears
  with the next play, skip or load.

## Types

```ts
type NativeAudioQueueItem = {
  src: string;          // Android: URL or path; desktop: local path or file:// URL
  id?: number;          // used by tracked lists, the playback log, progress and updateQueue
  title?: string;
  artist?: string;
  artworkUrl?: string;
};
type NativeAudioSetSourcePayload = NativeAudioQueueItem;

type NativeAudioSetQueuePayload = {
  items: NativeAudioQueueItem[];
  sourceId?: string;      // the playlist / album this queue plays, for "folder" tracked lists
  startIndex?: number;    // default 0
  startPosition?: number; // seconds into the start item, default 0
};

type NativeAudioQueue = {
  items: NativeAudioQueueItem[]; // list order
  currentIndex: number;          // -1 when empty
  playOrder: number[];           // queue indices in the order they play
};

type NativeAudioUpdateQueueOptions = {
  sourceId?: string;
  removedCurrent?: "finish" | "skip";       // default "finish"
  removedItems?: "remove" | "keep";         // default "remove"
  newItems?: "inPlace" | "end" | "next";    // default "inPlace"
  order?: "playlist" | "queue";             // default "playlist"
};
type NativeAudioUpdateQueueResult = { state: NativeAudioState; added: number; removed: number };

type NativeAudioSleepTimer = { minutes: number; fadeOutSeconds?: number } | { endOfTrack: true };

type NativeAudioOptions = {
  resumeLastQueue?: boolean | "off" | "paused" | "play"; // default "paused"
  previousRestartsAfterSeconds?: number;                  // default 3
  nextAtEnd?: "nothing" | "first";                        // default "nothing"
  repeatAddedTracks?: boolean;                            // default false
  volumeCurve?: "linear" | "quadratic" | "cubic";         // default "quadratic"
  trackProgress?: boolean;                                // default false
  mediaControls?: boolean;                                // desktop, default true
  pausedKeepAliveMinutes?: number;                        // Android, default 30
  keepAliveWhileCarConnected?: boolean;                   // Android with carSupport, default true
  keepQueueOnStop?: boolean;                              // Android, default true
};

type NativeAudioOutputDevices = {
  devices: { id: string; name: string; isDefault: boolean }[];
  selected: string | null; // the chosen id, null: follow the system default
  active: string | null;   // the id playing now
};

type NativeAudioTrackedListConfig = {
  id: string;
  track: "folder" | "item";
  limit?: number;             // default 10
  countAfterSeconds?: number; // default 0
};
type NativeAudioTrackedListEntry = { id: string | number; playedAtMs: number };
type NativeAudioTrackedListChange = {
  id: string;
  listId: string;
  entries: NativeAudioTrackedListEntry[]; // the whole list, newest first
  ids: Array<string | number>;
  changedAtMs: number;
};

type NativeAudioItemProgress = { itemId: number; progress: number; positionMs: number; updatedAtMs: number };

type NativeAudioPlaybackEvent = {
  id: string;
  type: "start" | "complete" | "skip";
  itemId: number;
  positionMs: number;
  atMs: number;
};

type NativeAudioProgressCheckpoint = { id: number; currentTime: number; updatedAtMs: number; status?: string };
```

The Android Auto types (`NativeAudioLibrary`, `NativeAudioLibraryItem`, `NativeAudioLibraryFolder`,
`NativeAudioControls`, `NativeAudioControlButton`, `NativeAudioControlIcon`, `NativeAudioControlPress`) are
described in [Android Auto](android-auto.md) and in `index.d.ts`.

## Validation and errors

Commands reject (with a message saying what's wrong) when:

- `setSource` / `setQueue` / `addToQueue` / `updateQueue` get an item without `src`.
- Android: `setQueue` / `addToQueue` get no items, or `setQueue` a `startIndex` out of range. (Desktop: an empty
  `setQueue` clears the queue, and `startIndex` is clamped.)
- `skipTo` / `removeFromQueue` / `moveInQueue` get an index out of range.
- `seekTo` gets a missing or non-finite position. (Past the end seeks to the end, from where playback moves on.)
- `setRate` gets `rate <= 0` (and always on desktop).
- `setSkipInterval` gets a negative or non-finite value.
- `setShuffle` gets no `enabled`; `setRepeatMode` a mode other than `off`, `all`, `one`.
- `setSleepTimer` gets neither `minutes > 0` nor `endOfTrack: true`, or `fadeOutSeconds < 0`.
- `setOptions` / `updateQueue` get a value outside the listed ones.
- `setOutputDevice` gets an id that isn't connected.
- `setLibrary`, `setControls`, `setTrackedLists`, `setTrackedList` and `setItemProgress` get invalid input.

Playback failures show up in the state as `error` (see [State](#state)).

## Permissions

`native-audio:default` allows every command. For a stricter setup, list the ones you use instead, or use the
allow / deny permissions in `permissions/autogenerated/commands/*.toml`:

`allow-initialize`, `allow-register-listener`, `allow-remove-listener` (needed for every listener),
`allow-set-source`, `allow-set-queue`, `allow-next`, `allow-previous`, `allow-skip-to`, `allow-add-to-queue`,
`allow-update-queue`, `allow-remove-from-queue`, `allow-move-in-queue`, `allow-get-queue`,
`allow-restore-last-queue`, `allow-play`, `allow-pause`, `allow-seek-to`, `allow-set-rate`, `allow-set-volume`,
`allow-get-output-devices`, `allow-set-output-device`, `allow-set-skip-interval`, `allow-set-shuffle`,
`allow-set-repeat-mode`, `allow-set-sleep-timer`, `allow-cancel-sleep-timer`, `allow-set-options`,
`allow-set-library`, `allow-set-controls`, `allow-set-control-active`, `allow-get-control-presses`,
`allow-acknowledge-control-presses`, `allow-get-playback-events`, `allow-acknowledge-playback-events`,
`allow-set-tracked-lists`, `allow-get-tracked-list`, `allow-set-tracked-list`, `allow-get-tracked-list-changes`,
`allow-acknowledge-tracked-list-changes`, `allow-get-item-progress`, `allow-set-item-progress`, `allow-get-state`,
`allow-get-progress-checkpoint`, `allow-clear-progress-checkpoint`, `allow-dispose`.
