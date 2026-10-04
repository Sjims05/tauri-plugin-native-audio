import { addPluginListener, invoke } from '@tauri-apps/api/core';

const PLUGIN_NAME = 'native-audio';
const STATE_EVENT = 'native_audio_state';
const CONTROL_EVENT = 'native_audio_control';
const PLAYBACK_EVENT = 'native_audio_playback_event';
const TRACKED_LIST_EVENT = 'native_audio_tracked_list';

const call = async (command, payload) => {
  return await invoke(`plugin:${PLUGIN_NAME}|${command}`, payload);
};

export const initialize = async () => await call('initialize');
export const setSource = async (payload) => await call('set_source', payload);
export const setQueue = async (payload) => await call('set_queue', payload);
export const next = async () => await call('next');
export const previous = async () => await call('previous');
export const skipTo = async (index) => await call('skip_to', { index });
export const addToQueue = async (items, options = {}) => await call('add_to_queue', { items, playNext: options.playNext ?? false });
export const updateQueue = async (items, options = {}) => await call('update_queue', { items, ...options });
export const removeFromQueue = async (index) => await call('remove_from_queue', { index });
export const moveInQueue = async (from, to) => await call('move_in_queue', { from, to });
export const getQueue = async () => await call('get_queue');
export const restoreLastQueue = async () => (await call('restore_last_queue')) ?? null;
export const play = async () => await call('play');
export const pause = async () => await call('pause');
export const seekTo = async (position) => await call('seek_to', { position });
export const setRate = async (rate) => await call('set_rate', { rate });
export const setVolume = async (volume) => await call('set_volume', { volume });
export const getOutputDevices = async () => await call('get_output_devices');
export const setOutputDevice = async (id) => await call('set_output_device', { id });
export const setSkipInterval = async (seconds) => await call('set_skip_interval', { seconds });
export const setShuffle = async (enabled) => await call('set_shuffle', { enabled });
export const setRepeatMode = async (mode) => await call('set_repeat_mode', { mode });
export const setLibrary = async (library) => await call('set_library', library);
export const setOptions = async (options) => await call('set_options', options);
export const setSleepTimer = async (timer) => await call('set_sleep_timer', timer);
export const cancelSleepTimer = async () => await call('cancel_sleep_timer');
export const getItemProgress = async (itemIds) => (await call('get_item_progress', { itemIds })).entries;
export const setItemProgress = async (entries, options = {}) =>
  (await call('set_item_progress', { entries, merge: options.merge ?? false })).entries;
export const setControls = async (controls) => await call('set_controls', controls);
export const setControlActive = async (buttonId, itemIds, active) => await call('set_control_active', { buttonId, itemIds, active });
export const getControlPresses = async () => (await call('get_control_presses')).presses;
export const acknowledgeControlPresses = async (ids) => await call('acknowledge_control_presses', { ids });

export const getPlaybackEvents = async () => (await call('get_playback_events')).events;
export const acknowledgePlaybackEvents = async (ids) => await call('acknowledge_playback_events', { ids });

// Hands every entry of a saved log to `handler` in order, once per app run: first the entries
// saved while the app wasn't running, then live ones. Each is acknowledged (removed from the log)
// only after the handler finished, so it's delivered again next time if the app closes midway.
const consumeLog = async (eventName, getPending, acknowledge, handler) => {
  const seen = new Set();
  let queue = Promise.resolve();
  const deliver = (entries) => {
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
  const listener = await addPluginListener(PLUGIN_NAME, eventName, (entry) => deliver([entry]));
  await deliver(await getPending());
  return () => listener.unregister();
};

export const onControlPressed = async (handler) =>
  await consumeLog(CONTROL_EVENT, getControlPresses, acknowledgeControlPresses, handler);
export const onPlaybackEvent = async (handler) =>
  await consumeLog(PLAYBACK_EVENT, getPlaybackEvents, acknowledgePlaybackEvents, handler);

export const setTrackedLists = async (lists) => await call('set_tracked_lists', { lists });
export const getTrackedList = async (id) => (await call('get_tracked_list', { id })).entries;
export const setTrackedList = async (id, entries, options = {}) =>
  (await call('set_tracked_list', { id, entries, merge: options.merge ?? false })).entries;
export const getTrackedListChanges = async () => (await call('get_tracked_list_changes')).changes;
export const acknowledgeTrackedListChanges = async (ids) => await call('acknowledge_tracked_list_changes', { ids });
export const onTrackedListChanged = async (handler) =>
  await consumeLog(TRACKED_LIST_EVENT, getTrackedListChanges, acknowledgeTrackedListChanges, handler);
export const getState = async () => await call('get_state');
export const getProgressCheckpoint = async () => await call('get_progress_checkpoint');
export const clearProgressCheckpoint = async () => await call('clear_progress_checkpoint');
export const dispose = async () => await call('dispose');
export const addStateListener = async (handler) => await addPluginListener(PLUGIN_NAME, STATE_EVENT, handler);
