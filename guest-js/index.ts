/**
 * JavaScript API of tauri-plugin-native-audio: native playback on Android (Media3) and desktop (the
 * plugin's Rust player). Built into dist-js with `npm run build` (the built files are committed, since
 * apps install the plugin straight from GitHub).
 */

import { addPluginListener, invoke } from '@tauri-apps/api/core';

export type NativeAudioStatus = 'idle' | 'loading' | 'playing' | 'ended' | 'error';

export type NativeAudioState = {
  status: NativeAudioStatus;
  currentTime: number;
  duration: number;
  isPlaying: boolean;
  buffering: boolean;
  rate: number;
  /** Index of the current item in the queue, or -1 when nothing is loaded. */
  queueIndex: number;
  queueLength: number;
  /** `id` of the current item, when one was given. */
  currentId?: number;
  shuffle: boolean;
  repeatMode: NativeAudioRepeatMode;
  /** When a running sleep timer pauses playback (epoch ms), or null. */
  sleepTimerEndsAtMs: number | null;
  /** A sleep timer that pauses at the end of the current track. */
  sleepTimerEndOfTrack: boolean;
  error?: string;
  /** The player's own volume (`setVolume`), 0 to 1, on top of the system volume. */
  volume: number;
};

/** `off`: stop after the last track. `all`: loop the whole queue. `one`: loop the current track. */
export type NativeAudioRepeatMode = 'off' | 'all' | 'one';

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
  /** The playable folder (playlist, album) this queue plays, for `track: 'folder'` tracked lists. */
  sourceId?: string;
  /** Defaults to 0. */
  startIndex?: number;
  /** Seconds into the start item. Defaults to 0. */
  startPosition?: number;
};

/** A playable item. Defined once in `items`, then referenced by `id` from any number of folders. */
export type NativeAudioLibraryItem = {
  id: number;
  src: string;
  title?: string;
  artist?: string;
  album?: string;
  artworkUrl?: string;
  /** Android Auto shows this as a header above a run of entries with the same group, e.g. "A". */
  group?: string;
  /** 0 to 1: shown as not played (0), partly played (with a progress bar) or played (1). */
  progress?: number;
};

export type NativeAudioLibraryFolder = {
  /** Unique across all folders. */
  id: string;
  title: string;
  subtitle?: string;
  artworkUrl?: string;
  /** Can be played as a whole (albums, playlists). Picking an item inside plays the folder from that item. */
  playable?: boolean;
  /** How Android Auto lays out this folder's children. */
  style?: 'list' | 'grid';
  /** Sub-folders, and item ids from `items`. */
  children?: Array<NativeAudioLibraryFolder | number>;
  /** Show a tracked list (see `setTrackedLists`) instead of `children`; it updates on its own. */
  trackedList?: string;
  /** Android Auto shows this as a header above a run of entries with the same group. */
  group?: string;
};

export type NativeAudioLibrary = {
  items: NativeAudioLibraryItem[];
  /** Top-level folders: Android Auto's tabs (it shows up to 4). */
  root: NativeAudioLibraryFolder[];
  /**
   * Search in Android Auto: the search button, and "play ... on <app>" by voice. Searches titles,
   * artists and albums natively. Default true.
   */
  search?: boolean;
  /** Shown in Android Auto when `root` is empty, e.g. "Open the app to sync". */
  emptyMessage?: string;
  /** Items without `artworkUrl` show the cover embedded in their audio file. Default true. */
  embeddedArtwork?: boolean;
  /**
   * Folders without `artworkUrl`: a 2x2 `collage` of their covers (one per album), their `first`
   * cover, or `none`. Default `collage`.
   */
  folderArtwork?: 'collage' | 'first' | 'none';
};

