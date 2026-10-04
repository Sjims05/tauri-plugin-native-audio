//! The plugin's commands on desktop, with the same names, arguments and results as on Android, so
//! the JavaScript API works unchanged. They hand over to the player (player.rs). Android-only
//! features (the Android Auto library, car buttons) are accepted and do nothing.

use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::State;

use super::persist::{ListConfig, ListEntry};
use super::player::{DesktopAudio, Item, ProgressEntry, Result};
use super::queue::{NewItems, UpdateOptions};

fn not_yet(feature: &str) -> String {
    format!("{feature} isn't available on desktop yet")
}

// ---- playback and the queue

#[tauri::command]
pub fn initialize(audio: State<'_, DesktopAudio>) -> Result<Value> {
    audio.initialize()
}

#[tauri::command]
pub fn register_listener(audio: State<'_, DesktopAudio>, event: String, handler: Channel<Value>) {
    audio.add_listener(event, handler);
}

#[tauri::command]
pub fn remove_listener(audio: State<'_, DesktopAudio>, event: String, channel_id: u32) {
    audio.remove_listener(&event, channel_id);
}

#[tauri::command]
pub fn set_source(
    audio: State<'_, DesktopAudio>,
    src: String,
    id: Option<i64>,
    title: Option<String>,
    artist: Option<String>,
    artwork_url: Option<String>,
) -> Result<Value> {
    audio.load(vec![Item { src, id, title, artist, artwork_url }], 0, 0.0, None)
}

#[tauri::command]
pub fn set_queue(
    audio: State<'_, DesktopAudio>,
    items: Vec<Item>,
    start_index: Option<usize>,
    start_position: Option<f64>,
    source_id: Option<String>,
) -> Result<Value> {
    audio.load(items, start_index.unwrap_or(0), start_position.unwrap_or(0.0), source_id)
}

#[tauri::command]
pub fn play(audio: State<'_, DesktopAudio>) -> Result<Value> {
    audio.play()
}

#[tauri::command]
pub fn pause(audio: State<'_, DesktopAudio>) -> Result<Value> {
    audio.pause()
}

#[tauri::command]
pub fn seek_to(audio: State<'_, DesktopAudio>, position: f64) -> Result<Value> {
    audio.seek_to(position)
}

#[tauri::command]
pub fn next(audio: State<'_, DesktopAudio>) -> Result<Value> {
    audio.next()
}

#[tauri::command]
pub fn previous(audio: State<'_, DesktopAudio>) -> Result<Value> {
    audio.previous()
}

#[tauri::command]
pub fn skip_to(audio: State<'_, DesktopAudio>, index: usize) -> Result<Value> {
    audio.skip_to(index)
}

#[tauri::command]
pub fn set_shuffle(audio: State<'_, DesktopAudio>, enabled: bool) -> Result<Value> {
    audio.set_shuffle(enabled)
}

#[tauri::command]
pub fn set_repeat_mode(audio: State<'_, DesktopAudio>, mode: String) -> Result<Value> {
    audio.set_repeat_mode(&mode)
}

#[tauri::command]
pub fn add_to_queue(audio: State<'_, DesktopAudio>, items: Vec<Item>, play_next: Option<bool>) -> Result<Value> {
    audio.add_to_queue(items, play_next.unwrap_or(false))
}

