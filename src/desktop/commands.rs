//! The plugin's commands on desktop, with the same names, arguments and results as on Android, so
//! the JavaScript API works unchanged. Android-only features (the Android Auto library, car buttons,
//! options, tracked lists, ...) are accepted and do nothing yet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::State;

use super::engine::Engine;

const STATE_EVENT: &str = "native_audio_state";

/// A queue item, as the JavaScript API sends it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

#[derive(Default)]
struct Inner {
    engine: Option<Engine>,
    items: Vec<Item>,
    listeners: HashMap<String, Vec<Channel<Value>>>,
    emitter_started: bool,
}

/// The desktop player, managed as Tauri state.
#[derive(Clone, Default)]
pub struct DesktopAudio(Arc<Mutex<Inner>>);

type CommandResult<T> = Result<T, String>;

impl DesktopAudio {
    /// The engine, opening the output device the first time.
    fn with_engine<T>(&self, f: impl FnOnce(&Engine, &mut Vec<Item>) -> T) -> CommandResult<T> {
        let mut inner = self.0.lock().unwrap();
        if inner.engine.is_none() {
            inner.engine = Some(Engine::new()?);
        }
        if !inner.emitter_started {
            inner.emitter_started = true;
            let me = self.clone();
            thread::Builder::new()
                .name("native-audio-state".into())
                .spawn(move || me.emit_loop())
                .map_err(|e| e.to_string())?;
        }
        let Inner { engine, items, .. } = &mut *inner;
        Ok(f(engine.as_ref().unwrap(), items))
    }

    /// The state in the shape of the JavaScript API's NativeAudioState.
    fn state(&self) -> Value {
        let inner = self.0.lock().unwrap();
        let length = inner.items.len();
        let Some(engine) = inner.engine.as_ref() else {
            return state_json("idle", 0.0, 0.0, false, -1, length, None, None);
        };
        let status = engine.status();
        let index = status.index.map_or(-1, |i| i as i64);
        let current_id = status.index.and_then(|i| inner.items.get(i)).and_then(|item| item.id);
        let kind = if status.ended && length > 0 {
            "ended"
        } else if status.playing && status.index.is_some() {
            "playing"
        } else {
            "idle"
        };
        state_json(
            kind,
            status.position_secs,
            status.duration_secs.unwrap_or(0.0),
            status.playing && !status.ended,
            index,
            length,
            current_id,
            status.error,
        )
    }

    fn emit(&self, event: &str, payload: &Value) {
        let mut inner = self.0.lock().unwrap();
        if let Some(channels) = inner.listeners.get_mut(event) {
            // A channel whose page is gone fails to send: drop it.
            channels.retain(|channel| channel.send(payload.clone()).is_ok());
        }
    }

    /// Sends the state to listeners when it changes, and the position regularly while playing.
    fn emit_loop(self) {
        let mut last_key = String::new();
        let mut last_sent = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(100));
            let state = self.state();
            let key = format!(
                "{}|{}|{}|{}|{}",
                state["status"], state["queueIndex"], state["isPlaying"], state["queueLength"], state["error"]
            );
            let playing = state["isPlaying"].as_bool() == Some(true);
            if key != last_key || (playing && last_sent.elapsed() >= Duration::from_millis(250)) {
                self.emit(STATE_EVENT, &state);
                last_key = key;
                last_sent = Instant::now();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn state_json(
    status: &str,
    current_time: f64,
    duration: f64,
    is_playing: bool,
    queue_index: i64,
    queue_length: usize,
    current_id: Option<i64>,
    error: Option<String>,
) -> Value {
    let mut state = json!({
        "status": status,
        "currentTime": current_time,
        "duration": duration,
        "isPlaying": is_playing,
        "buffering": false,
        "rate": 1.0,
        "queueIndex": queue_index,
        "queueLength": queue_length,
        "shuffle": false,
        "repeatMode": "off",
        "sleepTimerEndsAtMs": null,
        "sleepTimerEndOfTrack": false,
    });
    if let Some(id) = current_id {
        state["currentId"] = json!(id);
    }
    if let Some(error) = error {
        state["error"] = json!(error);
    }
    state
}

/// A local path from what the app passed: a plain path, or a file:// URL.
fn to_path(src: &str) -> PathBuf {
    let Some(rest) = src.strip_prefix("file://") else { return PathBuf::from(src) };
    // file:///C:/Music/x.mp3 -> C:/Music/x.mp3; also undo %-escapes like %20.
    let rest = if rest.len() > 2 && rest.starts_with('/') && rest.as_bytes()[2] == b':' { &rest[1..] } else { rest };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

fn load(audio: &DesktopAudio, items: Vec<Item>, start_index: usize, start_position: f64) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        let paths = items.iter().map(|item| to_path(&item.src)).collect();
        let index = start_index.min(items.len().saturating_sub(1));
        *queue = items;
        engine.load(paths, index, start_position.max(0.0));
    })?;
    // The engine opens the track on its own thread: give it a moment so the state shows it.
    thread::sleep(Duration::from_millis(30));
    Ok(audio.state())
}

fn not_yet(feature: &str) -> String {
    format!("{feature} isn't available on desktop yet")
}