export type NativeAudioQueue = {
  /** Queue entries in queue (list) order; indices match `queueIndex`, `skipTo` and the queue edits. */
  items: NativeAudioQueueItem[];
  /** -1 when the queue is empty. */
  currentIndex: number;
  /** Queue indices in the order they play: the shuffle order when shuffle is on. */
  playOrder: number[];
  /** The playable folder (playlist, album) the queue was started from (setQueue's `sourceId`, or picked in Android Auto). */
  sourceId?: string | null;
};

export type NativeAudioOptions = {
  /**
   * Load the last queue automatically: on Android when the audio service starts with nothing loaded
   * (Android Auto connecting, a headset play button); on desktop at the app's first `initialize()`
   * (call it once the UI is ready). `paused` (the default) loads it where
   * it was, `play` also starts playing, `off` leaves it to the app. `true` / `false` mean `paused` /
   * `off`. `restoreLastQueue()` works either way.
   *
   * Android Auto can start playback by itself when it connects (its own "auto-resume media"
   * setting), whatever this is set to: `paused` can't hold it back.
   */
  resumeLastQueue?: boolean | 'off' | 'paused' | 'play';
  /**
   * Previous (the app's, the notification's, the car's and the media keys') restarts the current
   * track when it's past this many seconds, otherwise goes to the previous track. 0: previous always
   * goes to the previous track. Default 3.
   */
  previousRestartsAfterSeconds?: number;
  /**
   * Next on the last track with repeat off: `nothing` (the default), or `first`: back to the first
   * track of the play order, paused.
   */
  nextAtEnd?: 'nothing' | 'first';
  /**
   * Keep tracks added with `addToQueue` when the queue repeats (repeat mode `all`). Default false:
   * they play once and are dropped when the queue starts over.
   */
  repeatAddedTracks?: boolean;
  /**
   * Android: how long the playback controls (notification, lock screen, Android Auto's player) stay
   * after a pause, so playback is easy to pick up again. Default 30; 0 lets them go right away.
   */
  pausedKeepAliveMinutes?: number;
  /**
   * Android, with carSupport: while the phone is connected to Android Auto, keep the controls after
   * a pause with no time limit, so Android Auto's player stays. `pausedKeepAliveMinutes` starts
   * counting when Android Auto disconnects. Default true.
   */
  keepAliveWhileCarConnected?: boolean;
  /**
   * Remember how far each item played (podcasts, audiobooks): Android Auto shows played / partly
   * played marks, and partly played items picked there continue where they stopped. Read and set it
   * with `getItemProgress` / `setItemProgress`. Default false.
   */
  trackProgress?: boolean;
  /**
   * Android: when something outside the app stops playback, pause instead and keep the queue loaded,
   * so the notification and Android Auto's player stay and playback continues where it was. Android's
   * media panel stops players that were paused for about 10 minutes; Bluetooth devices and Android
   * Auto can send a stop too. Default true; false lets them unload the queue.
   */
  keepQueueOnStop?: boolean;
  /**
   * Desktop: show the player in the system's media controls (the Windows media panel, macOS Now
   * Playing, MPRIS on Linux) with the media keys. Default true.
   */
  mediaControls?: boolean;
  /**
   * How a `setVolume` value becomes loudness. Hearing is roughly logarithmic, so with `linear` 50%
   * sounds only a little quieter; `quadratic` (the default) makes 50% about half as loud (-12 dB);
   * `cubic` gives finer control at low volumes (50% = -18 dB).
   */
  volumeCurve?: 'linear' | 'quadratic' | 'cubic';
};

export type NativeAudioOutputDevice = {
  /** Pass to `setOutputDevice`. */
  id: string;
  name: string;
  /** The system's default output. */
  isDefault: boolean;
};

export type NativeAudioOutputDevices = {
  devices: NativeAudioOutputDevice[];
  /** The chosen device, or null to follow the system default. */
  selected: string | null;
  /** The id of the device playing now: the chosen one, or the default while following it (or while the chosen one isn't there). Null before the first playback. */
  active: string | null;
};

