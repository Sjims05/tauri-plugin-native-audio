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
  /** Defaults to 0. */
  startIndex?: number;
  /** Seconds into the start item. Defaults to 0. */
  startPosition?: number;
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
export declare const getState: () => Promise<NativeAudioState>;
export declare const getProgressCheckpoint: () => Promise<NativeAudioProgressCheckpoint | null>;
export declare const clearProgressCheckpoint: () => Promise<void>;
export declare const dispose: () => Promise<void>;
export declare const addStateListener: (handler: (state: NativeAudioState) => void) => Promise<() => void>;
