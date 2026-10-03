//! The plugin's commands on desktop, with the same names, arguments and results as on Android, so
//! the JavaScript API works unchanged. Android-only features (the Android Auto library, car buttons,
//! tracked lists, ...) are accepted and do nothing yet.

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
use super::pipeline::{NextTrack, Track};
use super::queue::{Queue, Repeat};

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
    listeners: HashMap<String, Vec<Channel<Value>>>,
    emitter_started: bool,
    /// The playing entry when the emit loop last looked, to notice track changes.
    last_key: Option<u64>,
}

/// The desktop player, managed as Tauri state. Locks: `inner` before `queue`, never the other way
/// round. The engine's feeder thread takes only `queue` (to ask what follows a track).
#[derive(Clone, Default)]
pub struct DesktopAudio {
    inner: Arc<Mutex<Inner>>,
    queue: Arc<Mutex<Queue>>,
}

type CommandResult<T> = Result<T, String>;

impl DesktopAudio {
    /// The engine and the queue, opening the output device the first time.
    fn with_engine<T>(&self, f: impl FnOnce(&Engine, &mut Queue) -> T) -> CommandResult<T> {
        let mut inner = self.inner.lock().unwrap();
        if inner.engine.is_none() {
            inner.engine = Some(Engine::new(self.next_track())?);
        }
        if !inner.emitter_started {
            inner.emitter_started = true;
            let me = self.clone();
            thread::Builder::new()
                .name("native-audio-state".into())
                .spawn(move || me.emit_loop())
                .map_err(|e| e.to_string())?;
        }
        let mut queue = self.queue.lock().unwrap();
        Ok(f(inner.engine.as_ref().unwrap(), &mut queue))
    }

    /// What the engine plays after each track: the queue's next entry (repeat one: the same again).
    fn next_track(&self) -> NextTrack {
        let queue = self.queue.clone();
        Arc::new(move |key| {
            let queue = queue.lock().unwrap();
            queue.peek_next(key, true).and_then(|next| track_of(&queue, next))
        })
    }

    /// The state in the shape of the JavaScript API's NativeAudioState.
    fn state(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let queue = self.queue.lock().unwrap();
        let mut state = PlayerState {
            status: "idle",
            current_time: 0.0,
            duration: 0.0,
            is_playing: false,
            queue_index: -1,
            queue_length: queue.len(),
            current_id: None,
            shuffle: queue.shuffle,
            repeat: queue.repeat,
            error: None,
        };
        let Some(engine) = inner.engine.as_ref() else { return state.to_json() };
        let status = engine.status();
        let index = status.key.and_then(|key| queue.index_of(key));
        let loaded = queue.len() > 0;
        // A track that can't be played is skipped; "error" when that left nothing to play.
        state.status = if status.ended && status.error.is_some() {
            "error"
        } else if status.ended && loaded {
            "ended"
        } else if status.loading && loaded {
            "loading"
        } else if status.playing && index.is_some() {
            "playing"
        } else {
            "idle"
        };
        state.current_time = status.position_secs;
        state.duration = status.duration_secs.unwrap_or(0.0);
        state.is_playing = status.playing && !status.ended && index.is_some();
        state.queue_index = index.map_or(-1, |i| i as i64);
        state.current_id = index.and_then(|i| queue.entries()[i].item.id);
        state.error = status.error;
        state.to_json()
    }