export type NativeAudioSleepTimer =
  /** Pause after this many minutes, fading out over the last `fadeOutSeconds` (default 10). */
  | { minutes: number; fadeOutSeconds?: number }
  /** Pause at the end of the current track. */
  | { endOfTrack: true };

export type NativeAudioItemProgress = {
  itemId: number;
  /** 0 to 1; 1 = played (from 98% on). */
  progress: number;
  positionMs: number;
  /** When it changed; `setItemProgress` with `merge` keeps the newer version. */
  updatedAtMs: number;
};

/** Built-in icons for custom buttons. Toggle buttons show the filled version while active. */
export type NativeAudioControlIcon =
  | 'heart' | 'star' | 'thumbUp' | 'thumbDown' | 'bookmark' | 'flag' | 'checkCircle' | 'plusCircle' | 'minusCircle'
  | 'plus' | 'minus' | 'playlistAdd' | 'playlistRemove' | 'queueAdd' | 'queueNext' | 'queueRemove' | 'block'
  | 'share' | 'radio' | 'album' | 'artist' | 'feed' | 'settings' | 'sync' | 'quality' | 'signal';

export type NativeAudioControlButton =
  | { action: 'seekBack' | 'seekForward'; seconds: number }
  | { action: 'shuffle' }
  | { action: 'repeat' }
  /** Cycles through `rates` (default 1, 1.25, 1.5, 2). */
  | { action: 'speed'; rates?: number[] }
  /** Cycles off, then each option (minutes or 'endOfTrack'), then off. Default 15, 30, 60, end of track. */
  | { action: 'sleepTimer'; options?: Array<number | 'endOfTrack'>; fadeOutSeconds?: number }
  | {
      action: 'custom';
      /** Unique; comes back in presses as `buttonId`. */
      id: string;
      label: string;
      icon?: NativeAudioControlIcon;
      /** Name of a drawable in the app's Android resources, used instead of `icon`. */
      iconResource?: string;
      /** On / off per track (e.g. like). Presses report the new state as `active`. */
      toggle?: boolean;
    };

export type NativeAudioControls = {
  /** Extra buttons next to play / pause / previous / next, in order. */
  buttons: NativeAudioControlButton[];
  /** Repeat presses of the same button within this window are ignored. Default 500. */
  debounceMs?: number;
};

export type NativeAudioControlPress = {
  /** Unique per press. */
  id: string;
  buttonId: string;
  /** `id` of the track playing when it was pressed. */
  itemId: number | null;
  /** New state of a toggle button; null for other buttons. */
  active: boolean | null;
  pressedAtMs: number;
};

export type NativeAudioPlaybackEvent = {
  /** Unique per event. */
  id: string;
  /** `start`: a track started playing. `complete`: it played to the end. `skip`: it was left before the end. */
  type: 'start' | 'complete' | 'skip';
  /** `id` of the track. Tracks without an id aren't logged. */
  itemId: number;
  /** Position in the track when it happened. */
  positionMs: number;
  atMs: number;
};

export type NativeAudioTrackedListConfig = {
  /** Unique; used in `getTrackedList`, changes and a library folder's `trackedList`. */
  id: string;
  /**
   * `folder`: the playable folder (playlist, album) a queue was started from: picked in Android
   * Auto, or `setQueue({ sourceId })`. `item`: the tracks themselves.
   */
  track: 'folder' | 'item';
  /** How many to keep, most recent first. Default 10. */
  limit?: number;
  /** How long something has to play before it counts. Default 0: as soon as it starts. Finishing always counts. */
  countAfterSeconds?: number;
};

export type NativeAudioTrackedListEntry = {
  /** A folder id (string) or an item id (number). */
  id: string | number;
  /** When it was last played; used to merge versions of a list (newest wins). */
  playedAtMs: number;
};

export type NativeAudioTrackedListChange = {
  /** Unique per change. */
  id: string;
  listId: string;
  /** The whole list after the change, newest first. */
  entries: NativeAudioTrackedListEntry[];
  /** The entries' ids, as a shortcut. */
  ids: Array<string | number>;
  changedAtMs: number;
};