/// `removed_current`: "finish" (default) or "skip"; `removed_items`: "remove" (default) or "keep";
/// `new_items`: "inPlace" (default), "end" or "next"; `order`: "playlist" (default) or "queue".
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn update_queue(
    audio: State<'_, DesktopAudio>,
    items: Vec<Item>,
    source_id: Option<String>,
    removed_current: Option<String>,
    removed_items: Option<String>,
    new_items: Option<String>,
    order: Option<String>,
) -> Result<Value> {
    let pick = |value: Option<String>, name: &str, default: &str, other: &str| -> Result<bool> {
        match value.as_deref().unwrap_or(default) {
            v if v == default => Ok(false),
            v if v == other => Ok(true),
            v => Err(format!("{name} must be {default} or {other}, not \"{v}\"")),
        }
    };
    let skip_removed_current = pick(removed_current, "removedCurrent", "finish", "skip")?;
    let new_items = match new_items.as_deref().unwrap_or("inPlace") {
        "inPlace" => NewItems::InPlace,
        "end" => NewItems::End,
        "next" => NewItems::Next,
        other => return Err(format!("newItems must be inPlace, end or next, not \"{other}\"")),
    };
    let options = UpdateOptions {
        keep_removed: pick(removed_items, "removedItems", "remove", "keep")?,
        new_items,
        keep_queue_order: pick(order, "order", "playlist", "queue")?,
    };
    audio.update_queue(items, source_id, options, skip_removed_current)
}

#[tauri::command]
pub fn remove_from_queue(audio: State<'_, DesktopAudio>, index: usize) -> Result<Value> {
    audio.remove_from_queue(index)
}

#[tauri::command]
pub fn move_in_queue(audio: State<'_, DesktopAudio>, from: usize, to: usize) -> Result<Value> {
    audio.move_in_queue(from, to)
}

#[tauri::command]
pub fn get_state(audio: State<'_, DesktopAudio>) -> Value {
    audio.state()
}

#[tauri::command]
pub fn get_queue(audio: State<'_, DesktopAudio>) -> Value {
    audio.queue()
}

#[tauri::command]
pub fn restore_last_queue(audio: State<'_, DesktopAudio>) -> Result<Option<Value>> {
    audio.restore_last_queue()
}

#[tauri::command]
pub fn dispose(audio: State<'_, DesktopAudio>) {
    audio.dispose();
}

/// The previous / next media keys seek by this many seconds when above 0.
#[tauri::command]
pub fn set_skip_interval(audio: State<'_, DesktopAudio>, seconds: f64) -> Value {
    audio.set_skip_interval(seconds)
}

/// Desktop uses repeatAddedTracks, trackProgress, mediaControls, volumeCurve, resumeLastQueue,
/// previousRestartsAfterSeconds and nextAtEnd (saved, like on Android); the rest are Android's.
/// Tauri passes each option as its own argument.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn set_options(
    audio: State<'_, DesktopAudio>,
    repeat_added_tracks: Option<bool>,
    track_progress: Option<bool>,
    media_controls: Option<bool>,
    volume_curve: Option<String>,
    resume_last_queue: Option<Value>,
    previous_restarts_after_seconds: Option<f64>,
    next_at_end: Option<String>,
) -> Result<()> {
    audio.set_skip_rules(previous_restarts_after_seconds, next_at_end.as_deref())?;
    if let Some(mode) = resume_last_queue {
        audio.set_resume_last_queue(mode)?;
    }
    if let Some(on) = track_progress {
        audio.set_track_progress(on);
    }
    if let Some(curve) = volume_curve {
        audio.set_volume_curve(&curve)?;
    }
    if let Some(on) = repeat_added_tracks {
        audio.set_repeat_added(on);
    }
    if let Some(on) = media_controls {
        audio.set_media_controls(on);
    }
    Ok(())
}

// ---- output

#[tauri::command]
pub fn set_volume(audio: State<'_, DesktopAudio>, volume: f64) -> Value {
    audio.set_volume(volume)
}

#[tauri::command]
pub fn get_output_devices(audio: State<'_, DesktopAudio>) -> Value {
    audio.output_devices()
}

#[tauri::command]
pub fn set_output_device(audio: State<'_, DesktopAudio>, id: Option<String>) -> Result<Value> {
    audio.set_output_device(id)
}

// ---- coming in later steps

#[tauri::command]
pub fn set_rate() -> Result<Value> {
    Err(not_yet("Playback speed"))
}

#[tauri::command]
pub fn set_sleep_timer(
    audio: State<'_, DesktopAudio>,
    minutes: Option<f64>,
    end_of_track: Option<bool>,
    fade_out_seconds: Option<f64>,
) -> Result<Value> {
    audio.set_sleep_timer(minutes, end_of_track.unwrap_or(false), fade_out_seconds)
}

