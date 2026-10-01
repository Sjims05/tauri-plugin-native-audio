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
};

export type NativeAudioOptions = {
  /**
   * Android: load the last queue automatically when the audio service starts with nothing loaded
   * (Android Auto connecting, a headset play button). Default true. `restoreLastQueue()` works either way.
   */
  resumeLastQueue?: boolean;
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

export declare const initialize: () => Promise<NativeAudioState>;
export declare const setSource: (payload: NativeAudioSetSourcePayload) => Promise<NativeAudioState>;
export declare const setQueue: (payload: NativeAudioSetQueuePayload) => Promise<NativeAudioState>;
export declare const next: () => Promise<NativeAudioState>;
export declare const previous: () => Promise<NativeAudioState>;
export declare const skipTo: (index: number) => Promise<NativeAudioState>;
/**
 * Adds items to the playing queue: at the end, or right after the current track with `playNext`.
 * With shuffle on they get the same spot in the play order, and the rest of the order is kept.
 * With an empty queue, it becomes the queue.
 */
export declare const addToQueue: (items: NativeAudioQueueItem[], options?: { playNext?: boolean }) => Promise<NativeAudioState>;
/** Removes the entry at a queue index. Removing the current track moves on to the next one. */
export declare const removeFromQueue: (index: number) => Promise<NativeAudioState>;
/**
 * Moves a queue entry between queue indices. Without shuffle this changes the play order; with
 * shuffle on, the play order is kept (only the list position changes).
 */
export declare const moveInQueue: (from: number, to: number) => Promise<NativeAudioState>;
export declare const getQueue: () => Promise<NativeAudioQueue>;
/**
 * Loads the last queue (tracks, current track and position, shuffle order, repeat mode), paused.
 * It's saved automatically as it plays and changes. Resolves with null when nothing was saved.
 */
export declare const restoreLastQueue: () => Promise<NativeAudioState | null>;
export declare const play: () => Promise<NativeAudioState>;
export declare const pause: () => Promise<NativeAudioState>;
export declare const seekTo: (position: number) => Promise<NativeAudioState>;
export declare const setRate: (rate: number) => Promise<NativeAudioState>;
/**
 * What the previous / next buttons in the notification, lock screen and headset do.
 * `0` (the default) skips whole tracks in the queue; above `0` seeks back / forward by that many seconds.
 */
export declare const setSkipInterval: (seconds: number) => Promise<NativeAudioState>;
/**
 * Plays the queue in a shuffled order that starts with the current track. Every track plays once
 * per pass, and with repeat `all` each pass gets a new order. `queueIndex` stays the index in `items`.
 */
export declare const setShuffle: (enabled: boolean) => Promise<NativeAudioState>;
export declare const setRepeatMode: (mode: NativeAudioRepeatMode) => Promise<NativeAudioState>;
/**
 * The browsable library shown in Android Auto (with `plugins.native-audio.carSupport` enabled).
 * Saved natively, so Auto can browse and play it while the app isn't running. Replaces the
 * previous library. On iOS it's accepted but not used yet.
 */
export declare const setLibrary: (library: NativeAudioLibrary) => Promise<void>;
/** Saved on the device; only the options passed are changed. */
export declare const setOptions: (options: NativeAudioOptions) => Promise<void>;
/**
 * Extra buttons for Android Auto and the Android 13+ media controls (Android, with carSupport).
 * Saved on the device, so they're there even when Android Auto starts the app in the background.
 */
/** Pauses playback after some minutes (fading out) or at the end of the current track. Replaces a running timer. */
export declare const setSleepTimer: (timer: NativeAudioSleepTimer) => Promise<NativeAudioState>;
export declare const cancelSleepTimer: () => Promise<NativeAudioState>;
/** Recorded progress (setOptions `trackProgress`), of all items or the given ones. */
export declare const getItemProgress: (itemIds?: number[]) => Promise<NativeAudioItemProgress[]>;
/**
 * Sets progress from the app, e.g. "mark as played" or a synced copy. Replaces the given items'
 * progress, or with `merge: true` keeps whichever version is newer. Resolves with the result.
 */
export declare const setItemProgress: (
  entries: NativeAudioItemProgress[],
  options?: { merge?: boolean }
) => Promise<NativeAudioItemProgress[]>;
export declare const setControls: (controls: NativeAudioControls) => Promise<void>;
/** Sets a toggle button's state for tracks, e.g. from the app's own liked songs. */
export declare const setControlActive: (buttonId: string, itemIds: number[], active: boolean) => Promise<void>;
/** Logged custom button presses not acknowledged yet, oldest first. */
export declare const getControlPresses: () => Promise<NativeAudioControlPress[]>;
export declare const acknowledgeControlPresses: (ids: string[]) => Promise<void>;
/**
 * Calls `handler` for every custom button press: first those logged while the app wasn't running,
 * then live ones. Each is acknowledged after `handler` finishes. Returns a function to stop listening.
 */
/** Logged playback events not acknowledged yet, oldest first. */
export declare const getPlaybackEvents: () => Promise<NativeAudioPlaybackEvent[]>;
export declare const acknowledgePlaybackEvents: (ids: string[]) => Promise<void>;
/**
 * Calls `handler` for every playback event (track started / completed / skipped): first those
 * logged while the app wasn't running (e.g. playing from Android Auto), then live ones. Each is
 * acknowledged after `handler` finishes. Use it for recently played, play counts, progress or
 * scrobbling. Returns a function to stop listening.
 */
export declare const onPlaybackEvent: (handler: (event: NativeAudioPlaybackEvent) => void | Promise<void>) => Promise<() => void>;
/**
 * Declares lists the plugin keeps up to date natively, e.g. the last 5 playlists played: most
 * recent first, no duplicates, capped at `limit`. They keep updating while the app isn't running
 * (Android Auto). Lists no longer declared are dropped. Saved on the device.
 */
export declare const setTrackedLists: (lists: NativeAudioTrackedListConfig[]) => Promise<void>;
/** A tracked list's current entries, newest first. */
export declare const getTrackedList: (id: string) => Promise<NativeAudioTrackedListEntry[]>;
/**
 * Sets a tracked list from the app, e.g. from its database or a synced copy. Replaces it, or with
 * `merge: true` combines it with the current list: by id, newest `playedAtMs` wins, newest first,
 * capped at the limit. Not reported back as a change. Resolves with the resulting list.
 */
export declare const setTrackedList: (
  id: string,
  entries: NativeAudioTrackedListEntry[],
  options?: { merge?: boolean }
) => Promise<NativeAudioTrackedListEntry[]>;
/** Unacknowledged list changes: the latest one per list. */
export declare const getTrackedListChanges: () => Promise<NativeAudioTrackedListChange[]>;
export declare const acknowledgeTrackedListChanges: (ids: string[]) => Promise<void>;
/**
 * Calls `handler` with tracked list changes: first those from while the app wasn't running (only
 * the latest per list), then live ones. Each is acknowledged after `handler` finishes.
 */
export declare const onTrackedListChanged: (handler: (change: NativeAudioTrackedListChange) => void | Promise<void>) => Promise<() => void>;
export declare const onControlPressed: (handler: (press: NativeAudioControlPress) => void | Promise<void>) => Promise<() => void>;
export declare const getState: () => Promise<NativeAudioState>;
export declare const getProgressCheckpoint: () => Promise<NativeAudioProgressCheckpoint | null>;
export declare const clearProgressCheckpoint: () => Promise<void>;
export declare const dispose: () => Promise<void>;
export declare const addStateListener: (handler: (state: NativeAudioState) => void) => Promise<() => void>;