// ---- playback and the queue

#[tauri::command]
pub fn initialize(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|_, _| ())?;
    Ok(audio.state())
}

#[tauri::command]
pub fn register_listener(audio: State<'_, DesktopAudio>, event: String, handler: Channel<Value>) {
    audio.0.lock().unwrap().listeners.entry(event).or_default().push(handler);
}

#[tauri::command]
pub fn remove_listener(audio: State<'_, DesktopAudio>, event: String, channel_id: u32) {
    if let Some(channels) = audio.0.lock().unwrap().listeners.get_mut(&event) {
        channels.retain(|channel| channel.id() != channel_id);
    }
}

#[tauri::command]
pub fn set_source(
    audio: State<'_, DesktopAudio>,
    src: String,
    id: Option<i64>,
    title: Option<String>,
    artist: Option<String>,
    artwork_url: Option<String>,
) -> CommandResult<Value> {
    load(&audio, vec![Item { src, id, title, artist, artwork_url }], 0, 0.0)
}

#[tauri::command]
pub fn set_queue(
    audio: State<'_, DesktopAudio>,
    items: Vec<Item>,
    start_index: Option<usize>,
    start_position: Option<f64>,
) -> CommandResult<Value> {
    load(&audio, items, start_index.unwrap_or(0), start_position.unwrap_or(0.0))
}

#[tauri::command]
pub fn play(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, _| {
        // Finished: play the queue again from the start.
        if engine.status().ended {
            engine.skip_to(0);
        }
        engine.play();
    })?;
    Ok(audio.state())
}

#[tauri::command]
pub fn pause(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, _| engine.pause())?;
    Ok(audio.state())
}

#[tauri::command]
pub fn seek_to(audio: State<'_, DesktopAudio>, position: f64) -> CommandResult<Value> {
    audio.with_engine(|engine, _| engine.seek(position))?;
    thread::sleep(Duration::from_millis(30));
    Ok(audio.state())
}

#[tauri::command]
pub fn next(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, items| {
        if let Some(index) = engine.status().index {
            if index + 1 < items.len() {
                engine.skip_to(index + 1);
            }
        }
    })?;
    thread::sleep(Duration::from_millis(30));
    Ok(audio.state())
}

#[tauri::command]
pub fn previous(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, _| {
        let status = engine.status();
        match status.index {
            // Like on Android: past the first 3 seconds, previous restarts the track.
            Some(index) if status.position_secs <= 3.0 && index > 0 => engine.skip_to(index - 1),
            Some(_) => engine.seek(0.0),
            None => {}
        }
    })?;
    thread::sleep(Duration::from_millis(30));
    Ok(audio.state())
}

#[tauri::command]
pub fn skip_to(audio: State<'_, DesktopAudio>, index: usize) -> CommandResult<Value> {
    audio.with_engine(|engine, items| {
        if index < items.len() {
            engine.skip_to(index);
        }
    })?;
    thread::sleep(Duration::from_millis(30));
    Ok(audio.state())
}

#[tauri::command]
pub fn get_state(audio: State<'_, DesktopAudio>) -> Value {
    audio.state()
}

#[tauri::command]
pub fn get_queue(audio: State<'_, DesktopAudio>) -> Value {
    let state = audio.state();
    let items = audio.0.lock().unwrap().items.clone();
    json!({
        "items": items,
        "currentIndex": state["queueIndex"],
        "playOrder": (0..items.len()).collect::<Vec<_>>(),
    })
}

#[tauri::command]
pub fn restore_last_queue() -> Option<Value> {
    None
}

#[tauri::command]
pub fn dispose(audio: State<'_, DesktopAudio>) {
    let mut inner = audio.0.lock().unwrap();
    inner.engine = None;
    inner.items.clear();
}

// ---- coming with the desktop queue (shuffle, repeat, editing) and later steps

#[tauri::command]
pub fn set_shuffle() -> CommandResult<Value> {
    Err(not_yet("Shuffle"))
}

#[tauri::command]
pub fn set_repeat_mode() -> CommandResult<Value> {
    Err(not_yet("Repeat"))
}

#[tauri::command]
pub fn add_to_queue() -> CommandResult<Value> {
    Err(not_yet("Queue editing"))
}

#[tauri::command]
pub fn remove_from_queue() -> CommandResult<Value> {
    Err(not_yet("Queue editing"))
}

#[tauri::command]
pub fn move_in_queue() -> CommandResult<Value> {
    Err(not_yet("Queue editing"))
}

#[tauri::command]
pub fn set_rate() -> CommandResult<Value> {
    Err(not_yet("Playback speed"))
}

#[tauri::command]
pub fn set_sleep_timer() -> CommandResult<Value> {
    Err(not_yet("The sleep timer"))
}

#[tauri::command]
pub fn cancel_sleep_timer(audio: State<'_, DesktopAudio>) -> Value {
    audio.state()
}

// ---- Android-only (Android Auto, the media notification's buttons), or not on desktop yet:
// accepted so the same app code runs on every platform.

#[tauri::command]
pub fn set_skip_interval() {}

#[tauri::command]
pub fn set_library() {}

#[tauri::command]
pub fn set_options() {}

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