export type NativeAudioProgressCheckpoint = {
  id: number;
  currentTime: number;
  updatedAtMs: number;
  status?: 'idle' | 'loading' | 'playing' | 'ended' | 'error';
};

export type NativeAudioUpdateQueueOptions = {
  /** The queue's playable folder (for `folder` tracked lists); unchanged when left out. */
  sourceId?: string;
  /**
   * The playing track left the playlist: `finish` (default) lets it play to the end, then it leaves
   * the queue; `skip` moves on to the next track right away.
   */
  removedCurrent?: 'finish' | 'skip';
  /** Tracks no longer in the playlist: `remove` (default) them, or `keep` them in the queue where they are. */
  removedItems?: 'remove' | 'keep';
  /**
   * New tracks: `inPlace` (default) at their place in the playlist (with shuffle on: somewhere in
   * the rest of this pass), at the `end` (with shuffle on: the end of the play order, shuffled
   * among themselves), or `next`: right after the playing track, in playlist order.
   */
  newItems?: 'inPlace' | 'end' | 'next';
  /**
   * The queue's list order: the `playlist`'s new order (default), or the `queue`'s current order
   * with new tracks placed by `newItems` (in place: after the track before them in the playlist).
   * With shuffle on, the play order is kept either way.
   */
  order?: 'playlist' | 'queue';
};

export type NativeAudioUpdateQueueResult = {
  state: NativeAudioState;
  /** Tracks new to the playlist. */
  added: number;
  /** Tracks no longer in the playlist (also when kept, or playing to the end). */
  removed: number;
};

const PLUGIN_NAME = 'native-audio';
const STATE_EVENT = 'native_audio_state';
const CONTROL_EVENT = 'native_audio_control';
const PLAYBACK_EVENT = 'native_audio_playback_event';
const TRACKED_LIST_EVENT = 'native_audio_tracked_list';

const call = async <T>(command: string, payload?: Record<string, unknown>): Promise<T> =>
  await invoke<T>(`plugin:${PLUGIN_NAME}|${command}`, payload);

// Hands every entry of a saved log to `handler` in order, once per app run: first the entries
// saved while the app wasn't running, then live ones. Each is acknowledged (removed from the log)
// only after the handler finished, so it's delivered again next time if the app closes midway.
const consumeLog = async <T extends { id: string }>(
  eventName: string,
  getPending: () => Promise<T[]>,
  acknowledge: (ids: string[]) => Promise<void>,
  handler: (entry: T) => void | Promise<void>,
): Promise<() => void> => {
  const seen = new Set<string>();
  let queue = Promise.resolve();
  const deliver = (entries: T[]) => {
    queue = queue.then(async () => {
      for (const entry of entries) {
        if (seen.has(entry.id)) continue;
        seen.add(entry.id);
        try {
          await handler(entry);
          await acknowledge([entry.id]);
        } catch (error) {
          seen.delete(entry.id);
          console.error(`native-audio: ${eventName} handler failed`, error);
        }
      }
    });
    return queue;
  };
  const listener = await addPluginListener<T>(PLUGIN_NAME, eventName, (entry) => deliver([entry]));
  await deliver(await getPending());
  return () => void listener.unregister();
};

// ---- Setup and state

/** Call once, when the app's UI is ready (on desktop the last queue is resumed here, see `resumeLastQueue`). */
export const initialize = async (): Promise<NativeAudioState> => await call('initialize');

export const getState = async (): Promise<NativeAudioState> => await call('get_state');

/** Calls `handler` on every state change, and regularly while playing. Resolves to a function that stops listening. */
export const addStateListener = async (handler: (state: NativeAudioState) => void): Promise<() => void> => {
  const listener = await addPluginListener<NativeAudioState>(PLUGIN_NAME, STATE_EVENT, handler);
  return () => void listener.unregister();
};