    fn emit(&self, event: &str, payload: &Value) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(channels) = inner.listeners.get_mut(event) {
            // A channel whose page is gone fails to send: drop it.
            channels.retain(|channel| channel.send(payload.clone()).is_ok());
        }
    }

    /// A new track started playing (on its own, or skipped to): the queue may start over (repeat all).
    fn check_transition(&self) {
        let mut inner = self.inner.lock().unwrap();
        let Some(engine) = inner.engine.as_ref() else { return };
        let key = engine.status().key;
        if key == inner.last_key {
            return;
        }
        if let (Some(from), Some(to)) = (inner.last_key, key) {
            let mut queue = self.queue.lock().unwrap();
            queue.on_transition(from, to);
            resync(engine, &queue);
        }
        inner.last_key = key;
    }

    /// Sends the state to listeners when it changes, and the position regularly while playing.
    fn emit_loop(self) {
        let mut last_key = String::new();
        let mut last_sent = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(100));
            self.check_transition();
            let state = self.state();
            let key = format!(
                "{}|{}|{}|{}|{}|{}|{}",
                state["status"],
                state["queueIndex"],
                state["isPlaying"],
                state["queueLength"],
                state["shuffle"],
                state["repeatMode"],
                state["error"]
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

/// The JavaScript API's NativeAudioState.
struct PlayerState {
    status: &'static str,
    current_time: f64,
    duration: f64,
    is_playing: bool,
    queue_index: i64,
    queue_length: usize,
    current_id: Option<i64>,
    shuffle: bool,
    repeat: Repeat,
    error: Option<String>,
}

impl PlayerState {
    fn to_json(self) -> Value {
        let mut state = json!({
            "status": self.status,
            "currentTime": self.current_time,
            "duration": self.duration,
            "isPlaying": self.is_playing,
            "buffering": false,
            "rate": 1.0,
            "queueIndex": self.queue_index,
            "queueLength": self.queue_length,
            "shuffle": self.shuffle,
            "repeatMode": self.repeat.name(),
            "sleepTimerEndsAtMs": null,
            "sleepTimerEndOfTrack": false,
        });
        if let Some(id) = self.current_id {
            state["currentId"] = json!(id);
        }
        if let Some(error) = self.error {
            state["error"] = json!(error);
        }
        state
    }
}

/// The queue entry `key` as a track for the engine.
fn track_of(queue: &Queue, key: u64) -> Option<Track> {
    queue.entry(key).map(|entry| Track { key, path: to_path(&entry.item.src) })
}

/// After the queue changed: if the engine already prepared a different track to follow the playing
/// one (it does so a couple of seconds before the end), plays on from the same spot so the right
/// one follows. Otherwise there's nothing to do: the engine asks the queue when it gets there.
fn resync(engine: &Engine, queue: &Queue) {
    let status = engine.status();
    let Some(playing) = status.key else { return };
    if status.ended {
        return;
    }
    let prepared = match (status.upcoming, status.end_prepared) {
        (Some(upcoming), _) => Some(upcoming),
        (None, true) => None,
        (None, false) => return,
    };
    if queue.peek_next(playing, true) != prepared {
        if let Some(track) = track_of(queue, playing) {
            engine.start(track, status.position_secs);
        }
    }
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

/// The state once the engine has opened what a command asked for (on its own thread), so it shows
/// the new track. Waits without holding the locks: opening the track may need the queue.
fn settled(audio: &DesktopAudio) -> Value {
    let waiter = audio.inner.lock().unwrap().engine.as_ref().map(Engine::waiter);
    if let Some(waiter) = waiter {
        waiter.wait(Duration::from_millis(500));
    }
    audio.state()
}

fn load(audio: &DesktopAudio, items: Vec<Item>, start_index: usize, start_position: f64) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| match queue.set(items, start_index).and_then(|key| track_of(queue, key)) {
        Some(track) => engine.start(track, start_position.max(0.0)),
        None => engine.stop(),
    })?;
    Ok(settled(audio))
}

/// Starts the queue entry `key` from the beginning.
fn start_key(engine: &Engine, queue: &Queue, key: Option<u64>) {
    if let Some(track) = key.and_then(|key| track_of(queue, key)) {
        engine.start(track, 0.0);
    }
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
    audio.inner.lock().unwrap().listeners.entry(event).or_default().push(handler);
}

#[tauri::command]
pub fn remove_listener(audio: State<'_, DesktopAudio>, event: String, channel_id: u32) {
    if let Some(channels) = audio.inner.lock().unwrap().listeners.get_mut(&event) {
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
    audio.with_engine(|engine, queue| {
        if queue.len() == 0 {
            return;
        }
        // Finished: play the queue again from the start of the play order.
        if engine.status().ended {
            start_key(engine, queue, queue.order().first().and_then(|&i| queue.key_at(i)));
        }
        engine.play();
    })?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn pause(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, _| engine.pause())?;
    Ok(audio.state())
}

#[tauri::command]
pub fn seek_to(audio: State<'_, DesktopAudio>, position: f64) -> CommandResult<Value> {
    audio.with_engine(|engine, _| {
        // Past the end: the end, from where it moves on as usual.
        let last = engine.status().duration_secs.filter(|d| *d > 0.0).map_or(f64::MAX, |d| (d - 0.01).max(0.0));
        engine.seek(position.clamp(0.0, last));
    })?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn next(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        if let Some(key) = engine.status().key {
            start_key(engine, queue, queue.peek_next(key, false));
        }
    })?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn previous(audio: State<'_, DesktopAudio>) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        let status = engine.status();
        let Some(key) = status.key else { return };
        // Like on Android: past the first 3 seconds (or at the start of the queue), previous restarts the track.
        match queue.previous(key) {
            Some(previous) if status.position_secs <= 3.0 => start_key(engine, queue, Some(previous)),
            _ => engine.seek(0.0),
        }
    })?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn skip_to(audio: State<'_, DesktopAudio>, index: usize) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| start_key(engine, queue, queue.key_at(index)))?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn set_shuffle(audio: State<'_, DesktopAudio>, enabled: bool) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        queue.set_shuffle(enabled, engine.status().key);
        resync(engine, queue);
    })?;
    Ok(audio.state())
}

