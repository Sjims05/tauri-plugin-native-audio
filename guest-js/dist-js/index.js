/**
 * JavaScript API of tauri-plugin-native-audio: native playback on Android (Media3) and desktop (the
 * plugin's Rust player). Built into dist-js with `npm run build` (the built files are committed, since
 * apps install the plugin straight from GitHub).
 */
import { addPluginListener, invoke } from '@tauri-apps/api/core';
const PLUGIN_NAME = 'native-audio';
const STATE_EVENT = 'native_audio_state';
const CONTROL_EVENT = 'native_audio_control';
const PLAYBACK_EVENT = 'native_audio_playback_event';
const TRACKED_LIST_EVENT = 'native_audio_tracked_list';
const call = async (command, payload) => await invoke(`plugin:${PLUGIN_NAME}|${command}`, payload);
// Hands every entry of a saved log to `handler` in order, once per app run: first the entries
// saved while the app wasn't running, then live ones. Each is acknowledged (removed from the log)
// only after the handler finished, so it's delivered again next time if the app closes midway.
const consumeLog = async (eventName, getPending, acknowledge, handler) => {
    const seen = new Set();
    let queue = Promise.resolve();
    const deliver = (entries) => {
        queue = queue.then(async () => {
            for (const entry of entries) {
                if (seen.has(entry.id))
                    continue;
                seen.add(entry.id);
                try {
                    await handler(entry);
                    await acknowledge([entry.id]);
                }
                catch (error) {
                    seen.delete(entry.id);
                    console.error(`native-audio: ${eventName} handler failed`, error);
                }
            }
        });
        return queue;
    };
    const listener = await addPluginListener(PLUGIN_NAME, eventName, (entry) => deliver([entry]));
    await deliver(await getPending());
    return () => void listener.unregister();
};
// ---- Setup and state
/** Call once, when the app's UI is ready (on desktop the last queue is resumed here, see `resumeLastQueue`). */
export const initialize = async () => await call('initialize');
export const getState = async () => await call('get_state');
/** Calls `handler` on every state change, and regularly while playing. Resolves to a function that stops listening. */
export const addStateListener = async (handler) => {
    const listener = await addPluginListener(PLUGIN_NAME, STATE_EVENT, handler);
    return () => void listener.unregister();
};
/** Saved on the device (desktop too); only the options passed are changed. */
export const setOptions = async (options) => await call('set_options', options);
/** Stops playback and empties the queue. The saved queue stays, for `restoreLastQueue()`. */
export const dispose = async () => await call('dispose');
// ---- The queue
/** A queue with a single item. */
export const setSource = async (payload) => await call('set_source', payload);
export const setQueue = async (payload) => await call('set_queue', payload);
/**
 * Adds items to the playing queue: at the end, or right after the current track with `playNext`.
 * With shuffle on they get the same spot in the play order, and the rest of the order is kept.
 * With an empty queue, it becomes the queue.
 */
export const addToQueue = async (items, options = {}) => await call('add_to_queue', { items, playNext: options.playNext ?? false });
/**
 * The playlist (or album) the queue plays changed, e.g. after a sync: `items` is its new content.
 * Entries are matched by `id` (the same id twice: in order). What's still there keeps its place,
 * also in a shuffled play order; new tracks are added and missing ones removed (see the options).
 * Tracks added with `addToQueue` always stay, after the track they followed. The playing track's
 * details (title, artwork) are updated without interrupting it. With an empty queue, `items`
 * becomes the queue. Resolves with the state and how many tracks were added and removed.
 */
export const updateQueue = async (items, options = {}) => await call('update_queue', { items, ...options });
/** Removes the entry at a queue index. Removing the current track moves on to the next one. */
export const removeFromQueue = async (index) => await call('remove_from_queue', { index });
/**
 * Moves a queue entry between queue indices. Without shuffle this changes the play order; with
 * shuffle on, the play order is kept (only the list position changes).
 */
export const moveInQueue = async (from, to) => await call('move_in_queue', { from, to });
export const getQueue = async () => await call('get_queue');
/**
 * Loads the last queue (tracks, current track and position, shuffle order, repeat mode), paused.
 * It's saved automatically as it plays and changes. Resolves with null when nothing was saved.
 * When a queue is already loaded (for example playing from Android Auto), it's left alone and this
 * resolves with the current state, so it's safe to call every time the app starts.
 */
export const restoreLastQueue = async () => (await call('restore_last_queue')) ?? null;
// ---- Playback
export const play = async () => await call('play');
export const pause = async () => await call('pause');
/** Seconds into the current track. Past the end seeks to the end, from where playback moves on. */
export const seekTo = async (position) => await call('seek_to', { position });
export const next = async () => await call('next');
export const previous = async () => await call('previous');
export const skipTo = async (index) => await call('skip_to', { index });
/**
 * Plays the queue in a shuffled order that starts with the current track. Every track plays once
 * per pass, and with repeat `all` each pass gets a new order. `queueIndex` stays the index in `items`.
 */
export const setShuffle = async (enabled) => await call('set_shuffle', { enabled });
export const setRepeatMode = async (mode) => await call('set_repeat_mode', { mode });
/**
 * What the previous / next buttons in the notification, lock screen and headset (desktop: the media
 * keys and the system's media controls) do.
 * `0` (the default) skips whole tracks in the queue; above `0` seeks back / forward by that many seconds.
 */