#[tauri::command]
pub fn cancel_sleep_timer(audio: State<'_, DesktopAudio>) -> Value {
    audio.cancel_sleep_timer()
}

// ---- Android-only (Android Auto, car buttons): accepted so the same app code runs on every
// platform. Then what's saved: playback events, tracked lists, item progress, the checkpoint.

#[tauri::command]
pub fn set_library() {}

#[tauri::command]
pub fn set_controls() {}

#[tauri::command]
pub fn set_control_active() {}

#[tauri::command]
pub fn get_control_presses() -> Value {
    json!({ "presses": [] })
}

#[tauri::command]
pub fn acknowledge_control_presses() {}

#[tauri::command]
pub fn get_playback_events(audio: State<'_, DesktopAudio>) -> Value {
    audio.playback_events()
}

#[tauri::command]
pub fn acknowledge_playback_events(audio: State<'_, DesktopAudio>, ids: Vec<String>) {
    audio.acknowledge_playback_events(&ids);
}

#[tauri::command]
pub fn set_tracked_lists(audio: State<'_, DesktopAudio>, lists: Vec<ListConfig>) -> Result<()> {
    audio.set_tracked_lists(lists)
}

#[tauri::command]
pub fn get_tracked_list(audio: State<'_, DesktopAudio>, id: String) -> Value {
    audio.tracked_list(&id)
}

#[tauri::command]
pub fn set_tracked_list(audio: State<'_, DesktopAudio>, id: String, entries: Vec<ListEntry>, merge: Option<bool>) -> Result<Value> {
    audio.set_tracked_list(&id, entries, merge.unwrap_or(false))
}

#[tauri::command]
pub fn get_tracked_list_changes(audio: State<'_, DesktopAudio>) -> Value {
    audio.tracked_list_changes()
}

#[tauri::command]
pub fn acknowledge_tracked_list_changes(audio: State<'_, DesktopAudio>, ids: Vec<String>) {
    audio.acknowledge_tracked_list_changes(&ids);
}

#[tauri::command]
pub fn get_item_progress(audio: State<'_, DesktopAudio>, item_ids: Option<Vec<i64>>) -> Value {
    audio.item_progress(item_ids)
}

#[tauri::command]
pub fn set_item_progress(audio: State<'_, DesktopAudio>, entries: Vec<ProgressEntry>, merge: Option<bool>) -> Result<Value> {
    audio.set_item_progress(entries, merge.unwrap_or(false))
}

#[tauri::command]
pub fn get_progress_checkpoint(audio: State<'_, DesktopAudio>) -> Option<Value> {
    audio.progress_checkpoint()
}

#[tauri::command]
pub fn clear_progress_checkpoint(audio: State<'_, DesktopAudio>) {
    audio.clear_progress_checkpoint();
}

/// Every command, for the plugin's invoke handler.
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        initialize,
        register_listener,
        remove_listener,
        set_source,
        set_queue,
        play,
        pause,
        seek_to,
        next,
        previous,
        skip_to,
        get_state,
        get_queue,
        restore_last_queue,
        dispose,
        set_shuffle,
        set_repeat_mode,
        add_to_queue,
        update_queue,
        remove_from_queue,
        move_in_queue,
        set_volume,
        get_output_devices,
        set_output_device,
        set_rate,
        set_sleep_timer,
        cancel_sleep_timer,
        set_skip_interval,
        set_library,
        set_options,
        set_controls,
        set_control_active,
        get_control_presses,
        acknowledge_control_presses,
        get_playback_events,
        acknowledge_playback_events,
        set_tracked_lists,
        get_tracked_list,
        set_tracked_list,
        get_tracked_list_changes,
        acknowledge_tracked_list_changes,
        get_item_progress,
        set_item_progress,
        get_progress_checkpoint,
        clear_progress_checkpoint,
    ]
}