/** Saved on the device (desktop too); only the options passed are changed. */
export const setOptions = async (options: NativeAudioOptions): Promise<void> => await call('set_options', options);

/** Stops playback and empties the queue. The saved queue stays, for `restoreLastQueue()`. */
export const dispose = async (): Promise<void> => await call('dispose');

// ---- The queue

/** A queue with a single item. */
export const setSource = async (payload: NativeAudioSetSourcePayload): Promise<NativeAudioState> =>
  await call('set_source', payload);

export const setQueue = async (payload: NativeAudioSetQueuePayload): Promise<NativeAudioState> =>
  await call('set_queue', payload);

/**
 * Adds items to the playing queue: at the end, or right after the current track with `playNext`.
 * With shuffle on they get the same spot in the play order, and the rest of the order is kept.
 * With an empty queue, it becomes the queue.
 */
export const addToQueue = async (items: NativeAudioQueueItem[], options: { playNext?: boolean } = {}): Promise<NativeAudioState> =>
  await call('add_to_queue', { items, playNext: options.playNext ?? false });

/**
 * The playlist (or album) the queue plays changed, e.g. after a sync: `items` is its new content.
 * Entries are matched by `id` (the same id twice: in order). What's still there keeps its place,
 * also in a shuffled play order; new tracks are added and missing ones removed (see the options).
 * Tracks added with `addToQueue` always stay, after the track they followed. The playing track's
 * details (title, artwork) are updated without interrupting it. With an empty queue, `items`
 * becomes the queue. Resolves with the state and how many tracks were added and removed.
 */
export const updateQueue = async (
  items: NativeAudioQueueItem[],
  options: NativeAudioUpdateQueueOptions = {},
): Promise<NativeAudioUpdateQueueResult> => await call('update_queue', { items, ...options });

/** Removes the entry at a queue index. Removing the current track moves on to the next one. */
export const removeFromQueue = async (index: number): Promise<NativeAudioState> => await call('remove_from_queue', { index });

/**
 * Moves a queue entry between queue indices. Without shuffle this changes the play order; with
 * shuffle on, the play order is kept (only the list position changes).
 */
export const moveInQueue = async (from: number, to: number): Promise<NativeAudioState> =>
  await call('move_in_queue', { from, to });

export const getQueue = async (): Promise<NativeAudioQueue> => await call('get_queue');

/**
 * Loads the last queue (tracks, current track and position, shuffle order, repeat mode), paused.
 * It's saved automatically as it plays and changes. Resolves with null when nothing was saved.
 * When a queue is already loaded (for example playing from Android Auto), it's left alone and this
 * resolves with the current state, so it's safe to call every time the app starts.
 */
export const restoreLastQueue = async (): Promise<NativeAudioState | null> =>
  (await call<NativeAudioState | null>('restore_last_queue')) ?? null;

// ---- Playback

export const play = async (): Promise<NativeAudioState> => await call('play');
export const pause = async (): Promise<NativeAudioState> => await call('pause');

/** Seconds into the current track. Past the end seeks to the end, from where playback moves on. */
export const seekTo = async (position: number): Promise<NativeAudioState> => await call('seek_to', { position });

export const next = async (): Promise<NativeAudioState> => await call('next');
export const previous = async (): Promise<NativeAudioState> => await call('previous');
export const skipTo = async (index: number): Promise<NativeAudioState> => await call('skip_to', { index });

/**
 * Plays the queue in a shuffled order that starts with the current track. Every track plays once
 * per pass, and with repeat `all` each pass gets a new order. `queueIndex` stays the index in `items`.
 */
export const setShuffle = async (enabled: boolean): Promise<NativeAudioState> => await call('set_shuffle', { enabled });

export const setRepeatMode = async (mode: NativeAudioRepeatMode): Promise<NativeAudioState> =>
  await call('set_repeat_mode', { mode });