export const setSkipInterval = async (seconds) => await call('set_skip_interval', { seconds });
/** Pauses playback after some minutes (fading out) or at the end of the current track. Replaces a running timer. */
export const setSleepTimer = async (timer) => await call('set_sleep_timer', timer);
export const cancelSleepTimer = async () => await call('cancel_sleep_timer');
/** Playback speed (Android, iOS). Rejects on desktop. */
export const setRate = async (rate) => await call('set_rate', { rate });
/**
 * The player's own volume, 0 to 1 (a slider's position), on top of the system volume. Default 1.
 * What's heard follows the `volumeCurve` option. Saved for the next run.
 */
export const setVolume = async (volume) => await call('set_volume', { volume });
/**
 * Desktop: the audio outputs to choose from. Android picks the output itself (speaker, headphones,
 * Bluetooth, the car) and returns an empty list.
 */
export const getOutputDevices = async () => await call('get_output_devices');
/**
 * Desktop: plays on this output (an `id` from `getOutputDevices`), or follows the system default
 * with null (the default; it also moves along when the default changes). When the chosen device goes
 * away, playback moves to the default and comes back once it's there again. Saved: the next run
 * plays on it too. Android: does nothing.
 */
export const setOutputDevice = async (id) => await call('set_output_device', { id });
// ---- Syncing with the app
/**
 * Declares lists the plugin keeps up to date natively, e.g. the last 5 playlists played: most
 * recent first, no duplicates, capped at `limit`. They keep updating while the app isn't running
 * (Android Auto). Lists no longer declared are dropped. Saved on the device.
 */
export const setTrackedLists = async (lists) => await call('set_tracked_lists', { lists });
/** A tracked list's current entries, newest first. */
export const getTrackedList = async (id) => (await call('get_tracked_list', { id })).entries;
/**
 * Sets a tracked list from the app, e.g. from its database or a synced copy. Replaces it, or with
 * `merge: true` combines it with the current list: by id, newest `playedAtMs` wins, newest first,
 * capped at the limit. Not reported back as a change. Resolves with the resulting list.
 */
export const setTrackedList = async (id, entries, options = {}) => (await call('set_tracked_list', { id, entries, merge: options.merge ?? false })).entries;
/** Unacknowledged list changes: the latest one per list. */
export const getTrackedListChanges = async () => (await call('get_tracked_list_changes')).changes;
export const acknowledgeTrackedListChanges = async (ids) => await call('acknowledge_tracked_list_changes', { ids });
/**
 * Calls `handler` with tracked list changes: first those from while the app wasn't running (only
 * the latest per list), then live ones. Each is acknowledged after `handler` finishes.
 */
export const onTrackedListChanged = async (handler) => await consumeLog(TRACKED_LIST_EVENT, getTrackedListChanges, acknowledgeTrackedListChanges, handler);
/** Logged playback events not acknowledged yet, oldest first. */
export const getPlaybackEvents = async () => (await call('get_playback_events')).events;
export const acknowledgePlaybackEvents = async (ids) => await call('acknowledge_playback_events', { ids });
/**
 * Calls `handler` for every playback event (track started / completed / skipped): first those
 * logged while the app wasn't running (e.g. playing from Android Auto), then live ones. Each is
 * acknowledged after `handler` finishes. Use it for recently played, play counts, progress or
 * scrobbling. Returns a function to stop listening.
 */
export const onPlaybackEvent = async (handler) => await consumeLog(PLAYBACK_EVENT, getPlaybackEvents, acknowledgePlaybackEvents, handler);
/** Recorded progress (setOptions `trackProgress`), of all items or the given ones. */
export const getItemProgress = async (itemIds) => (await call('get_item_progress', { itemIds })).entries;
/**
 * Sets progress from the app, e.g. "mark as played" or a synced copy. Replaces the given items'
 * progress, or with `merge: true` keeps whichever version is newer. Resolves with the result.
 */
export const setItemProgress = async (entries, options = {}) => (await call('set_item_progress', { entries, merge: options.merge ?? false })).entries;
export const getProgressCheckpoint = async () => (await call('get_progress_checkpoint')) ?? null;
export const clearProgressCheckpoint = async () => await call('clear_progress_checkpoint');
// ---- Android Auto and the Android media controls
/**
 * The browsable library shown in Android Auto (with `plugins.native-audio.carSupport` enabled).
 * Saved natively, so Auto can browse and play it while the app isn't running. Replaces the
 * previous library. Elsewhere it's accepted and not used.
 */
export const setLibrary = async (library) => await call('set_library', library);
/**
 * Extra buttons for Android Auto and the Android 13+ media controls (Android, with carSupport).
 * Saved on the device, so they're there even when Android Auto starts the app in the background.
 */
export const setControls = async (controls) => await call('set_controls', controls);
/** Sets a toggle button's state for tracks, e.g. from the app's own liked songs. */
export const setControlActive = async (buttonId, itemIds, active) => await call('set_control_active', { buttonId, itemIds, active });
/** Logged custom button presses not acknowledged yet, oldest first. */
export const getControlPresses = async () => (await call('get_control_presses')).presses;
export const acknowledgeControlPresses = async (ids) => await call('acknowledge_control_presses', { ids });
/**
 * Calls `handler` for every custom button press: first those logged while the app wasn't running,
 * then live ones. Each is acknowledged after `handler` finishes. Returns a function to stop listening.
 */
export const onControlPressed = async (handler) => await consumeLog(CONTROL_EVENT, getControlPresses, acknowledgeControlPresses, handler);
