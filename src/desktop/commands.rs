//! The plugin's commands on desktop, with the same names, arguments and results as on Android, so
//! the JavaScript API works unchanged. They hand over to the player (player.rs). Android-only
//! features (the Android Auto library, car buttons, tracked lists, ...) are accepted and do nothing yet.

use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::State;

use super::player::{DesktopAudio, Item, Result};

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
    audio.load(vec![Item { src, id, title, artist, artwork_url }], 0, 0.0)
}

#[tauri::command]
pub fn set_queue(
    audio: State<'_, DesktopAudio>,
    items: Vec<Item>,
    start_index: Option<usize>,
    start_position: Option<f64>,
) -> Result<Value> {
    audio.load(items, start_index.unwrap_or(0), start_position.unwrap_or(0.0))
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
pub fn restore_last_queue() -> Option<Value> {
    None
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

/// Desktop uses repeatAddedTracks, mediaControls and volumeCurve; the rest are Android's.
#[tauri::command]
pub fn set_options(
    audio: State<'_, DesktopAudio>,
    repeat_added_tracks: Option<bool>,
    media_controls: Option<bool>,
    volume_curve: Option<String>,
) -> Result<()> {
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
pub fn set_sleep_timer() -> Result<Value> {
    Err(not_yet("The sleep timer"))
}

#[tauri::command]
pub fn cancel_sleep_timer(audio: State<'_, DesktopAudio>) -> Value {
    audio.state()
}

// ---- Android-only (Android Auto, car buttons), or not on desktop yet: accepted so the same app
// code runs on every platform.

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
pub fn get_playback_events() -> Value {
    json!({ "events": [] })
}

#[tauri::command]
pub fn acknowledge_playback_events() {}

#[tauri::command]
pub fn set_tracked_lists() {}

#[tauri::command]
pub fn get_tracked_list() -> Value {
    json!({ "entries": [] })
}

#[tauri::command]
pub fn set_tracked_list(entries: Value) -> Value {
    json!({ "entries": entries })
}

#[tauri::command]
pub fn get_tracked_list_changes() -> Value {
    json!({ "changes": [] })
}

#[tauri::command]
pub fn acknowledge_tracked_list_changes() {}

#[tauri::command]
pub fn get_item_progress() -> Value {
    json!({ "entries": [] })
}

#[tauri::command]
pub fn set_item_progress(entries: Value) -> Value {
    json!({ "entries": entries })
}

#[tauri::command]
pub fn get_progress_checkpoint() -> Option<Value> {
    None
}

#[tauri::command]
pub fn clear_progress_checkpoint() {}

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