#[tauri::command]
pub fn set_repeat_mode(audio: State<'_, DesktopAudio>, mode: String) -> CommandResult<Value> {
    let repeat = Repeat::parse(&mode).ok_or_else(|| format!("unknown repeat mode \"{mode}\" (off, all or one)"))?;
    audio.with_engine(|engine, queue| {
        queue.repeat = repeat;
        resync(engine, queue);
    })?;
    Ok(audio.state())
}

#[tauri::command]
pub fn add_to_queue(audio: State<'_, DesktopAudio>, items: Vec<Item>, play_next: Option<bool>) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        let was_empty = queue.len() == 0;
        let first = queue.add(items, play_next.unwrap_or(false), engine.status().key);
        if was_empty {
            // It's the queue now: ready to play, like after setQueue.
            match first.and_then(|key| track_of(queue, key)) {
                Some(track) => engine.start(track, 0.0),
                None => engine.stop(),
            }
        } else {
            resync(engine, queue);
        }
    })?;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn remove_from_queue(audio: State<'_, DesktopAudio>, index: usize) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        let key = queue.key_at(index).ok_or_else(|| format!("no queue item at index {index}"))?;
        let playing = engine.status().key;
        if playing == Some(key) {
            // Removing the playing track moves on to what would have come next.
            let next = queue.peek_next(key, false).filter(|&next| next != key);
            queue.remove(index);
            match next.and_then(|next| track_of(queue, next)) {
                Some(track) => engine.start(track, 0.0),
                None => engine.stop(),
            }
        } else {
            queue.remove(index);
            resync(engine, queue);
        }
        Ok::<_, String>(())
    })??;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn move_in_queue(audio: State<'_, DesktopAudio>, from: usize, to: usize) -> CommandResult<Value> {
    audio.with_engine(|engine, queue| {
        if !queue.move_entry(from, to) {
            return Err(format!("can't move queue item {from} to {to}"));
        }
        // Without shuffle the play order changed with the list.
        resync(engine, queue);
        Ok(())
    })??;
    Ok(settled(&audio))
}

#[tauri::command]
pub fn get_state(audio: State<'_, DesktopAudio>) -> Value {
    audio.state()
}

#[tauri::command]
pub fn get_queue(audio: State<'_, DesktopAudio>) -> Value {
    let state = audio.state();
    let queue = audio.queue.lock().unwrap();
    let items: Vec<&Item> = queue.entries().iter().map(|entry| &entry.item).collect();
    json!({
        "items": items,
        "currentIndex": state["queueIndex"],
        "playOrder": queue.order(),
    })
}

#[tauri::command]
pub fn restore_last_queue() -> Option<Value> {
    None
}

#[tauri::command]
pub fn dispose(audio: State<'_, DesktopAudio>) {
    let engine = {
        let mut inner = audio.inner.lock().unwrap();
        inner.last_key = None;
        inner.engine.take()
    };
    // Stopping the engine waits for its feeder thread, which may need the queue: no locks held here.
    drop(engine);
    audio.queue.lock().unwrap().set(Vec::new(), 0);
}

/// Of the options, desktop uses repeatAddedTracks so far; the rest are Android's.
#[tauri::command]
pub fn set_options(audio: State<'_, DesktopAudio>, repeat_added_tracks: Option<bool>) {
    if let Some(on) = repeat_added_tracks {
        audio.queue.lock().unwrap().repeat_added = on;
    }
}

// ---- coming in later steps

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