/**
 * What the previous / next buttons in the notification, lock screen and headset (desktop: the media
 * keys and the system's media controls) do.
 * `0` (the default) skips whole tracks in the queue; above `0` seeks back / forward by that many seconds.
 */
export const setSkipInterval = async (seconds: number): Promise<NativeAudioState> =>
  await call('set_skip_interval', { seconds });

/** Pauses playback after some minutes (fading out) or at the end of the current track. Replaces a running timer. */
export const setSleepTimer = async (timer: NativeAudioSleepTimer): Promise<NativeAudioState> =>
  await call('set_sleep_timer', timer);

export const cancelSleepTimer = async (): Promise<NativeAudioState> => await call('cancel_sleep_timer');

/** Playback speed (Android, iOS). Rejects on desktop. */
export const setRate = async (rate: number): Promise<NativeAudioState> => await call('set_rate', { rate });

/**
 * The player's own volume, 0 to 1 (a slider's position), on top of the system volume. Default 1.
 * What's heard follows the `volumeCurve` option. Saved for the next run.
 */
export const setVolume = async (volume: number): Promise<NativeAudioState> => await call('set_volume', { volume });

/**
 * Desktop: the audio outputs to choose from. Android picks the output itself (speaker, headphones,
 * Bluetooth, the car) and returns an empty list.
 */
export const getOutputDevices = async (): Promise<NativeAudioOutputDevices> => await call('get_output_devices');

/**
 * Desktop: plays on this output (an `id` from `getOutputDevices`), or follows the system default
 * with null (the default; it also moves along when the default changes). When the chosen device goes
 * away, playback moves to the default and comes back once it's there again. Saved: the next run
 * plays on it too. Android: does nothing.
 */
export const setOutputDevice = async (id: string | null): Promise<NativeAudioOutputDevices> =>
  await call('set_output_device', { id });

// ---- Syncing with the app

/**
 * Declares lists the plugin keeps up to date natively, e.g. the last 5 playlists played: most
 * recent first, no duplicates, capped at `limit`. They keep updating while the app isn't running
 * (Android Auto). Lists no longer declared are dropped. Saved on the device.
 */
export const setTrackedLists = async (lists: NativeAudioTrackedListConfig[]): Promise<void> =>
  await call('set_tracked_lists', { lists });

/** A tracked list's current entries, newest first. */
export const getTrackedList = async (id: string): Promise<NativeAudioTrackedListEntry[]> =>
  (await call<{ entries: NativeAudioTrackedListEntry[] }>('get_tracked_list', { id })).entries;

/**
 * Sets a tracked list from the app, e.g. from its database or a synced copy. Replaces it, or with
 * `merge: true` combines it with the current list: by id, newest `playedAtMs` wins, newest first,
 * capped at the limit. Not reported back as a change. Resolves with the resulting list.
 */
export const setTrackedList = async (
  id: string,
  entries: NativeAudioTrackedListEntry[],
  options: { merge?: boolean } = {},
): Promise<NativeAudioTrackedListEntry[]> =>
  (await call<{ entries: NativeAudioTrackedListEntry[] }>('set_tracked_list', { id, entries, merge: options.merge ?? false })).entries;

/** Unacknowledged list changes: the latest one per list. */
export const getTrackedListChanges = async (): Promise<NativeAudioTrackedListChange[]> =>
  (await call<{ changes: NativeAudioTrackedListChange[] }>('get_tracked_list_changes')).changes;

export const acknowledgeTrackedListChanges = async (ids: string[]): Promise<void> =>
  await call('acknowledge_tracked_list_changes', { ids });

/**
 * Calls `handler` with tracked list changes: first those from while the app wasn't running (only
 * the latest per list), then live ones. Each is acknowledged after `handler` finishes.
 */
export const onTrackedListChanged = async (
  handler: (change: NativeAudioTrackedListChange) => void | Promise<void>,
): Promise<() => void> => await consumeLog(TRACKED_LIST_EVENT, getTrackedListChanges, acknowledgeTrackedListChanges, handler);

/** Logged playback events not acknowledged yet, oldest first. */
export const getPlaybackEvents = async (): Promise<NativeAudioPlaybackEvent[]> =>
  (await call<{ events: NativeAudioPlaybackEvent[] }>('get_playback_events')).events;

export const acknowledgePlaybackEvents = async (ids: string[]): Promise<void> =>
  await call('acknowledge_playback_events', { ids });

/**
 * Calls `handler` for every playback event (track started / completed / skipped): first those
 * logged while the app wasn't running (e.g. playing from Android Auto), then live ones. Each is
 * acknowledged after `handler` finishes. Use it for recently played, play counts, progress or
 * scrobbling. Returns a function to stop listening.
 */
export const onPlaybackEvent = async (
  handler: (event: NativeAudioPlaybackEvent) => void | Promise<void>,
): Promise<() => void> => await consumeLog(PLAYBACK_EVENT, getPlaybackEvents, acknowledgePlaybackEvents, handler);

/** Recorded progress (setOptions `trackProgress`), of all items or the given ones. */
export const getItemProgress = async (itemIds?: number[]): Promise<NativeAudioItemProgress[]> =>
  (await call<{ entries: NativeAudioItemProgress[] }>('get_item_progress', { itemIds })).entries;

/**
 * Sets progress from the app, e.g. "mark as played" or a synced copy. Replaces the given items'
 * progress, or with `merge: true` keeps whichever version is newer. Resolves with the result.
 */
export const setItemProgress = async (
  entries: NativeAudioItemProgress[],
  options: { merge?: boolean } = {},
): Promise<NativeAudioItemProgress[]> =>
  (await call<{ entries: NativeAudioItemProgress[] }>('set_item_progress', { entries, merge: options.merge ?? false })).entries;

export const getProgressCheckpoint = async (): Promise<NativeAudioProgressCheckpoint | null> =>
  (await call<NativeAudioProgressCheckpoint | null>('get_progress_checkpoint')) ?? null;

export const clearProgressCheckpoint = async (): Promise<void> => await call('clear_progress_checkpoint');

// ---- Android Auto and the Android media controls

/**
 * The browsable library shown in Android Auto (with `plugins.native-audio.carSupport` enabled).
 * Saved natively, so Auto can browse and play it while the app isn't running. Replaces the
 * previous library. Elsewhere it's accepted and not used.
 */
export const setLibrary = async (library: NativeAudioLibrary): Promise<void> => await call('set_library', library);

/**
 * Extra buttons for Android Auto and the Android 13+ media controls (Android, with carSupport).
 * Saved on the device, so they're there even when Android Auto starts the app in the background.
 */
export const setControls = async (controls: NativeAudioControls): Promise<void> => await call('set_controls', controls);

/** Sets a toggle button's state for tracks, e.g. from the app's own liked songs. */
export const setControlActive = async (buttonId: string, itemIds: number[], active: boolean): Promise<void> =>
  await call('set_control_active', { buttonId, itemIds, active });

/** Logged custom button presses not acknowledged yet, oldest first. */
export const getControlPresses = async (): Promise<NativeAudioControlPress[]> =>
  (await call<{ presses: NativeAudioControlPress[] }>('get_control_presses')).presses;

export const acknowledgeControlPresses = async (ids: string[]): Promise<void> =>
  await call('acknowledge_control_presses', { ids });

/**
 * Calls `handler` for every custom button press: first those logged while the app wasn't running,
 * then live ones. Each is acknowledged after `handler` finishes. Returns a function to stop listening.
 */
export const onControlPressed = async (
  handler: (press: NativeAudioControlPress) => void | Promise<void>,
): Promise<() => void> => await consumeLog(CONTROL_EVENT, getControlPresses, acknowledgeControlPresses, handler);
